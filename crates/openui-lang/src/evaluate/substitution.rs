//! One substitution traversal, with adapters for immutable syntax and descriptor
//! data. Adapters preserve their existing copy/assignment and malformed-data
//! semantics; they never convert a whole template to the other representation.
use super::*;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Leaf,
    Member,
    Index,
    Binary,
    Unary,
    Ternary,
    Array,
    Object,
    Call,
    Assign,
}
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Field {
    Object,
    Index,
    Left,
    Right,
    Operand,
    Condition,
    Then,
    Else,
    Value,
}
impl Field {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Object => "obj",
            Self::Index => "index",
            Self::Left => "left",
            Self::Right => "right",
            Self::Operand => "operand",
            Self::Condition => "cond",
            Self::Then => "then",
            Self::Else => "else",
            Self::Value => "value",
        }
    }
}

// Static dispatch keeps the parser's representation and zero-sized policy.
// In particular it pays for neither descriptor inspection nor eager data copies.
pub(super) trait Template: Copy {
    type Variable: ?Sized;
    type Output: std::fmt::Debug;
    type Child;
    type Sequence;
    type Props;
    fn enter<P: SubstitutionPolicy>(
        self,
        var: &Self::Variable,
        value: &Js,
        policy: P,
    ) -> std::ops::ControlFlow<Option<Self::Output>>;
    fn kind(self) -> Kind;
    fn child(self, field: Field) -> Option<Self>;
    fn call_policy<P: SubstitutionPolicy>(self, policy: P) -> (bool, P);
    fn fields(self, fields: &mut [(Field, Option<Self::Child>)]) -> Option<Self::Output>;
    fn assignment(self, visit: impl FnOnce(Self) -> Option<Self::Output>) -> Option<Self::Output>;
    fn shared(child: Self::Output) -> Self::Child;
    fn member(self, child: Option<Self::Output>) -> Option<Self::Output>;
    fn sequence<P: SubstitutionPolicy>(
        self,
        policy: P,
        visit: impl FnMut(Self, P) -> Option<Self::Output>,
    ) -> Option<Self::Sequence>;
    fn array(self, items: Self::Sequence) -> Self::Output;
    fn entries<P: SubstitutionPolicy>(
        self,
        policy: P,
        visit: impl FnMut(Self, P) -> Option<Self::Output>,
    ) -> Option<Self::Output>;
    fn props<P: SubstitutionPolicy>(
        self,
        policy: P,
        visit: impl FnMut(Self, P) -> Option<Self::Output>,
    ) -> Option<Self::Props>;
    fn call(
        self,
        args: Option<Self::Sequence>,
        props: Option<Self::Props>,
        mapped_only: bool,
    ) -> Option<Self::Output>;
}

pub(super) fn walk<N: Template, P: SubstitutionPolicy>(
    node: N,
    var: &N::Variable,
    value: &Js,
    policy: P,
) -> Option<N::Output> {
    crate::stack::grow_stack!(named "substitute(node, var, value)", walk(node, var, value, policy), budget else None);
    if let std::ops::ControlFlow::Break(out) = node.enter(var, value, policy) {
        return out;
    }
    let kind = node.kind();
    let (mapped_only, policy) = if kind == Kind::Call {
        node.call_policy(policy)
    } else if kind == Kind::Assign {
        (false, policy.escaped())
    } else {
        (false, policy)
    };
    let child = |field| {
        node.child(field)
            .and_then(|n| walk(n, var, value, policy))
            .map(N::shared)
    };
    let visit = |n, policy| walk(n, var, value, policy);
    match kind {
        Kind::Leaf => None,
        Kind::Member => node.member(
            node.child(Field::Object)
                .and_then(|n| walk(n, var, value, policy)),
        ),
        Kind::Index => node.fields(&mut [
            (Field::Object, child(Field::Object)),
            (Field::Index, child(Field::Index)),
        ]),
        Kind::Binary => node.fields(&mut [
            (Field::Left, child(Field::Left)),
            (Field::Right, child(Field::Right)),
        ]),
        Kind::Unary => node.fields(&mut [(Field::Operand, child(Field::Operand))]),
        Kind::Ternary => node.fields(&mut [
            (Field::Condition, child(Field::Condition)),
            (Field::Then, child(Field::Then)),
            (Field::Else, child(Field::Else)),
        ]),
        Kind::Assign => node.assignment(|n| walk(n, var, value, policy)),
        Kind::Array => node.sequence(policy, visit).map(|items| node.array(items)),
        Kind::Object => node.entries(policy, visit),
        Kind::Call => {
            // Both representations visit positional syntax before mapped props,
            // even when their bodies can be shared after paying both charges.
            let args = node.sequence(policy, visit);
            let props = node.props(policy, visit);
            node.call(args, props, mapped_only)
        }
    }
}

