//! Lowering expression trees to elements and plain values: resolving
//! references, mapping positional args to named props, applying defaults and
//! reporting invalid props, as the reference parser's `materializeValue` and
//! `materializeExpr`.

use std::collections::HashSet;
use std::sync::Arc;

use crate::ast::{Ast, RefType};
use crate::builtins::{is_lazy_builtin, is_reserved_call};
use crate::js::{JsObject, OBJECT_PROTOTYPE_KEYS, parse_json};
use crate::library::{Call, CallKind, Library, Param};
use crate::result::{Element, ValidationError, Value, write_value};
use crate::table::{Cache, Entry, Output, Reach, Slot, Table};
use crate::validation::{
    Issue, element_parts, is_ast_node, params_signature, resolve_invalid_value,
    validate_schema_value,
};
use crate::{MAX_DEPTH, MAX_RESULT_SIZE};

pub(crate) struct Ctx<'a> {
    pub table: &'a Table,
    pub cache: &'a mut Cache,
    pub library: &'a Library,
    pub partial: bool,
    /// Statements being materialized right now, to break reference cycles.
    pub visited: Vec<Slot>,
    /// The statement errors are attributed to.
    pub current_statement_id: Option<String>,
    /// Open materialize calls, across references too: a chain of references
    /// nests as deeply as one big expression.
    pub depth: usize,
    /// About how many bytes of JSON the result has grown to so far, counting
    /// every place a statement is inlined. See [`MAX_RESULT_SIZE`].
    pub size: usize,
    /// What the materialization in progress has done on the side.
    pub frame: Frame,
    /// Statements this pass has materialized (or taken from the cache), as
    /// values and as expressions. The reference memoizes them per pass, so
    /// one taken again reports its errors only the first time, unless it met
    /// an unresolved reference: then the reference didn't memoize it.
    pub seen: HashSet<(Slot, bool)>,
}

/// The side effects of materializing one statement (or the entry): errors
/// and unresolved references in the order they were met, the statements
/// reached, and how deep it went. A fresh frame is opened for each statement
/// so its effects can be cached and replayed with it.
#[derive(Default)]
pub(crate) struct Frame {
    pub errors: Vec<ValidationError>,
    pub unresolved: Vec<String>,
    /// Every defined statement looked up here, sorted and deduplicated when
    /// the frame closes.
    pub reached: Vec<Slot>,
    /// The reached trees of the cached statements taken as they were, and of
    /// the statements materialized afresh inside this one.
    pub children: Vec<Arc<Reach>>,
    /// The deepest `depth` a materialize call started at.
    pub max_depth: usize,
    /// A reference hit a statement already being materialized.
    pub cycle: bool,
    /// Went past `MAX_DEPTH`; the deeper parts became null.
    pub limited: bool,
    /// Grew past `MAX_RESULT_SIZE`; the rest became null.
    pub size_limited: bool,
}

/// What each piece adds to [`Ctx::size`]: about the bytes of JSON it writes,
/// erring high, so the result's JSON stays under the limit.
const ELEMENT_SIZE: usize = 100;
const ERROR_SIZE: usize = 100;

/// The JSON a node writes besides its children (counted when they are
/// materialized) and inlined statements (counted when inlined). In value
/// mode, runtime expressions are counted in expression mode instead, and a
/// component becoming an element adds the rest of `ELEMENT_SIZE` later.
fn json_size(node: &Ast, expr: bool) -> usize {
    let size = match node {
        Ast::Ref(_) => 0,
        Ast::Str(v) => 20 + v.len(),
        Ast::Num(_) => 41,
        Ast::Bool(_) => 20,
        Ast::Null => 10,
        Ast::Ph(n) | Ast::StateRef(n) => 24 + n.len(),
        Ast::RuntimeRef { n, .. } => 44 + n.len(),
        Ast::Arr(_) => 22,
        Ast::Obj(_) => 26,
        Ast::Comp { name, .. } => 34 + name.len(),
        Ast::BinOp { .. } | Ast::Ternary { .. } => 42,
        Ast::UnaryOp { .. } | Ast::Index { .. } => 34,
        Ast::Member { field, .. } => 36 + field.len(),
        Ast::Assign { target, .. } => 38 + target.len(),
    };
    match node {
        _ if expr => size,
        // Plain values write less than their AST would.
        Ast::Str(v) => 3 + v.len(),
        Ast::Num(_) => 25,
        Ast::Bool(_) | Ast::Null | Ast::Ph(_) => 6,
        Ast::Arr(_) | Ast::Obj(_) => 3,
        Ast::Ref(_) | Ast::Comp { .. } => size,
        _ => 0,
    }
}

