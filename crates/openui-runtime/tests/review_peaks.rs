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
// Layout, collection growth and std bookkeeping vary across 64-bit targets.
// Allow 5% or 4 KiB above the measured macOS baseline, whichever is larger.
// The large witnesses still reject doubled leaves and old geometric prop
// capacities. The small absolute cap rejects growth above 4 KiB; the scalar
// control below separately rejects even one extra retained comparison copy.
fn portable_cap(baseline: usize) -> usize {
    baseline + (baseline / 20).max(4 * 1024)
}
// Amplify the small witness's 1 KiB payload so layout tolerance cannot hide an
// extra scalar comparison copy. The equivalent literal needs no state credit;
// only one payload copy plus 4 KiB of metadata may distinguish these peaks.
const SCALAR_BYTES: usize = 32 * 1024;
fn scalar_peak(library: &Library, state: bool) -> (usize, String) {
    let text = "x".repeat(SCALAR_BYTES);
    let source = if state {
        "$s".to_owned()
    } else {
        format!("{text:?}")
    };
    let mut program = parse("root=Box(0)", library, None);
    let mut ast = parse_expression(&tokenize(&format!("@Each($rows,i0,Box({source}+i0))")));
    mapped(&mut ast);
    program.root.as_mut().unwrap().has_dynamic_props = true;
    program
        .root
        .as_mut()
        .unwrap()
        .props
        .define("a", Value::Ast(Arc::new(ast)));
    let mut store = Store::new();
    store.set("$s", Some(Value::Str(text))).unwrap();
    store
        .set("$rows", Some(Value::Arr(Arc::new(vec![Value::Num(0.)]))))
        .unwrap();
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let result = evaluate_tree_with_limits(
        TreeInput::new(&program),
        &store,
        Limits::DEFAULT
            .with_result_size(4_100_000)
            .with_work(1_000_000_000),
    );
    let peak = test_alloc::peak().saturating_sub(before);
    assert!(!result.size_limited && !result.depth_limited && result.errors.is_empty());
    (peak, result.to_json().unwrap())
}

#[test]
fn runtime_container_capacity_keeps_both_review_peaks_below_main() {
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
        let cap = portable_cap(if unread { 198_899_280 } else { 198_899_344 });
        assert!(
            peak <= cap,
            "repeated-reference unread={unread} peak {peak} > {cap}"
        );
        assert_eq!(r.size_limited, !unread);
        assert!(!r.depth_limited && r.errors.is_empty());
        println!(
            "repeated-reference unread={unread} input={} peak={peak} json={} size={} depth={}",
            text.len(),
            r.to_json().unwrap().len(),
            r.size_limited,
            r.depth_limited
        );
    }
    let (literal_peak, literal_json) = scalar_peak(&l, false);
    let (state_peak, state_json) = scalar_peak(&l, true);
    println!("scalar control: literal={literal_peak} state={state_peak}");
    assert_eq!(state_json, literal_json);
    assert!(
        state_peak <= literal_peak + SCALAR_BYTES + 4 * 1024,
        "scalar credit retained an extra string: literal={literal_peak}, state={state_peak}"
    );
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
            if rows == 1 && depth == 1 {
                // Scalar input-credit bookkeeping retains one string comparison
                // copy plus name/index capacity; see TESTING.md (#64).
                assert!(
                    peak <= portable_cap(9_536),
                    "one-row scalar-credit peak {peak}"
                );
                assert!(!r.size_limited && !r.depth_limited && r.errors.is_empty());
                assert_eq!(r.to_json().unwrap().len(), 1_225);
            }
            if rows == 3 && depth == 8 {
                assert!(peak <= portable_cap(3_714_336), "unique-leaf peak {peak}");
                assert!(r.size_limited && !r.depth_limited && r.errors.is_empty());
                assert_eq!(r.to_json().unwrap().len(), 163_573);
            }
            println!(
                "unique-leaf rows={rows} depth={depth} peak={peak} json={} size={} depth_cut={}",
                r.to_json().unwrap().len(),
                r.size_limited,
                r.depth_limited
            );
        }
    }
}
