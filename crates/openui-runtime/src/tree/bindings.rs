//! Binding configuration and atomic, bounded field writes.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// Binding props by component and prop name, independent of JSON Schema.
#[derive(Clone, Debug, Default)]
pub struct BindingProps {
    marked: BTreeMap<String, BTreeSet<String>>,
    undecided: BTreeMap<String, BTreeSet<String>>,
}
impl BindingProps {
    /// An empty configuration.
    pub fn new() -> Self {
        Self::default()
    }
    /// Mark one prop as a binding. Names are literal, including prototype names.
    pub fn insert(&mut self, component: impl Into<String>, prop: impl Into<String>) {
        let component = component.into();
        let prop = prop.into();
        if let Some(props) = self.undecided.get_mut(&component) {
            props.remove(&prop);
        }
        self.marked.entry(component).or_default().insert(prop);
    }
    /// Whether this exact component and prop pair is marked.
    pub fn contains(&self, component: &str, prop: &str) -> bool {
        self.marked.get(component).is_some_and(|p| p.contains(prop))
    }
    /// Component/prop pairs whose annotations could not be decided. `insert`
    /// resolves a reported pair as a binding. Structural names are needed to
    /// enumerate all affected props; signature-only input reports only names
    /// it can recognize, and cannot enumerate fields hidden by malformed text.
    pub fn undecided(&self) -> impl Iterator<Item = (&str, &str)> {
        self.undecided.iter().flat_map(|(component, props)| {
            props
                .iter()
                .map(move |prop| (component.as_str(), prop.as_str()))
        })
    }
    pub(super) fn component(&self, component: &str) -> Option<&BTreeSet<String>> {
        self.marked.get(component)
    }
    /// Read direct `$binding<…>` parameter annotations from a prompt spec's
    /// `components` signatures. The wrapper must enclose the entire prop type;
    /// marks on array items, union branches or object fields do not mark it.
    /// JSON Schema alone cannot recover schema-object identity or marks that
    /// have changed since the spec was generated; supply those pairs explicitly.
    /// The schema's `$defs` property names anchor each annotation independently,
    /// so unescaped literals and punctuation in names cannot hide later props.
    /// Repeated parameter delimiters inside literal text are ambiguous: that
    /// field is reported by `undecided`; the preceding annotation can also be
    /// affected. A delimiter in a prop name, such as `a, b` alongside `b`, can
    /// create the same ambiguity. An annotation whose own
    /// unescaped quotes/backslashes or angle brackets cannot be balanced is also
    /// reported independently of later fields. Without structural properties,
    /// only well-formed signatures with ordinary names can be read reliably.
    /// Supply undecidable pairs explicitly with `insert`.
    pub fn from_prompt_spec(spec: &Value) -> Self {
        let mut result = Self::new();
        let Value::Obj(spec) = spec else {
            return result;
        };
        let Some(Value::Obj(components)) = spec.get("components") else {
            return result;
        };
        for (name, component) in components.iter() {
            let properties = spec
                .get("schema")
                .and_then(|s| {
                    if let Value::Obj(s) = s {
                        s.get("$defs")
                    } else {
                        None
                    }
                })
                .and_then(|s| {
                    if let Value::Obj(s) = s {
                        s.get(name)
                    } else {
                        None
                    }
                })
                .and_then(|s| {
                    if let Value::Obj(s) = s {
                        s.get("properties")
                    } else {
                        None
                    }
                })
                .and_then(|v| {
                    if let Value::Obj(v) = v {
                        Some(v.as_ref())
                    } else {
                        None
                    }
                });
            let signature = if let Value::Obj(component) = component {
                match component.get("signature") {
                    Some(Value::Str(s)) => Some(s.as_str()),
                    _ => None,
                }
            } else {
                None
            };
            let Some((signature, args)) =
                signature.and_then(|s| s.split_once('(').map(|(_, args)| (s, args)))
            else {
                if let Some(properties) = properties {
                    let props = properties.iter().map(|(name, _)| name.clone()).collect();
                    result.undecided.insert(name.clone(), props);
                }
                continue;
            };
            if let Some(properties) = properties {
                let args = signature
                    .strip_prefix(name)
                    .and_then(|s| s.strip_prefix('('))
                    .unwrap_or(args);
                let (props, undecided) = anchored_props(args, properties);
                if !props.is_empty() {
                    result.marked.insert(name.clone(), props);
                }
                if !undecided.is_empty() {
                    result.undecided.insert(name.clone(), undecided);
                }
                continue;
            }
            let mut props = BTreeSet::new();
            let mut undecided = BTreeSet::new();
            let mut closed = false;
            let mut depth = 0usize;
            let mut quote = None;
            let mut escape = false;
            let mut start = 0;
            for (i, c) in args.char_indices() {
                if let Some(q) = quote {
                    if escape {
                        escape = false;
                    } else if c == '\\' {
                        escape = true;
                    } else if c == q {
                        quote = None;
                    }
                    continue;
                }
                match c {
                    '\'' | '"' | '`' => quote = Some(c),
                    '(' | '[' | '{' | '<' => depth = depth.saturating_add(1),
                    ')' if depth == 0 => {
                        Self::parameter(&mut props, &mut undecided, &args[start..i]);
                        closed = true;
                        break;
                    }
                    ')' | ']' | '}' | '>' => depth = depth.saturating_sub(1),
                    ',' if depth == 0 => {
                        Self::parameter(&mut props, &mut undecided, &args[start..i]);
                        start = i + 1;
                    }
                    _ => {}
                }
            }
            if !closed {
                Self::parameter(
                    &mut props,
                    &mut undecided,
                    args[start..].trim_end_matches(')'),
                );
            }
            if !props.is_empty() {
                result.marked.insert(name.clone(), props);
            }
            if !undecided.is_empty() {
                result.undecided.insert(name.clone(), undecided);
            }
        }
        result
    }
    fn parameter(props: &mut BTreeSet<String>, undecided: &mut BTreeSet<String>, parameter: &str) {
        if let Some((name, annotation)) = parameter.split_once(':') {
            let name = name.trim().trim_end_matches('?');
            match binding_annotation(annotation) {
                Some(true) => {
                    props.insert(name.to_owned());
                }
                None => {
                    undecided.insert(name.to_owned());
                }
                Some(false) => {}
            }
        }
    }
}
// Read each structurally named field independently. Upstream emits literal
// contents without escaping them, so quote balance cannot delimit parameters.
fn anchored_props(
    args: &str,
    properties: &JsObject<Value>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let names: BTreeMap<_, _> = properties
        .iter()
        .enumerate()
        .map(|(i, (name, _))| (name.as_str(), i))
        .collect();
    let Some(args) = args.strip_suffix(')') else {
        return (
            BTreeSet::new(),
            properties.iter().map(|(name, _)| name.clone()).collect(),
        );
    };
    let longest = names.keys().map(|n| n.len()).max().unwrap_or(0);
    let mut anchors = vec![None; names.len()];
    let mut counts = vec![0usize; names.len()];
    let mut binding_candidates = vec![false; names.len()];
    for start in std::iter::once(0).chain(args.match_indices(", ").map(|(i, _)| i + 2)) {
        let tail = &args[start..];
        for (end, c) in tail
            .char_indices()
            .take_while(|(i, _)| *i <= longest.saturating_add(1))
        {
            if c != ':' || !tail[end..].starts_with(": ") {
                continue;
            }
            let name = &tail[..end];
            for name in std::iter::once(name).chain(name.strip_suffix('?')) {
                if let Some(i) = names.get(name)
                    && let (Some(anchor), Some(count)) = (anchors.get_mut(*i), counts.get_mut(*i))
                {
                    *count = count.saturating_add(1);
                    *anchor = Some((start, start + end + 2));
                    if let Some(binding) = binding_candidates.get_mut(*i) {
                        *binding |= tail[end + 2..].trim_start().starts_with("$binding<");
                    }
                }
            }
        }
    }
    let mut result = BTreeSet::new();
    let mut undecided = BTreeSet::new();
    for (i, (name, _)) in properties.iter().enumerate() {
        let annotation = (|| {
            if counts.get(i) == Some(&0) {
                return None;
            }
            // Nested fields can repeat a top-level name. If every candidate
            // is unmarked, no ambiguity about its binding status remains.
            if binding_candidates.get(i) == Some(&false) {
                return Some(false);
            }
            if counts.get(i) != Some(&1) {
                return None;
            }
            let (_, begin) = anchors.get(i).copied().flatten()?;
            // Even an unreadable literal's opening quote establishes that its
            // annotation has no outer binding wrapper.
            if !args.get(begin..)?.trim_start().starts_with("$binding<") {
                return Some(false);
            }
            let next = i.saturating_add(1);
            let end = if next == anchors.len() {
                args.len()
            } else {
                if counts.get(next) != Some(&1) {
                    return None;
                }
                let (start, _) = anchors.get(next).copied().flatten()?;
                start.checked_sub(2)?
            };
            binding_annotation(args.get(begin..end)?)
        })();
        match annotation {
            Some(true) => {
                result.insert(name.clone());
            }
            None => {
                undecided.insert(name.clone());
            }
            Some(false) => {}
        }
    }
    (result, undecided)
}
// A reactive array item/union branch can start an unmarked prop's type with
// `$binding<T>[]` or `$binding<T> | U`. Only the complete outer wrapper marks
// the prop schema itself in upstream's WeakSet.
fn binding_annotation(annotation: &str) -> Option<bool> {
    let Some(inner) = annotation.trim().strip_prefix("$binding<") else {
        return Some(false);
    };
    let mut depth = 1usize;
    let mut quote = None;
    let mut escape = false;
    for (i, c) in inner.char_indices() {
        if let Some(q) = quote {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == q {
                quote = None;
                // Upstream does not escape literal text. A quote followed by
                // bare text is ambiguous, not a second binding wrapper whose
                // closing angle can safely be used for this field.
                if inner[i + c.len_utf8()..]
                    .chars()
                    .next()
                    .is_some_and(|next| {
                        !next.is_whitespace()
                            && !matches!(next, '>' | '[' | ']' | ')' | '}' | ',' | '|' | '?')
                    })
                {
                    return None;
                }
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => quote = Some(c),
            '<' => depth = depth.saturating_add(1),
            '>' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(inner[i + 1..].trim().is_empty());
                }
            }
            _ => {}
        }
    }
    None
}
// Prop-result order belongs here: primitives cannot be markers, and elements
// retain their priority. The core merely projects the selected replacement.
pub(super) fn project_marker<'v>(
    value: ExpressionValue<'v>,
    state: &dyn Fn(&str) -> ExpressionValue<'v>,
) -> Option<ExpressionValue<'v>> {
    // Caller-owned records can answer this fixed-key predicate directly;
    // no expression, projection or property coercion is involved.
    if let Some(data) = value.data() {
        match data {
            Value::Obj(o) if matches!(o.get("__reactive"),Some(Value::Str(s)) if s=="assign") => {}
            _ => return None,
        }
    }
    if !value.is_object() || value.property("__reactive").as_str() != Some("assign") {
        return None;
    }
    if value.property("type").as_str() == Some("element")
        && value.property("typeName").as_str().is_some()
        && value.property("props").is_object()
        && value.property("partial").as_bool().is_some()
    {
        return None;
    }
    let target = value.property("target");
    let result = target.as_str().map(state);
    Some(match result {
        Some(v) if !v.is_nullish() => v,
        _ => ExpressionValue::null(),
    })
}
pub(super) fn is_marker(value: &Value) -> bool {
    matches!(value, Value::Obj(o) if matches!(o.get("__reactive"), Some(Value::Str(s)) if s == "assign"))
}
pub(super) fn state_marker_ast(value: &Ast) -> Option<Value> {
    match value {
        Ast::StateRef(n) => Some(marker(Some(Value::Str(n.clone())))),
        _ => None,
    }
}
pub(super) fn state_marker(value: &Value) -> Option<Value> {
    match value {
        Value::Ast(a) => state_marker_ast(a),
        Value::Obj(o) if matches!(o.get("k"),Some(Value::Str(k)) if k=="StateRef") => {
            Some(marker(o.get("n").cloned()))
        }
        _ => None,
    }
}
fn marker(target: Option<Value>) -> Value {
    let mut marker = JsObject::new();
    marker.define("__reactive", Value::Str("assign".into()));
    if let Some(target) = target {
        marker.define("target", target);
    }
    marker.define("expr", Value::Ast(Arc::new(Ast::StateRef("$value".into()))));
    Value::Obj(Arc::new(marker))
}