impl Ctx<'_> {
    pub(crate) fn push_issue(&mut self, component: &str, path: &str, issue: Issue) {
        let message = issue.message(component, path);
        self.size += ERROR_SIZE + message.len() + component.len() + path.len();
        self.frame.errors.push(ValidationError {
            code: issue.code(),
            component: component.to_owned(),
            path: path.to_owned(),
            message,
            statement_id: self.current_statement_id.clone(),
            severity: issue.severity(),
        });
    }

    fn unresolved(&mut self, name: &str) {
        self.size += name.len() + 3;
        self.frame.unresolved.push(name.to_owned());
    }

    /// Whether the result can still grow by `more`; if not, it's cut here.
    fn fits(&mut self, more: usize) -> bool {
        if self.size.saturating_add(more) > MAX_RESULT_SIZE {
            self.frame.size_limited = true;
            return false;
        }
        true
    }

    fn unknown_component(&mut self, name: &str) {
        let library = self.library;
        let available = library.component_names().map(str::to_owned).collect();
        self.push_issue(name, "", Issue::UnknownComponent { available });
    }

    /// Note a materialize call starting at the current depth, writing about
    /// `size` bytes; `None` past either limit.
    fn enter(&mut self, size: usize) -> Option<()> {
        self.frame.max_depth = self.frame.max_depth.max(self.depth);
        if self.depth >= MAX_DEPTH {
            self.frame.limited = true;
            return None;
        }
        if !self.fits(size) {
            return None;
        }
        self.size += size;
        self.depth += 1;
        Some(())
    }
}

/// Iterator variables in scope inside `@Each` templates.
type Scope = HashSet<String>;

/// Lower a node to what ends up in the tree: elements, plain values, and AST
/// for builtins and runtime expressions; `Null` for placeholders and for
/// components that had to be dropped. Past `MAX_DEPTH`, `Null`.
pub(crate) fn materialize_value(node: &Ast, ctx: &mut Ctx) -> Value {
    if ctx.enter(json_size(node, false)).is_none() {
        return Value::Null;
    }
    let value = materialize_value_at_depth(node, ctx);
    ctx.depth -= 1;
    value
}

// The functions on the recursive path stay small, and the heavier
// non-recursive work lives in `#[inline(never)]` helpers. In unoptimized
// builds a frame holds every temporary its function could need, so this is
// what keeps MAX_DEPTH levels inside a small thread stack.
fn materialize_value_at_depth(node: &Ast, ctx: &mut Ctx) -> Value {
    match node {
        Ast::Ref(name) => match resolve_ref(name, ctx, Mode::Value) {
            Resolved::Value(v) => v,
            Resolved::Ast(a) => Value::Ast(Arc::new(a)),
        },
        Ast::Str(v) => Value::Str(v.clone()),
        Ast::Num(n) => Value::Num(*n),
        Ast::Bool(b) => Value::Bool(*b),
        Ast::Null | Ast::Ph(_) => Value::Null,
        Ast::Arr(els) => materialize_array(els, ctx),
        Ast::Obj(entries) => materialize_object(entries, ctx),
        Ast::Comp {
            name,
            args,
            function,
            mapped_props,
        } => materialize_comp(name, args, *function, mapped_props, ctx),
        // Runtime expressions stay AST, with their insides normalized.
        _ => Value::Ast(Arc::new(materialize_expr(node, ctx, &Scope::new()))),
    }
}

fn materialize_array(els: &[Ast], ctx: &mut Ctx) -> Value {
    let mut items = Vec::new();
    for el in els {
        if matches!(el, Ast::Ph(_)) {
            continue; // unresolved placeholders drop out of arrays
        }
        let value = materialize_value(el, ctx);
        // So do components and refs that came out null (dropped, unresolved, unknown).
        if matches!(value, Value::Null) && matches!(el, Ast::Comp { .. } | Ast::Ref(_)) {
            continue;
        }
        items.push(value);
    }
    Value::Arr(Arc::new(items))
}

fn materialize_object(entries: &[(String, Ast)], ctx: &mut Ctx) -> Value {
    let mut obj = JsObject::new();
    for (key, value) in entries {
        ctx.size += key.len() + 4;
        let value = materialize_value(value, ctx);
        obj.set(key.clone(), value);
    }
    Value::Obj(Arc::new(obj))
}

#[derive(Clone, Copy)]
enum Mode {
    Value,
    Expr,
}

enum Resolved {
    Value(Value),
    Ast(Ast),
}

