//! Distinguish output payload from bookkeeping in the same-limit peak comparison.
use openui_lang::{Ast, JsObject, Library, parse, parse_expression, parse_json, tokenize};
use openui_runtime::{Limits, Store, TreeInput, Value, evaluate_tree_with_limits};
use std::{collections::HashSet, sync::Arc};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
fn main() {
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&schema);
    let unique = std::env::args().nth(1).as_deref() == Some("unique");
    let room = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(4_100_000);
    let mut body = if unique {
        "$s+i0+i1+i2+i3+i4+i5+i6"
    } else {
        "$s"
    }
    .to_string();
    for i in 0..7 {
        body = format!("@Each($rows,i{i},Box({body}))");
    }
    let text = if unique {
        "root=Box($s)".into()
    } else {
        format!("root=Box({body})")
    };
    let mut p = parse(&text, &l, None);
    if unique {
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
        let mut expression = parse_expression(&tokenize(&body));
        mapped(&mut expression);
        if let Some(root) = &mut p.root {
            root.props.define("a", Value::Ast(Arc::new(expression)));
        }
    }
    let mut s = Store::new();
    s.set("$s", Some(Value::Str("x".repeat(1000)))).unwrap();
    s.set(
        "$rows",
        Some(Value::Arr(Arc::new(vec![
            Value::Num(0.),
            Value::Num(1.),
            Value::Num(2.),
        ]))),
    )
    .unwrap();
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p),
        &s,
        Limits::DEFAULT
            .with_result_size(room)
            .with_work(1_000_000_000),
    );
    let peak = test_alloc::peak() - before;
    let mut seen = HashSet::new();
    let mut strings = HashSet::new();
    let mut payload = 0;
    let mut pending = r.root.iter().collect::<Vec<_>>();
    while let Some(v) = pending.pop() {
        match v {
            Value::Arr(a) => {
                if seen.insert((0, Arc::as_ptr(a) as usize)) {
                    pending.extend(a.iter());
                }
            }
            Value::Obj(o) => {
                if seen.insert((1, Arc::as_ptr(o) as usize)) {
                    pending.extend(o.values());
                }
            }
            Value::Element(e) => {
                if seen.insert((2, Arc::as_ptr(e) as usize)) {
                    pending.extend(e.props.values());
                }
            }
            Value::Str(s) if strings.insert(s.as_ptr() as usize) => {
                payload += s.len();
            }
            _ => {}
        }
    }
    println!(
        "peak={peak} cut={} unique_strings={} retained_string_payload={payload} json_bytes={}",
        r.size_limited || r.depth_limited,
        strings.len(),
        r.to_json().len()
    );
}
