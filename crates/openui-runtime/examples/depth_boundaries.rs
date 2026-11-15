//! Record first-use depth boundaries through public runtime APIs.
#[path = "../tests/support/reuse.rs"]
#[allow(dead_code)]
mod reuse;
use openui_lang::{Library, parse};
use openui_runtime::{
    BindingWriteError, JsObject, Limits, PreparedReferenceValues, Store, TreeInput, Value,
    evaluate_tree_with_limits, run_action, write_binding,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(1024 * 1024 * 1024);
fn nested(mut s: String, n: usize) -> String {
    for i in 0..n {
        s = format!("@Each([1],i{i},{s})")
    }
    s
}
fn main() {
    let bare = std::env::args().nth(1).as_deref() == Some("bare");
    let mut j = reuse::schema();
    j["actions"] = openui_lang::parse_json(
        r#"{"Probe":{"params":{"type":"object","properties":{"value":{}}}}}"#,
    )
    .unwrap();
    let l = Library::from_json_schema(&j);
    let f = reuse::functions(&j);
    for kind in [
        "state",
        "query",
        "prepared",
        "function",
        "param",
        "action",
        "action-function",
        "action-param",
        "binding",
        "alias",
    ] {
        for data in [false, true] {
            for object in [false, true] {
                for level in [0, 12] {
                    for slot in 0..2 {
                        let name = match kind {
                            "query" | "prepared" => "q",
                            "binding" => "$value",
                            "alias" => "$alias",
                            _ => "$d",
                        };
                        let leaf = if bare {
                            name.to_string()
                        } else {
                            match kind {
                                "function" | "action-function" => format!("@Id({name})"),
                                "param" => format!("@Probe({name})"),
                                _ => format!("[{name}]"),
                            }
                        };
                        let inner = nested(leaf, level);
                        let body = if slot == 0 {
                            format!("[{inner},null]")
                        } else {
                            format!("[null,{inner}]")
                        };
                        let src = if kind == "action-param" {
                            format!("root=Box(Action([@Probe({body})]))")
                        } else if kind.starts_with("action") {
                            format!("root=Box(Action([@Set($y,{body})]))")
                        } else {
                            format!("q=Query(\"t\",{{}})\nroot=Box({body})")
                        };
                        let p = parse(&src, &l, None);
                        let p = if data { reuse::shaped(p) } else { p };
                        let plan = evaluate_tree_with_limits(
                            TreeInput::new(&p)
                                .with_library(&l)
                                .with_actions(&j["actions"]),
                            &Store::new(),
                            Limits::DEFAULT,
                        );
                        let action = match &plan.root {
                            Some(Value::Element(e)) => e.props.get("a").cloned(),
                            _ => None,
                        };
                        let mut marker = JsObject::new();
                        marker.define("__reactive", Value::Str("assign".into()));
                        marker.define("target", Value::Str("$out".into()));
                        let ast = Value::Ast(Arc::new(openui_lang::parse_expression(
                            &openui_lang::tokenize(&body),
                        )));
                        let ast = if data {
                            let p = parse("root=Box(null)", &l, None);
                            let mut p = p;
                            p.root.as_mut().unwrap().props.define("a", ast);
                            reuse::shaped(p)
                                .root
                                .unwrap()
                                .props
                                .get("a")
                                .unwrap()
                                .clone()
                        } else {
                            ast
                        };
                        marker.define("expr", ast);
                        let marker = Value::Obj(Arc::new(marker));
                        let mut boundary = 0;
                        for n in 700..=780 {
                            let v = reuse::deep(n, object);
                            let mut s = Store::new();
                            s.set("$d", Some(v.clone())).unwrap();
                            s.set("$alias", Some(v.clone())).unwrap();
                            let mut refs = JsObject::new();
                            refs.define("q", v.clone());
                            let prepared = PreparedReferenceValues::new(&refs);
                            let input = TreeInput::new(&p)
                                .with_library(&l)
                                .with_functions(&f)
                                .with_actions(&j["actions"]);
                            let cut = if kind == "binding" {
                                matches!(write_binding(input,&mut s,&marker,v),Err(BindingWriteError::Cut(f)) if f.depth_limited)
                            } else if kind == "action-param" {
                                let draw =
                                    evaluate_tree_with_limits(input.clone(), &s, Limits::DEFAULT);
                                let action = match &draw.root {
                                    Some(Value::Element(e)) => e.props.get("a"),
                                    _ => None,
                                };
                                run_action(input, &mut s, action, "", None).depth_limited
                            } else if kind.starts_with("action") {
                                run_action(input, &mut s, action.as_ref(), "", None).depth_limited
                            } else {
                                let input = match kind {
                                    "query" => input.with_reference_values(&refs),
                                    "prepared" => input.with_prepared_reference_values(&prepared),
                                    _ => input,
                                };
                                evaluate_tree_with_limits(input, &s, Limits::DEFAULT).depth_limited
                            };
                            if cut {
                                boundary = n;
                                break;
                            }
                        }
                        println!("{kind}\t{data}\t{object}\t{level}\t{slot}\t{boundary}");
                    }
                }
            }
        }
    }
}