/// Inline the referenced statement. Unknown or cyclic refs are recorded as
/// unresolved; refs to Query/Mutation statements become `RuntimeRef`s.
///
/// A statement materialized before is taken from the cache when it's still
/// there (a change to anything it references drops it) and it fits under the
/// depth limit from here. A cached statement can't reach a statement being
/// materialized right now: that would be a cycle, and a materialization that
/// met a cycle is never cached.
fn resolve_ref(name: &str, ctx: &mut Ctx, mode: Mode) -> Resolved {
    let placeholder = |mode: Mode| match mode {
        Mode::Expr => Resolved::Ast(Ast::Ph(name.to_owned())),
        Mode::Value => Resolved::Value(Value::Null),
    };
    let table = ctx.table;
    let Some(slot) = table.slot(name) else {
        ctx.unresolved(name);
        return placeholder(mode);
    };
    let target = match table.get(slot) {
        Some(stmt) if !ctx.visited.contains(&slot) => stmt.expr(),
        found => {
            ctx.frame.cycle |= found.is_some();
            ctx.unresolved(name);
            return placeholder(mode);
        }
    };
    ctx.frame.reached.push(slot);
    if let Ast::Comp { name: callee, .. } = target
        && is_reserved_call(callee)
    {
        let ref_type = if callee == "Mutation" {
            RefType::Mutation
        } else {
            RefType::Query
        };
        let node = Ast::RuntimeRef {
            n: name.to_owned(),
            ref_type,
        };
        return match mode {
            Mode::Expr => Resolved::Ast(node),
            Mode::Value => Resolved::Value(Value::Ast(Arc::new(node))),
        };
    }

    let expr = matches!(mode, Mode::Expr);
    if let Some(entry) = ctx.cache.get(slot, ctx.partial, expr)
        && ctx.depth + entry.r_max < MAX_DEPTH
    {
        // Inlined again, it grows the result as much as it did the first time.
        let size = entry.size;
        if !ctx.fits(size) {
            return match mode {
                Mode::Expr => Resolved::Ast(Ast::Null),
                Mode::Value => Resolved::Value(Value::Null),
            };
        }
        ctx.size += size;
        let first = ctx.seen.insert((slot, expr));
        let entry = ctx
            .cache
            .get(slot, ctx.partial, expr)
            .expect("still cached");
        replay(
            entry,
            &mut ctx.frame,
            ctx.depth,
            first || !entry.unresolved.is_empty(),
        );
        return match &entry.output {
            Output::Value(v) => Resolved::Value(v.clone()),
            Output::Ast(a) => Resolved::Ast(a.clone()),
        };
    }

    ctx.visited.push(slot);
    let previous = ctx.current_statement_id.replace(name.to_owned());
    let outer = std::mem::take(&mut ctx.frame);
    let start_depth = ctx.depth;
    let start_size = ctx.size;
    let output = match mode {
        Mode::Value => {
            let mut value = materialize_value(target, ctx);
            if tag_statement(&mut value, name) {
                ctx.size += name.len() + 18; // `,"statementId":"name"`
            }
            Output::Value(value)
        }
        Mode::Expr => Output::Ast(materialize_expr(target, ctx, &Scope::new())),
    };
    let frame = std::mem::replace(&mut ctx.frame, outer);
    ctx.current_statement_id = previous;
    ctx.visited.pop();
    let resolved = match &output {
        Output::Value(v) => Resolved::Value(v.clone()),
        Output::Ast(a) => Resolved::Ast(a.clone()),
    };
    let size = ctx.size - start_size;
    finish(frame, output, slot, expr, (start_depth, size), ctx);
    ctx.seen.insert((slot, expr));
    resolved
}

/// Account for a cached statement as if it had been materialized here, its
/// errors included unless this pass has reported them already.
#[inline(never)]
fn replay(entry: &Entry, frame: &mut Frame, depth: usize, errors: bool) {
    if errors {
        frame.errors.extend(entry.errors.iter().cloned());
    }
    frame.unresolved.extend(entry.unresolved.iter().cloned());
    frame.children.push(entry.reach.clone());
    frame.max_depth = frame.max_depth.max(depth + entry.r_max);
}

/// Fold a finished statement's frame into the outer one, and cache it unless
/// it met a cycle or the depth limit (then it depends on where it was reached from).
#[inline(never)]
fn finish(
    frame: Frame,
    output: Output,
    slot: Slot,
    expr: bool,
    (start_depth, size): (usize, usize),
    ctx: &mut Ctx,
) {
    let Frame {
        errors,
        unresolved,
        mut reached,
        children,
        max_depth,
        cycle,
        limited,
        size_limited,
    } = frame;
    reached.sort_unstable();
    reached.dedup();
    let reach = Arc::new(Reach {
        direct: reached,
        children,
    });
    let reach = ctx.cache.share_reach(slot, reach);
    ctx.frame.errors.extend(errors.iter().cloned());
    ctx.frame.unresolved.extend(unresolved.iter().cloned());
    ctx.frame.children.push(reach.clone());
    ctx.frame.max_depth = ctx.frame.max_depth.max(max_depth);
    ctx.frame.cycle |= cycle;
    ctx.frame.limited |= limited;
    ctx.frame.size_limited |= size_limited;
    if cycle || limited || size_limited {
        return;
    }
    let entry = Entry {
        output,
        reach,
        unresolved,
        errors,
        r_max: max_depth.saturating_sub(start_depth),
        size,
    };
    ctx.cache.put(slot, ctx.partial, expr, entry);
}

