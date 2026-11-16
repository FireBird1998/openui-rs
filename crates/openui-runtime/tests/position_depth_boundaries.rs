//! Cached walks preserve independently measured first-use depth boundaries.
#[path = "support/reuse.rs"]
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
        s = format!("@Each([1],i{i},{s})");
    }
    s
}
#[test]
fn first_same_deeper_and_shallower_uses_preserve_their_exact_boundary() {
    let mut j = reuse::schema();
    j["actions"] = openui_lang::parse_json(
        r#"{"Probe":{"params":{"type":"object","properties":{"value":{}}}}}"#,
    )
    .unwrap();
    let l = Library::from_json_schema(&j);
    let f = reuse::functions(&j);
    // Recorded by examples/depth_boundaries.rs after separating lookup from
    // consumption depth. Each row has ONE value use; a previous read cannot
    // hide a deeper use. The archived depth-first-use files retain main's rules.
    let rows = include_str!("support/depth-current-use.tsv")
        .lines()
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let rows_bare = include_str!("support/depth-current-use-bare.tsv")
        .lines()
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .collect::<Vec<_>>();
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
                for bare_prime in [false, true] {
                    for positions in [
                        [None, Some(12)],
                        [Some(12), Some(12)],
                        [Some(0), Some(12)],
                        [Some(12), Some(0)],
                    ] {
                        if bare_prime && positions[0].is_none() {
                            continue;
                        }
                        let boundary = positions
                            .iter()
                            .enumerate()
                            .filter_map(|(slot, level)| {
                                level.map(|level| {
                                    let rows = if bare_prime && slot == 0 {
                                        &rows_bare
                                    } else {
                                        &rows
                                    };
                                    let row = rows
                                        .iter()
                                        .find(|r| {
                                            r[0] == kind
                                                && r[1] == data.to_string()
                                                && r[2] == object.to_string()
                                                && r[3] == level.to_string()
                                                && r[4] == slot.to_string()
                                        })
                                        .unwrap();
                                    row[5].parse::<usize>().unwrap()
                                })
                            })
                            .min()
                            .unwrap();
                        let mut values = Vec::new();
                        for (slot, level) in positions.iter().enumerate() {
                            let name = match kind {
                                "query" | "prepared" => "q",
                                "binding" => "$value",
                                "alias" if slot == 1 => "$alias",
                                _ => "$d",
                            };
                            let leaf = match kind {
                                "function" | "action-function" => format!("@Id({name})"),
                                "param" => format!("@Probe({name})"),
                                _ => format!("[{name}]"),
                            };
                            let leaf = if bare_prime && slot == 0 {
                                name.to_string()
                            } else {
                                leaf
                            };
                            values.push(level.map_or_else(|| "null".into(), |n| nested(leaf, n)));
                        }
                        let body = format!("[{},{}]", values[0], values[1]);
                        let source = if kind == "action-param" {
                            format!("root=Box(Action([@Probe({body})]))")
                        } else if kind.starts_with("action") {
                            format!("root=Box(Action([@Set($y,{body})]))")
                        } else {
                            format!("q=Query(\"t\",{{}})\nroot=Box({body})")
                        };
                        let p = parse(&source, &l, None);
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
                            let mut p = parse("root=Box(null)", &l, None);
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
                        for n in [boundary - 2, boundary - 1, boundary] {
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
                                matches!(write_binding(input,&mut s,&marker,v),Err(BindingWriteError::Cut(flags)) if flags.depth_limited)
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
                            assert_eq!(
                                cut,
                                n >= boundary,
                                "kind={kind} data={data} object={object} bare_prime={bare_prime} positions={positions:?} n={n} first_cut={boundary}"
                            );
                        }
                    }
                }
            }
        }
    }
}
