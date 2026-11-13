//! Own fields of the core's typed JavaScript objects, including shaped elements.
use super::*;
pub(super) fn shaped_element(o: &JsObject<Value>) -> bool {
    matches!(o.get("type"),Some(Value::Str(s)) if s=="element")
        && matches!(o.get("typeName"), Some(Value::Str(_)))
        && o.get("props").is_some_and(object)
        && matches!(o.get("partial"), Some(Value::Bool(_)))
}
pub(super) fn own_fields(v: &Value) -> std::borrow::Cow<'_, JsObject<Value>> {
    let mut o = JsObject::new();
    match v {
        Value::Obj(obj) => return std::borrow::Cow::Borrowed(obj.as_ref()),
        Value::Arr(items) => {
            for (i, v) in items.iter().enumerate() {
                o.define(i.to_string(), v.clone());
            }
        }
        Value::Element(e) => {
            o.define("type", Value::Str("element".into()));
            o.define("typeName", Value::Str(e.type_name.clone()));
            o.define("props", Value::Obj(Arc::new(e.props.clone())));
            o.define("partial", Value::Bool(e.partial));
            o.define("hasDynamicProps", Value::Bool(e.has_dynamic_props));
            if let Some(id) = &e.statement_id {
                o.define("statementId", Value::Str(id.clone()));
            }
        }
        Value::Ast(a) => {
            let child = |a: &Arc<Ast>| Value::Ast(a.clone());
            let list = |a: &[Ast]| {
                Value::Arr(Arc::new(
                    a.iter().map(|a| Value::Ast(Arc::new(a.clone()))).collect(),
                ))
            };
            let text = |s: &str| Value::Str(s.to_owned());
            let kind = match &**a {
                Ast::Comp { .. } => "Comp",
                Ast::Str(_) => "Str",
                Ast::Num(_) => "Num",
                Ast::Bool(_) => "Bool",
                Ast::Null => "Null",
                Ast::Arr(_) => "Arr",
                Ast::Obj(_) => "Obj",
                Ast::Ref(_) => "Ref",
                Ast::Ph(_) => "Ph",
                Ast::StateRef(_) => "StateRef",
                Ast::RuntimeRef { .. } => "RuntimeRef",
                Ast::BinOp { .. } => "BinOp",
                Ast::UnaryOp { .. } => "UnaryOp",
                Ast::Ternary { .. } => "Ternary",
                Ast::Member { .. } => "Member",
                Ast::Index { .. } => "Index",
                Ast::Assign { .. } => "Assign",
            };
            o.define("k", text(kind));
            match &**a {
                Ast::Str(s) => o.define("v", text(s)),
                Ast::Num(n) => o.define("v", Value::Num(*n)),
                Ast::Bool(b) => o.define("v", Value::Bool(*b)),
                Ast::Ref(n) | Ast::Ph(n) | Ast::StateRef(n) => o.define("n", text(n)),
                Ast::RuntimeRef { n, ref_type } => {
                    o.define("n", text(n));
                    o.define("refType", text(ref_type.as_str()));
                }
                Ast::Arr(a) => o.define("els", list(a)),
                Ast::Obj(a) => o.define(
                    "entries",
                    Value::Arr(Arc::new(
                        a.iter()
                            .map(|(k, v)| {
                                Value::Arr(Arc::new(vec![text(k), Value::Ast(Arc::new(v.clone()))]))
                            })
                            .collect(),
                    )),
                ),
                Ast::BinOp { op, left, right } => {
                    o.define("op", text(op));
                    o.define("left", child(left));
                    o.define("right", child(right));
                }
                Ast::UnaryOp { op, operand } => {
                    o.define("op", text(op));
                    o.define("operand", child(operand));
                }
                Ast::Member { obj, field } => {
                    o.define("obj", child(obj));
                    o.define("field", text(field));
                }
                Ast::Index { obj, index } => {
                    o.define("obj", child(obj));
                    o.define("index", child(index));
                }
                Ast::Ternary {
                    cond,
                    then,
                    otherwise,
                } => {
                    o.define("cond", child(cond));
                    o.define("then", child(then));
                    o.define("else", child(otherwise));
                }
                Ast::Assign { target, value } => {
                    o.define("target", text(target));
                    o.define("value", child(value));
                }
                Ast::Comp {
                    name,
                    args,
                    function,
                    mapped_props,
                } => {
                    o.define("name", text(name));
                    o.define("args", list(args));
                    if *function {
                        o.define("fn", Value::Bool(true));
                    }
                    if let Some(p) = mapped_props {
                        o.define(
                            "mappedProps",
                            Value::Obj(Arc::new(
                                p.iter()
                                    .map(|(k, v)| (k.clone(), Value::Ast(Arc::new(v.clone()))))
                                    .fold(JsObject::new(), |mut o, (k, v)| {
                                        o.define(k, v);
                                        o
                                    }),
                            )),
                        );
                    }
                }
                Ast::Null => {}
            }
        }
        _ => {}
    }
    std::borrow::Cow::Owned(o)
}