/// `result.statementId = name` on an element (or anything shaped like one).
/// Whether it was one.
pub(crate) fn tag_statement(value: &mut Value, name: &str) -> bool {
    if element_parts(value).is_none() {
        return false;
    }
    match value {
        Value::Element(e) => Arc::make_mut(e).statement_id = Some(name.to_owned()),
        Value::Obj(o) => Arc::make_mut(o).set("statementId", Value::Str(name.to_owned())),
        _ => {}
    }
    true
}

fn materialize_comp(
    name: &str,
    args: &[Ast],
    function: bool,
    mapped_props: &Option<Arc<JsObject<Ast>>>,
    ctx: &mut Ctx,
) -> Value {
    if function && !is_runtime_call(name, ctx) {
        return report(ctx, name, Issue::UnknownFunction);
    }
    if is_reserved_call(name) {
        return report(ctx, name, Issue::InlineReserved);
    }
    if is_runtime_call(name, ctx) {
        let call = runtime_call(name, args, function, mapped_props, ctx, &Scope::new());
        return Value::Ast(Arc::new(call));
    }
    let library = ctx.library;
    let Some(def) = library.component(name) else {
        return unknown_component(ctx, name);
    };
    let mut props = JsObject::new();
    // Set when a required prop is invalid with no default to fall back on.
    let mut drop_component = false;
    for (param, arg) in def.params.iter().zip(args) {
        let value = materialize_value(arg, ctx);
        drop_component |= add_prop(&mut props, param, value, name, ctx);
    }
    finish_component(name, &def.params, args.len(), props, drop_component, ctx)
}

/// A builtin, one of the library's functions or actions, or a reserved
/// call: these stay AST for the runtime.
fn is_runtime_call(name: &str, ctx: &Ctx) -> bool {
    ctx.library.call(name).is_some() || is_reserved_call(name)
}

/// A runtime call, kept as AST: `@Each` with its iterator in scope, anything
/// else with its args normalized and, for a function or step, by param name.
fn runtime_call(
    name: &str,
    args: &[Ast],
    function: bool,
    mapped_props: &Option<Arc<JsObject<Ast>>>,
    ctx: &mut Ctx,
    scope: &Scope,
) -> Ast {
    if let Some(lazy) = materialize_lazy_builtin(name, args, function, mapped_props, ctx, scope) {
        return lazy;
    }
    let start = ctx.size;
    let args = expr_list(args, ctx, scope);
    let args_size = ctx.size - start;
    mapped_call(name, args, args_size, function, mapped_props, ctx)
}

/// `materializeCall`: a builtin's args by param name; a library function's
/// or action's checked like props first. An invalid function call is null,
/// and an invalid action an empty plan, `Action([])`. Lazy calls and reserved
/// ones keep their args as they are.
#[inline(never)]
fn mapped_call(
    name: &str,
    args: Arc<Vec<Ast>>,
    args_size: usize,
    function: bool,
    mapped_props: &Option<Arc<JsObject<Ast>>>,
    ctx: &mut Ctx,
) -> Ast {
    let library = ctx.library;
    let Some(def) = library.call(name).filter(|def| !def.lazy) else {
        return Ast::Comp {
            name: name.to_owned(),
            args,
            function,
            mapped_props: mapped_props.clone(),
        };
    };
    let mapped = if def.builtin {
        Some(name_args(&args, &def.params))
    } else {
        map_call_args(name, &args, def, ctx)
    };
    let Some(by_name) = mapped else {
        return match def.kind {
            CallKind::Action => Ast::Comp {
                name: "Action".to_owned(),
                args: Arc::new(vec![Ast::Arr(Arc::new(Vec::new()))]),
                function: false,
                mapped_props: None,
            },
            CallKind::Function => Ast::Null,
        };
    };
    // `mappedProps` holds the args again: as much again in the result.
    let keys: usize = by_name.iter().map(|(k, _)| k.len() + 4).sum();
    let size = args_size + 16 + keys;
    if !ctx.fits(size) {
        return Ast::Null;
    }
    ctx.size += size;
    Ast::Comp {
        name: name.to_owned(),
        args,
        function,
        mapped_props: Some(Arc::new(by_name)),
    }
}

/// `nameArgs`: args by param name, in param order; extra args are left out.
fn name_args(args: &[Ast], params: &[Param]) -> JsObject<Ast> {
    let mut by_name = JsObject::new();
    for (param, arg) in params.iter().zip(args) {
        by_name.set(param.name.clone(), arg.clone());
    }
    by_name
}