/// A refused field write; state and pristine status remain unchanged.
#[derive(Debug)]
#[non_exhaustive]
pub enum BindingWriteError {
    /// Not an assign marker with a string target and an expression.
    InvalidMarker,
    /// Evaluation exhausted its shared allowance.
    Cut(crate::EvaluationFlags),
    /// The expression failed, before any state was written.
    Evaluation(EvaluationFailure),
    /// The store refused the evaluated result.
    Store(crate::StoreError),
}
impl std::fmt::Display for BindingWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMarker => f.write_str("invalid binding marker"),
            Self::Cut(_) => f.write_str("binding evaluation cut"),
            Self::Evaluation(e) => f.write_str(&super::failure_message(e)),
            Self::Store(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for BindingWriteError {}

/// Write through an assign marker with default limits. Returns whether state
/// changed. The tree input supplies the same query defaults, loaded references
/// and library classification as drawing. A marker is the capability to write
/// an expression of state, the tree's statements and query data into any state
/// variable. It runs no host action and each write is bounded by the limits.
/// Upstream also trusts markers supplied as data; JSON round trips are allowed.
/// Non-string targets are refused because the Rust store has string keys.
pub fn write_binding(
    input: TreeInput<'_>,
    store: &mut Store,
    marker: &Value,
    value: Value,
) -> Result<bool, BindingWriteError> {
    write_binding_with_limits(input, store, marker, value, Limits::DEFAULT)
}
/// A fresh session reads the current store and exposes the new value through
/// `EvaluationScope::extra`. Null `$value` falls back to state, as upstream does.
/// Query defaults resolve without that overlay; component-valued transforms
/// evaluate without a prop schema context, like upstream state-field writes.
/// A cut or failure never writes; successful results use the store's ordinary
/// admission path (including the total-state admission limits due in #39).
pub fn write_binding_with_limits(
    input: TreeInput<'_>,
    store: &mut Store,
    marker: &Value,
    value: Value,
    limits: Limits,
) -> Result<bool, BindingWriteError> {
    let Value::Obj(fields) = marker else {
        return Err(BindingWriteError::InvalidMarker);
    };
    let (Some(Value::Str(target)), Some(expr)) = (fields.get("target"), fields.get("expr")) else {
        return Err(BindingWriteError::InvalidMarker);
    };
    if !is_marker(marker) {
        return Err(BindingWriteError::InvalidMarker);
    }
    let marker_bytes = input::value_size(
        marker,
        limits,
        &mut Default::default(),
        &mut limits.work_for(usize::MAX),
    )
    .map_err(runner::sizing_cut)?;
    let evaluated = evaluate_kept(input, store, expr, value, marker_bytes, limits)?;
    store
        .set(target, evaluated)
        .map_err(BindingWriteError::Store)
}

#[derive(Clone, Copy)]
pub(super) enum KeptExpression<'a> {
    Ast(&'a Ast),
    Value(&'a Value),
}
impl<'a> From<&'a Ast> for KeptExpression<'a> {
    fn from(value: &'a Ast) -> Self {
        Self::Ast(value)
    }
}
impl<'a> From<&'a Value> for KeptExpression<'a> {
    fn from(value: &'a Value) -> Self {
        Self::Value(value)
    }
}
// Binding writes always have a value overlay and a measured marker.
fn evaluate_kept(
    input: TreeInput<'_>,
    store: &Store,
    expr: &Value,
    value: Value,
    marker_bytes: usize,
    limits: Limits,
) -> Result<crate::StateValue, BindingWriteError> {
    let tree_bytes = input::input_size(&input, limits).map_err(runner::sizing_cut)?;
    let measured = if input.prepared_reference_values.is_none() {
        input.reference_values.map(PreparedReferenceValues::new)
    } else {
        None
    };
    let supplied = input.prepared_reference_values.or(measured.as_ref());
    let bytes = marker_bytes
        .saturating_add(tree_bytes)
        .saturating_add(supplied.map_or(0, |v| v.bytes));
    struct Scope<'a> {
        tree: TreeScope<'a>,
        value: Value,
    }
    impl EvaluationScope for Scope<'_> {
        fn cache_references(&self) -> bool {
            self.tree.cache_references()
        }
        fn wants_unmapped_call(&self, call: &Value, name: &str) -> bool {
            self.tree.wants_unmapped_call(call, name)
        }
        fn call_output(
            &self,
            call: &Ast,
            eval: &ExpressionEvaluator<'_>,
        ) -> openui_lang::ExpressionOutput {
            self.tree.call_output(call, eval)
        }
        fn call_value_output(
            &self,
            call: &Value,
            name: &str,
            eval: &ExpressionEvaluator<'_>,
        ) -> openui_lang::ExpressionOutput {
            self.tree.call_value_output(call, name, eval)
        }

        fn state(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            self.tree.state(name, eval)
        }
        fn reference(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            eval.without_extra(|| self.tree.reference(name, eval))
                .flatten()
        }
        fn prepared_reference(&self, name: &str) -> Option<&crate::PreparedReferenceValue> {
            self.tree.prepared_reference(name)
        }
        fn extra_is_object(&self) -> bool {
            true
        }
        fn extra(&self, name: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            (name == "$value").then(|| self.value.clone())
        }
        fn call(&self, call: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            self.tree.call(call, eval)
        }
        fn call_value_named(
            &self,
            call: &Value,
            name: &str,
            eval: &ExpressionEvaluator<'_>,
        ) -> Option<Value> {
            self.tree.call_value_named(call, name, eval)
        }
        fn wants_each_item(&self, value: &ExpressionValue<'_>, context: bool) -> bool {
            self.tree.wants_each_item(value, context)
        }
        fn each_item(
            &self,
            value: Option<&Value>,
            eval: &ExpressionEvaluator<'_>,
        ) -> Option<Value> {
            self.tree.each_item(value, eval)
        }
    }
    let evaluated = {
        let scope = Scope {
            tree: TreeScope::new(&input, store, supplied),
            value,
        };
        let session = EvaluationSession::with_limits(&scope, bytes, limits);
        let evaluated = session.with_evaluator(|eval| {
            eval.without_extra(|| scope.tree.initialize_queries(input.result, eval));
            eval.with_call_context(false, || match expr {
                Value::Ast(ast) => eval.evaluate_result(ast),
                value => eval.evaluate_value(value),
            })
        });
        match evaluated {
            Some(result) => result,
            None => return Err(BindingWriteError::Cut(session.flags())),
        }
    };
    if evaluated.size_limited || evaluated.depth_limited {
        return Err({
            let mut flags = crate::EvaluationFlags::default();
            flags.size_limited = evaluated.size_limited;
            flags.depth_limited = evaluated.depth_limited;
            BindingWriteError::Cut(flags)
        });
    }
    if let Some(failure) = evaluated.failure {
        return Err(BindingWriteError::Evaluation(failure));
    }
    Ok(evaluated.value)
}
