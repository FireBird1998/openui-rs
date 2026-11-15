//! Allocator witnesses for repeated-reference and unique-leaf loop peaks.
use openui_lang::{Ast, JsObject, Library, parse, parse_expression, parse_json, tokenize};
use openui_runtime::{Limits, Store, TreeInput, Value, evaluate_tree_with_limits};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
fn mapped(ast: &mut Ast) {
    if let Ast::Comp {
        name,
        args,
        mapped_props,
        ..
    } = ast
    {
        for arg in Arc::make_mut(args) {
            mapped(arg);
        }
        if name == "Box" {
            let mut props = JsObject::new();
            if let Some(child) = args.first() {
                props.define("a", child.clone());
            }
            *mapped_props = Some(Arc::new(props));
        }
    }
}
fn main() {
    let l = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap(),
    );
    let n = 2652;
    let list = (0..n).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
    for unread in [false, true] {
        let text = if unread {
            format!("L=[{list}]\nq=Query(\"t\",{{}},@Each(L,i,L))\nroot=Box(1)")
        } else {
            format!("L=[{list}]\nroot=Box(@Each(L,i,L))")
        };
        let p = parse(&text, &l, None);
        let s = Store::new();
        test_alloc::reset_peak();
        let before = test_alloc::peak();
        let r = evaluate_tree_with_limits(TreeInput::new(&p), &s, Limits::DEFAULT);
        let peak = test_alloc::peak().saturating_sub(before);
        println!(
            "repeated-reference unread={unread} input={} peak={peak} json={} size={} depth={}",
            text.len(),
            r.to_json().unwrap().len(),
            r.size_limited,
            r.depth_limited
        );
    }
    for rows in [1, 3] {
        for depth in 1..=8 {
            let mut body = (0..depth).fold("$s".to_owned(), |s, i| format!("{s}+i{i}"));
            for i in 0..depth {
                body = format!("@Each($rows,i{i},Box({body}))");
            }
            let mut p = parse("root=Box($s)", &l, None);
            let mut a = parse_expression(&tokenize(&body));
            mapped(&mut a);
            if let Some(root) = p.root.as_mut() {
                root.props.define("a", Value::Ast(Arc::new(a)));
            }
            let mut s = Store::new();
            s.set("$s", Some(Value::Str("x".repeat(1000)))).unwrap();
            s.set(
                "$rows",
                Some(Value::Arr(Arc::new(
                    (0..rows).map(|n| Value::Num(n as f64)).collect(),
                ))),
            )
            .unwrap();
            test_alloc::reset_peak();
            let before = test_alloc::peak();
            let r = evaluate_tree_with_limits(
                TreeInput::new(&p),
                &s,
                Limits::DEFAULT
                    .with_result_size(4_100_000)
                    .with_work(1_000_000_000),
            );
            let peak = test_alloc::peak().saturating_sub(before);
            println!(
                "unique-leaf rows={rows} depth={depth} peak={peak} json={} size={} depth_cut={}",
                r.to_json().unwrap().len(),
                r.size_limited,
                r.depth_limited
            );
        }
    }
}