/// `mapCallArgs`: a library function's or action's args by param name, each
/// literal one checked against its param's schema as a prop is. Excess args
/// are reported and left out; a missing or null required arg without a
/// default, or an invalid one, makes the call invalid (`None`); an invalid
/// optional one is left out, or replaced by its default. An action with any
/// error at all is invalid: it never runs with a value the model didn't write.
#[inline(never)]
fn map_call_args(name: &str, args: &[Ast], def: &Call, ctx: &mut Ctx) -> Option<JsObject<Ast>> {
    let params = &def.params;
    if args.len() > params.len() {
        let issue = Issue::ExcessArgs {
            declared: params.len(),
            got: args.len(),
        };
        ctx.push_issue(name, "", issue);
    }
    let mut mapped = name_args(args, params);
    let errors_before = ctx.frame.errors.len();
    let mut invalid = false;
    for param in params {
        let arg = mapped.get(&param.name);
        // `mapped[name]` reads an inherited method for `toString` and the
        // like, which isn't a literal: checked at runtime.
        if arg.is_none() && OBJECT_PROTOTYPE_KEYS.contains(&param.name.as_str()) {
            continue;
        }
        let literal = match arg {
            Some(arg) => match literal_value(arg) {
                Some(value) => Some(value),
                None => continue, // not a literal: checked at runtime
            },
            None => None,
        };
        let path = format!("/{}", param.name);
        let value = match literal {
            Some(value) if !matches!(value, Value::Null) => value,
            _ => {
                if !param.required || param.default.is_some() {
                    continue;
                }
                let signature = Some(params_signature(name, params));
                let issue = if arg.is_some() {
                    Issue::NullRequired { signature }
                } else {
                    Issue::MissingRequired { signature }
                };
                ctx.push_issue(name, &path, issue);
                invalid = true;
                continue;
            }
        };
        let mut next = value.clone();
        if validate_schema_value(&mut next, &param.schema, name, &path, ctx) {
            match (&param.default, param.required) {
                (Some(default), _) => next = Value::from_json(default),
                (None, true) => {
                    invalid = true;
                    continue;
                }
                (None, false) => {
                    mapped.remove(&param.name);
                    continue;
                }
            }
        }
        if !unchanged(&next, &value) {
            mapped.set(param.name.clone(), to_literal_ast(&next));
        }
    }
    if def.kind == CallKind::Action && ctx.frame.errors.len() > errors_before {
        invalid = true;
    }
    (!invalid).then_some(mapped)
}

/// `literalValue`: the value a literal node stands for; `None` for anything
/// that needs evaluating (a reference, a call, an operator).
/// Whether validation left a value as it was: the reference compares by
/// identity (`next !== lit.v`), and gets back the very object when nothing
/// changed. Structurally, then, but with `NaN` equal to itself, so an object
/// holding one isn't rebuilt (which would also drop a duplicate key).
fn unchanged(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => x == y || x.is_nan() && y.is_nan(),
        (Value::Arr(x), Value::Arr(y)) => {
            Arc::ptr_eq(x, y)
                || x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| unchanged(a, b))
        }
        (Value::Obj(x), Value::Obj(y)) => {
            Arc::ptr_eq(x, y)
                || x.len() == y.len()
                    && x.iter()
                        .zip(y.iter())
                        .all(|((k, a), (l, b))| k == l && unchanged(a, b))
        }
        _ => a == b,
    }
}

fn literal_value(node: &Ast) -> Option<Value> {
    Some(match node {
        Ast::Str(v) => Value::Str(v.clone()),
        Ast::Num(n) => Value::Num(*n),
        Ast::Bool(b) => Value::Bool(*b),
        Ast::Null => Value::Null,
        Ast::Arr(els) => Value::Arr(Arc::new(
            els.iter().map(literal_value).collect::<Option<Vec<_>>>()?,
        )),
        Ast::Obj(entries) => {
            let mut obj = JsObject::new();
            for (key, value) in entries.iter() {
                obj.set(key.clone(), literal_value(value)?);
            }
            Value::Obj(Arc::new(obj))
        }
        _ => return None,
    })
}

/// `toLiteralAST`: a plain value as the literal node that writes it.
fn to_literal_ast(value: &Value) -> Ast {
    match value {
        Value::Str(v) => Ast::Str(v.clone()),
        Value::Num(n) => Ast::Num(*n),
        Value::Bool(b) => Ast::Bool(*b),
        Value::Arr(items) => Ast::Arr(Arc::new(items.iter().map(to_literal_ast).collect())),
        Value::Obj(obj) => Ast::Obj(Arc::new(
            obj.iter()
                .map(|(k, v)| (k.clone(), to_literal_ast(v)))
                .collect(),
        )),
        // Null; elements and AST don't come out of literals.
        _ => Ast::Null,
    }
}