impl Template for &Ast {
    type Variable = str;
    type Output = Ast;
    type Child = Arc<Ast>;
    type Sequence = Vec<Ast>;
    type Props = Arc<JsObject<Ast>>;
    fn enter<P: SubstitutionPolicy>(
        self,
        var: &str,
        value: &Js,
        _: P,
    ) -> std::ops::ControlFlow<Option<Ast>> {
        if !step(1) {
            return std::ops::ControlFlow::Break(Some(Ast::Null));
        }
        if matches!(self, Ast::Ref(name) if name == var) {
            return std::ops::ControlFlow::Break(Some(literal(value)));
        }
        std::ops::ControlFlow::Continue(())
    }
    fn kind(self) -> Kind {
        match self {
            Ast::Member { .. } => Kind::Member,
            Ast::Index { .. } => Kind::Index,
            Ast::BinOp { .. } => Kind::Binary,
            Ast::UnaryOp { .. } => Kind::Unary,
            Ast::Ternary { .. } => Kind::Ternary,
            Ast::Arr(_) => Kind::Array,
            Ast::Obj(_) => Kind::Object,
            Ast::Comp { .. } => Kind::Call,
            Ast::Assign { .. } => Kind::Assign,
            _ => Kind::Leaf,
        }
    }
    fn child(self, field: Field) -> Option<Self> {
        match (self, field) {
            (Ast::Member { obj, .. } | Ast::Index { obj, .. }, Field::Object) => Some(obj),
            (Ast::Index { index, .. }, Field::Index) => Some(index),
            (Ast::BinOp { left, .. }, Field::Left) => Some(left),
            (Ast::BinOp { right, .. }, Field::Right) => Some(right),
            (Ast::UnaryOp { operand, .. }, Field::Operand) => Some(operand),
            (Ast::Ternary { cond, .. }, Field::Condition) => Some(cond),
            (Ast::Ternary { then, .. }, Field::Then) => Some(then),
            (Ast::Ternary { otherwise, .. }, Field::Else) => Some(otherwise),
            (Ast::Assign { value, .. }, Field::Value) => Some(value),
            _ => None,
        }
    }
    fn call_policy<P: SubstitutionPolicy>(self, policy: P) -> (bool, P) {
        if let Ast::Comp {
            name, mapped_props, ..
        } = self
        {
            let mapped =
                P::SHARE && name != "Each" && mapped_props.is_some() && policy.mapped_only(name);
            (mapped, policy.at_call(name, mapped))
        } else {
            (false, policy)
        }
    }
    fn fields(self, fields: &mut [(Field, Option<Arc<Ast>>)]) -> Option<Ast> {
        if fields.iter().all(|(_, v)| v.is_none()) {
            return None;
        }
        let mut get = |field, old: &Arc<Ast>| {
            fields
                .iter_mut()
                .find(|(f, _)| *f == field)
                .and_then(|(_, v)| v.take())
                .unwrap_or_else(|| old.clone())
        };
        match self {
            Ast::Index { obj, index } => Some(Ast::Index {
                obj: get(Field::Object, obj),
                index: get(Field::Index, index),
            }),
            Ast::BinOp { op, left, right } => Some(Ast::BinOp {
                op,
                left: get(Field::Left, left),
                right: get(Field::Right, right),
            }),
            Ast::UnaryOp { op, operand } => Some(Ast::UnaryOp {
                op,
                operand: get(Field::Operand, operand),
            }),
            Ast::Ternary {
                cond,
                then,
                otherwise,
            } => Some(Ast::Ternary {
                cond: get(Field::Condition, cond),
                then: get(Field::Then, then),
                otherwise: get(Field::Else, otherwise),
            }),
            _ => None,
        }
    }
    fn assignment(self, visit: impl FnOnce(Self) -> Option<Ast>) -> Option<Ast> {
        let Ast::Assign { target, value } = self else {
            return None;
        };
        // Rust evaluates fields in order: main clones the target even when
        // the value is unchanged. Preserve that allocation and its lifetime.
        Some(Ast::Assign {
            target: target.clone(),
            value: Arc::new(visit(value)?),
        })
    }
    fn shared(child: Ast) -> Arc<Ast> {
        Arc::new(child)
    }
    fn member(self, new: Option<Ast>) -> Option<Ast> {
        let Ast::Member { obj, field } = self else {
            return None;
        };
        if let Ast::Obj(entries) = new.as_ref().unwrap_or(obj)
            && let Some((_, v)) = entries.iter().find(|(k, _)| k == field)
        {
            return Some(v.clone());
        }
        Some(Ast::Member {
            obj: Arc::new(new?),
            field: field.clone(),
        })
    }
    fn sequence<P: SubstitutionPolicy>(
        self,
        policy: P,
        mut visit: impl FnMut(Self, P) -> Option<Ast>,
    ) -> Option<Vec<Ast>> {
        let nodes = match self {
            Ast::Arr(nodes) | Ast::Comp { args: nodes, .. } => nodes,
            _ => return None,
        };
        rewrite_list::<_, true>(
            nodes,
            || !crate::progress::stalled_so_far(),
            |n| visit(n, policy),
        )
    }

