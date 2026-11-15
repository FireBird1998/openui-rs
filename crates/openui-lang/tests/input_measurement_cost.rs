//! Input byte counts stay stable while supplied parser trees are measured cheaply.
use openui_lang::{
    Ast, EvaluationSession, JsObject, Library, Limits, ParseResult, PreparedInput, Value, parse,
    parse_json,
};
use std::{sync::Arc, time::Instant};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn lib() -> Library {
    Library::from_json_schema(&parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap())
}
fn bytes(input: &PreparedInput) -> usize {
    let limits = Limits::DEFAULT
        .with_result_floor(0)
        .with_amplification(1)
        .with_result_ceiling(usize::MAX)
        .with_work(usize::MAX);
    let mut low = 0usize;
    let mut high = 64 * 1024 * 1024usize;
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        let s = EvaluationSession::with_limits(&(), 0, limits);
        let fits = s.with_evaluator(|e| e.register_input_floor(input) && e.reserve_result(mid));
        if fits { low = mid } else { high = mid - 1 }
    }
    low
}
fn shared() -> ParseResult {
    let leaf = Arc::new(Ast::Str("shared".into()));
    let bin = Ast::BinOp {
        op: "+",
        left: leaf.clone(),
        right: leaf.clone(),
    };
    let args = Arc::new(vec![bin.clone(), Ast::Num(3.)]);
    let mut props = JsObject::new();
    props.define("a", bin);
    let props = Arc::new(props);
    let call = Ast::Comp {
        name: "Box".into(),
        args: args.clone(),
        function: false,
        mapped_props: Some(props.clone()),
    };
    let root = Ast::Arr(Arc::new(vec![
        call.clone(),
        call,
        Ast::Comp {
            name: "Other".into(),
            args,
            function: true,
            mapped_props: Some(props),
        },
    ]));
    let mut p = ParseResult {
        root_expression: Some(Arc::new(root)),
        ..ParseResult::default()
    };
    let value = Value::Ast(leaf);
    p.state_declarations.define("$a", value.clone());
    p.state_declarations.define("$b", value);
    p
}
#[test]
fn parsed_and_shared_inputs_keep_their_measured_byte_counts() {
    let l = lib();
    let programs = [
        parse(
            "$n=0\nroot=Box(Action([@Set($n,$n+1),@OpenUrl(\"a\")]))",
            &l,
            None,
        ),
        parse(
            "$n=0\nq=Query(\"t\",{},{x:1})\nm=Mutation(\"t\",{x:$n})\nroot=Box(Action([@Set($n,q.x > 0 ? -$n+1 : [1,{x:2}][0]),@Run(m),@ToAssistant(\"msg\",{x:$n})]))",
            &l,
            None,
        ),
        shared(),
    ];
    let actual: Vec<_> = programs
        .iter()
        .map(|p| bytes(&PreparedInput::from_parse_result(p, Limits::DEFAULT)))
        .collect();
    println!("pinned byte counts: {actual:?}");
    assert_eq!(actual, vec![418, 1056, 334]);
}
#[test]
fn borrowed_program_measurement_keeps_byte_counts_without_per_node_allocations() {
    for n in [1000, 5000] {
        let text = format!(
            "$n=0\nroot=Box(Action([{}]))",
            vec!["@Set($n,$n+1)"; n].join(",")
        );
        let p = parse(&text, &lib(), None);
        let expected = bytes(&PreparedInput::from_parse_result(&p, Limits::DEFAULT));
        assert_eq!(expected, if n == 1000 { 143174 } else { 715174 });
        let before = test_alloc::allocation_calls();
        let start = Instant::now();
        for _ in 0..100 {
            std::hint::black_box(PreparedInput::from_parse_result(&p, Limits::DEFAULT));
        }
        let elapsed = start.elapsed().as_secs_f64();
        assert!(
            (test_alloc::allocation_calls() - before) / 100 < 64,
            "AST nodes must not allocate measurement metadata"
        );
        println!(
            "{n} steps, {} source bytes, {expected} measured bytes: {:.2} source MB/s, {:.2} measured MB/s, {} allocations/measure",
            text.len(),
            text.len() as f64 * 100. / elapsed / 1e6,
            expected as f64 * 100. / elapsed / 1e6,
            (test_alloc::allocation_calls() - before) / 100
        );
    }
}

#[test]
fn unmeasurable_action_parts_do_not_inflate_the_input_floor() {
    let mut v = Value::Null;
    for _ in 0..1000 {
        v = Value::Arr(Arc::new(vec![v]));
    }
    let input = PreparedInput::from_value(&v, Limits::DEFAULT);
    assert_eq!(bytes(&input), 0, "a refused input part earns no credit");
    let mut o = JsObject::new();
    o.define("type", Value::Str("set".into()));
    o.define("valueAST", v);
    assert_eq!(
        bytes(&PreparedInput::from_value(
            &Value::Obj(Arc::new(o)),
            Limits::DEFAULT
        )),
        0
    );
}