/// `nameRawCallArgs`: in a Query's or Mutation's args, every call to a
/// builtin, library function or step gets its args by param name, as
/// written. A library call's literal args are checked as props are; an
/// invalid call keeps its args by name all the same, and is null at runtime.
/// A call is checked before the calls inside it, as `walkAST` visits them.
pub(crate) fn name_raw_call_args(args: &[Ast], ctx: &mut Ctx, statement_id: &str) -> Vec<Ast> {
    let outer = ctx.current_statement_id.replace(statement_id.to_owned());
    let named = args.iter().map(|arg| name_calls(arg, ctx)).collect();
    ctx.current_statement_id = outer;
    named
}

fn name_calls(node: &Ast, ctx: &mut Ctx) -> Ast {
    let all = |nodes: &[Ast], ctx: &mut Ctx| -> Arc<Vec<Ast>> {
        Arc::new(nodes.iter().map(|n| name_calls(n, ctx)).collect())
    };
    let one = |node: &Ast, ctx: &mut Ctx| Arc::new(name_calls(node, ctx));
    match node {
        Ast::Comp {
            name,
            args,
            function,
            mapped_props,
        } => {
            let library = ctx.library;
            let def = library.call(name).filter(|def| !def.lazy);
            let checked = match def {
                Some(def) if !def.builtin => map_call_args(name, args, def, ctx),
                _ => None,
            };
            let named_args = all(args, ctx);
            let mapped_props = match def {
                None => mapped_props.clone(),
                Some(def) => {
                    let mut by_name = checked.unwrap_or_else(|| name_args(args, &def.params));
                    // Args as written are shared with `args`, so they carry
                    // their own calls' names too.
                    for (i, param) in def.params.iter().enumerate().take(args.len()) {
                        if by_name.get(&param.name) == Some(&args[i]) {
                            by_name.set(param.name.clone(), named_args[i].clone());
                        }
                    }
                    Some(Arc::new(by_name))
                }
            };
            Ast::Comp {
                name: name.clone(),
                args: named_args,
                function: *function,
                mapped_props,
            }
        }
        Ast::Arr(els) => Ast::Arr(all(els, ctx)),
        Ast::Obj(entries) => Ast::Obj(Arc::new(
            entries
                .iter()
                .map(|(k, v)| (k.clone(), name_calls(v, ctx)))
                .collect(),
        )),
        Ast::BinOp { op, left, right } => {
            let left = one(left, ctx);
            Ast::BinOp {
                op,
                left,
                right: one(right, ctx),
            }
        }
        Ast::UnaryOp { op, operand } => Ast::UnaryOp {
            op,
            operand: one(operand, ctx),
        },
        Ast::Ternary {
            cond,
            then,
            otherwise,
        } => {
            let cond = one(cond, ctx);
            let then = one(then, ctx);
            Ast::Ternary {
                cond,
                then,
                otherwise: one(otherwise, ctx),
            }
        }
        Ast::Member { obj, field } => Ast::Member {
            obj: one(obj, ctx),
            field: field.clone(),
        },
        Ast::Index { obj, index } => {
            let obj = one(obj, ctx);
            Ast::Index {
                obj,
                index: one(index, ctx),
            }
        }
        Ast::Assign { target, value } => Ast::Assign {
            target: target.clone(),
            value: one(value, ctx),
        },
        _ => node.clone(),
    }
}

#[inline(never)]
fn report(ctx: &mut Ctx, name: &str, issue: Issue) -> Value {
    ctx.push_issue(name, "", issue);
    Value::Null
}

#[inline(never)]
fn unknown_component(ctx: &mut Ctx, name: &str) -> Value {
    ctx.unknown_component(name);
    Value::Null
}

/// Validate one prop and store it. True if it's required, invalid, and has no
/// default, which drops the component.
#[inline(never)]
fn add_prop(
    props: &mut JsObject<Value>,
    param: &Param,
    mut value: Value,
    component: &str,
    ctx: &mut Ctx,
) -> bool {
    ctx.size += param.name.len() + 4;
    // The reference assigns the raw value before validating, then the
    // validated one: a null `__proto__` drops the prototype, so the second
    // assignment makes `__proto__` an own prop.
    if param.name == "__proto__" && matches!(value, Value::Null) {
        props.drop_prototype();
    }
    let path = format!("/{}", param.name);
    let invalid = validate_schema_value(&mut value, &param.schema, component, &path, ctx);
    props.set(param.name.clone(), value);
    invalid && resolve_invalid_value(props, &param.name, param.required, param.default.as_ref())
}