    fn array(self, items: Vec<Ast>) -> Ast {
        Ast::Arr(Arc::new(items))
    }
    fn entries<P: SubstitutionPolicy>(
        self,
        policy: P,
        mut visit: impl FnMut(Self, P) -> Option<Ast>,
    ) -> Option<Ast> {
        let Ast::Obj(entries) = self else {
            return None;
        };
        let out = rewrite_list::<_, true>(
            entries,
            || !crate::progress::stalled_so_far(),
            |(k, v)| visit(v, policy).map(|v| (k.clone(), v)),
        )?;
        Some(Ast::Obj(Arc::new(out)))
    }

    fn props<P: SubstitutionPolicy>(
        self,
        policy: P,
        mut visit: impl FnMut(Self, P) -> Option<Ast>,
    ) -> Option<Arc<JsObject<Ast>>> {
        let Ast::Comp {
            mapped_props: Some(props),
            ..
        } = self
        else {
            return None;
        };
        let mut changed = false;
        let mut out = JsObject::new();
        for (key, v) in props.iter() {
            if crate::progress::stalled_so_far() {
                break;
            }
            let new = visit(v, policy);
            changed |= new.is_some();
            out.define(key.clone(), new.unwrap_or_else(|| v.clone()));
        }
        changed.then(|| Arc::new(out))
    }
    fn call(
        self,
        new_args: Option<Vec<Ast>>,
        mut new_props: Option<Arc<JsObject<Ast>>>,
        mapped_only: bool,
    ) -> Option<Ast> {
        let Ast::Comp {
            name,
            args,
            function,
            mapped_props,
        } = self
        else {
            return None;
        };
        if mapped_only
            && args.len() == 1
            && let (Some(props), Some(new_args), Some(new_props)) =
                (mapped_props, &new_args, &mut new_props)
            && props.len() == 1
            && let (Some((key, old)), Some(arg), Some(replacement)) =
                (props.iter().next(), args.first(), new_args.first())
            && same_ast_body(arg, old)
            && let Some(target) = Arc::make_mut(new_props).get_mut(key)
        {
            *target = replacement.clone();
        }
        if new_args.is_none() && new_props.is_none() {
            return None;
        }
        Some(Ast::Comp {
            name: name.clone(),
            args: new_args.map_or_else(|| args.clone(), Arc::new),
            function: *function,
            mapped_props: new_props.or_else(|| mapped_props.clone()),
        })
    }
}

// Typed lists copy only after a change. Descriptor .map must still allocate its
// eager scratch, including on unchanged input. One loop preserves both prices.
pub(super) fn rewrite_list<'a, T: Clone, const LAZY: bool>(
    nodes: &'a [T],
    running: impl Fn() -> bool,
    mut visit: impl FnMut(&'a T) -> Option<T>,
) -> Option<Vec<T>> {
    let mut changed = false;
    let mut out = if LAZY { None } else { Some(Vec::new()) };
    for (i, node) in nodes.iter().enumerate() {
        if !running() {
            break;
        }
        let new = visit(node);
        changed |= new.is_some();
        if let Some(new) = new {
            out.get_or_insert_with(|| nodes.get(..i).unwrap_or_default().to_vec())
                .push(new);
        } else if let Some(out) = &mut out {
            out.push(node.clone());
        }
    }
    out.filter(|_| changed)
}