/// Excess args, missing required props (defaults first), and the element.
#[inline(never)]
fn finish_component(
    name: &str,
    params: &[Param],
    arg_count: usize,
    mut props: JsObject<Value>,
    drop_component: bool,
    ctx: &mut Ctx,
) -> Value {
    if arg_count > params.len() {
        let issue = Issue::ExcessArgs {
            declared: params.len(),
            got: arg_count,
        };
        ctx.push_issue(name, "", issue);
    }

    // Missing or null required props take their default; without one, the
    // component is dropped.
    let missing: Vec<&Param> = params
        .iter()
        .filter(|p| {
            p.required && (!props.has(&p.name) || matches!(props.get(&p.name), Some(Value::Null)))
        })
        .collect();
    let mut still_missing = Vec::new();
    for p in missing {
        match &p.default {
            Some(default) => props.set(p.name.clone(), Value::from_json(default)),
            None => still_missing.push(p),
        }
    }
    if !still_missing.is_empty() {
        let signature = params_signature(name, params);
        for p in still_missing {
            let signature = Some(signature.clone());
            let issue = if props.has(&p.name) {
                Issue::NullRequired { signature }
            } else {
                Issue::MissingRequired { signature }
            };
            ctx.push_issue(name, &format!("/{}", p.name), issue);
        }
        return Value::Null;
    }
    if drop_component {
        return Value::Null;
    }

    ctx.size += ELEMENT_SIZE - 34;
    let has_dynamic_props = props.values().any(contains_dynamic_value);
    Value::Element(Arc::new(Element {
        type_name: name.to_owned(),
        props,
        partial: ctx.partial,
        has_dynamic_props,
        statement_id: None,
    }))
}

/// `@Each(items, var, template)`: the iterator variable (a ref or a string)
/// stays in scope while the other args are normalized; `var` itself is kept as
/// written. `None` if this isn't a lazy builtin with a usable variable.
fn materialize_lazy_builtin(
    name: &str,
    args: &[Ast],
    function: bool,
    mapped_props: &Option<Arc<JsObject<Ast>>>,
    ctx: &mut Ctx,
    scope: &Scope,
) -> Option<Ast> {
    if !is_lazy_builtin(name) || args.len() < 3 {
        return None;
    }
    let var = match &args[1] {
        Ast::Ref(n) => n,
        Ast::Str(v) => v,
        _ => return None,
    };
    if var.is_empty() {
        return None;
    }
    let mut inner = scope.clone();
    inner.insert(var.clone());
    let args = args
        .iter()
        .enumerate()
        .map(|(i, a)| {
            if i == 1 {
                a.clone()
            } else {
                materialize_expr(a, ctx, &inner)
            }
        })
        .collect::<Vec<_>>();
    Some(Ast::Comp {
        name: name.to_owned(),
        args: Arc::new(args),
        function,
        mapped_props: mapped_props.clone(),
    })
}

/// Normalize a node for runtime evaluation: resolve refs, map library
/// components' args to `mapped_props`. The structure stays AST. Past
/// `MAX_DEPTH`, `Null`.
pub(crate) fn materialize_expr(node: &Ast, ctx: &mut Ctx, scope: &Scope) -> Ast {
    if ctx.enter(json_size(node, true)).is_none() {
        return Ast::Null;
    }
    let ast = materialize_expr_at_depth(node, ctx, scope);
    ctx.depth -= 1;
    ast
}

fn materialize_expr_at_depth(node: &Ast, ctx: &mut Ctx, scope: &Scope) -> Ast {
    match node {
        Ast::Ref(name) if scope.contains(name) => node.clone(),
        Ast::Ref(name) => match resolve_ref(name, ctx, Mode::Expr) {
            Resolved::Ast(a) => a,
            Resolved::Value(_) => unreachable!("expr mode resolves to AST"),
        },
        Ast::Comp {
            name,
            args,
            function,
            mapped_props,
        } => expr_comp(name, args, *function, mapped_props, ctx, scope),
        Ast::Arr(els) => Ast::Arr(expr_list(els, ctx, scope)),
        Ast::Obj(entries) => expr_object(entries, ctx, scope),
        Ast::BinOp { op, left, right } => {
            let left = expr_boxed(left, ctx, scope);
            Ast::BinOp {
                op,
                left,
                right: expr_boxed(right, ctx, scope),
            }
        }
        Ast::UnaryOp { op, operand } => Ast::UnaryOp {
            op,
            operand: expr_boxed(operand, ctx, scope),
        },
        Ast::Ternary {
            cond,
            then,
            otherwise,
        } => expr_ternary(cond, then, otherwise, ctx, scope),
        Ast::Member { obj, field } => Ast::Member {
            obj: expr_boxed(obj, ctx, scope),
            field: field.clone(),
        },
        Ast::Index { obj, index } => {
            let obj = expr_boxed(obj, ctx, scope);
            Ast::Index {
                obj,
                index: expr_boxed(index, ctx, scope),
            }
        }
        Ast::Assign { target, value } => Ast::Assign {
            target: target.clone(),
            value: expr_boxed(value, ctx, scope),
        },
        // Literals, placeholders, state and runtime refs pass through.
        _ => node.clone(),
    }
}

fn expr_boxed(node: &Ast, ctx: &mut Ctx, scope: &Scope) -> Arc<Ast> {
    Arc::new(materialize_expr(node, ctx, scope))
}

fn expr_list(nodes: &[Ast], ctx: &mut Ctx, scope: &Scope) -> Arc<Vec<Ast>> {
    Arc::new(
        nodes
            .iter()
            .map(|n| materialize_expr(n, ctx, scope))
            .collect(),
    )
}

fn expr_object(entries: &[(String, Ast)], ctx: &mut Ctx, scope: &Scope) -> Ast {
    Ast::Obj(Arc::new(
        entries
            .iter()
            .map(|(k, v)| {
                ctx.size += k.len() + 8;
                (k.clone(), materialize_expr(v, ctx, scope))
            })
            .collect(),
    ))
}

fn expr_ternary(cond: &Ast, then: &Ast, otherwise: &Ast, ctx: &mut Ctx, scope: &Scope) -> Ast {
    let cond = expr_boxed(cond, ctx, scope);
    let then = expr_boxed(then, ctx, scope);
    Ast::Ternary {
        cond,
        then,
        otherwise: expr_boxed(otherwise, ctx, scope),
    }
}

fn expr_comp(
    name: &str,
    args: &[Ast],
    function: bool,
    mapped_props: &Option<Arc<JsObject<Ast>>>,
    ctx: &mut Ctx,
    scope: &Scope,
) -> Ast {
    if function && !is_runtime_call(name, ctx) {
        ctx.push_issue(name, "", Issue::UnknownFunction);
        return Ast::Null;
    }
    if is_runtime_call(name, ctx) {
        return runtime_call(name, args, function, mapped_props, ctx, scope);
    }
    let start = ctx.size;
    let args = expr_list(args, ctx, scope);
    let args_size = ctx.size - start;
    library_comp(name, args, args_size, mapped_props, ctx)
}

/// A component call in a runtime expression: a library component gets
/// `mapped_props` (args by param name); an unknown one is reported.
#[inline(never)]
fn library_comp(
    name: &str,
    args: Arc<Vec<Ast>>,
    args_size: usize,
    mapped_props: &Option<Arc<JsObject<Ast>>>,
    ctx: &mut Ctx,
) -> Ast {
    let library = ctx.library;
    if let Some(def) = library.component(name) {
        // `mappedProps` holds the args again: as much again in the result.
        let keys: usize = def
            .params
            .iter()
            .take(args.len())
            .map(|p| p.name.len() + 4)
            .sum();
        let mapped = args_size + 16 + keys;
        if !ctx.fits(mapped) {
            return Ast::Null;
        }
        ctx.size += mapped;
        let by_name = name_args(&args, &def.params);
        return Ast::Comp {
            name: name.to_owned(),
            args,
            function: false,
            mapped_props: Some(Arc::new(by_name)),
        };
    }
    ctx.unknown_component(name);
    Ast::Comp {
        name: name.to_owned(),
        args,
        function: false,
        mapped_props: mapped_props.clone(),
    }
}

/// `Object.values(props).some(containsDynamicValue)` for an object shaped like
/// an element whose props aren't a plain object: an array's items, or the
/// fields of an element or an AST node as they are in JSON, which is what the
/// reference walks. Only objects written to look like elements get here.
fn values_contain_dynamic(props: &Value) -> bool {
    if let Value::Arr(items) = props {
        return items.iter().any(contains_dynamic_value);
    }
    let mut json = String::new();
    write_value(&mut json, props);
    let fields = parse_json(&json).map(|json| Value::from_json(&json));
    matches!(fields, Ok(Value::Obj(fields)) if fields.values().any(contains_dynamic_value))
}

/// Whether a prop value holds anything that needs runtime evaluation.
///
/// An element answers from its own flag: it was set from this same check over
/// its props when the element was made, and nothing changes those props
/// afterwards. That keeps a parent from walking the whole tree below it, which
/// the streaming parser shares between results.
fn contains_dynamic_value(value: &Value) -> bool {
    if is_ast_node(value) {
        return true;
    }
    match value {
        Value::Arr(items) => items.iter().any(contains_dynamic_value),
        Value::Element(e) => e.has_dynamic_props,
        _ => match element_parts(value) {
            Some((_, Some(props))) => props.values().any(contains_dynamic_value),
            Some((_, None)) => {
                matches!(value, Value::Obj(o) if o.get("props").is_some_and(values_contain_dynamic))
            }
            // A `{steps: [...]}` literal is evaluated too, so data can't pose as
            // custom action steps.
            None => match value {
                Value::Obj(o) => {
                    matches!(o.get("steps"), Some(Value::Arr(_)))
                        || o.values().any(contains_dynamic_value)
                }
                _ => false,
            },
        },
    }
}
