//! Running queries again through the manager's invalidate command, and the
//! action runner handing a query's `@Run` to the embedder instead of a host event.
use openui_lang::{JsonValue, Library, ParseResult, parse, parse_json};
use openui_runtime::{
    Limits, QueryCommand, QueryCommandError, QueryManager, Store, ToolCall, ToolOutcome, TreeInput,
    Value, evaluate_tree, run_action,
};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);

fn program(source: &str) -> ParseResult {
    parse(source, &Library::from_json_schema(&JsonValue::Null), None)
}

fn evaluate(manager: &mut QueryManager, program: &ParseResult, store: &Store) -> Vec<ToolCall> {
    manager
        .apply(QueryCommand::Evaluate { program, store })
        .unwrap()
        .calls
}

fn run(manager: &mut QueryManager, ids: &[&str]) -> Vec<ToolCall> {
    manager
        .apply(QueryCommand::Invalidate { statement_ids: ids })
        .unwrap()
        .calls
}

fn finish(manager: &mut QueryManager, call: &ToolCall, value: &str) -> Vec<ToolCall> {
    let value = JsonValue::String(value.into());
    manager
        .apply(QueryCommand::Complete {
            id: &call.id,
            outcome: ToolOutcome::Result(Some(&value)),
        })
        .unwrap()
        .calls
}

#[test]
fn many_runs_during_one_call_start_exactly_one_more_call() {
    let store = Store::default();
    let source = program("q = Query(\"search\", {}, \"loading\")");
    let mut manager = QueryManager::default();
    let first = evaluate(&mut manager, &source, &store).remove(0);
    for _ in 0..10_000 {
        assert!(run(&mut manager, &["q", "q"]).is_empty());
        assert!(run(&mut manager, &[]).is_empty());
        assert_eq!(manager.cache_entry_count(), 1);
    }
    let again = finish(&mut manager, &first, "first");
    assert_eq!(
        again.len(),
        1,
        "one remembered run, however many were asked for"
    );
    assert!(again[0].id.is_pending() && !first.id.is_pending());
    assert!(finish(&mut manager, &again[0], "second").is_empty());
    assert!(manager.snapshot_json().contains(r#""q":"second""#));
}

#[test]
fn a_refused_completion_keeps_the_call_and_its_remembered_run() {
    let store = Store::default();
    let source = program("q = Query(\"search\", {}, \"loading\")");
    let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_depth(8));
    let first = evaluate(&mut manager, &source, &store).remove(0);
    run(&mut manager, &["q"]);
    let snapshot = manager.snapshot_json().to_owned();
    let deep = parse_json(&format!("{}1{}", "[".repeat(20), "]".repeat(20))).unwrap();
    assert!(matches!(
        manager.apply(QueryCommand::Complete {
            id: &first.id,
            outcome: ToolOutcome::Result(Some(&deep)),
        }),
        Err(QueryCommandError::Limit(error)) if error.flags.depth_limited
    ));
    assert_eq!(manager.snapshot_json(), snapshot);
    assert!(first.id.is_pending());
    assert_eq!(finish(&mut manager, &first, "fits").len(), 1);
}

#[test]
fn a_remembered_run_calls_with_the_arguments_of_the_query_that_asked() {
    // Both argument records have the cache key of {"v":"__undefined__"}, so
    // the queries share one entry and its call, but they call differently.
    let store = Store::default();
    let source = program(
        "a = Query(\"t\", {v: [0][2]}, \"a\")\nb = Query(\"t\", {v: \"__undefined__\"}, \"b\")",
    );
    let mut manager = QueryManager::default();
    let calls = evaluate(&mut manager, &source, &store);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].arguments_json().as_deref(), Some("{}"));
    run(&mut manager, &["b"]);
    let again = finish(&mut manager, &calls[0], "shared");
    assert_eq!(
        again[0].arguments_json().as_deref(),
        Some(r#"{"v":"__undefined__"}"#)
    );
    // Asked through a, the remembered run uses a's arguments.
    run(&mut manager, &["a"]);
    let third = finish(&mut manager, &again[0], "again");
    assert_eq!(third[0].arguments_json().as_deref(), Some("{}"));
}

#[test]
fn a_remembered_run_is_forgotten_when_no_query_is_left_on_its_key() {
    let mut store = Store::default();
    let source = program("$f = 0\nq = Query(\"t\", {f: $f}, \"q\")");
    store.set("$f", Some(Value::Num(0.0))).unwrap();
    let mut manager = QueryManager::default();
    let first = evaluate(&mut manager, &source, &store).remove(0);
    run(&mut manager, &["q"]);
    store.set("$f", Some(Value::Num(1.0))).unwrap();
    let second = evaluate(&mut manager, &source, &store).remove(0);
    // Delivered to no current query: the call ends and the run is forgotten.
    assert!(finish(&mut manager, &first, "stale").is_empty());
    // The first key is still held as the previous one. Returning to it
    // starts a call there, and its end starts no remembered run.
    store.set("$f", Some(Value::Num(0.0))).unwrap();
    let back = evaluate(&mut manager, &source, &store).remove(0);
    assert!(finish(&mut manager, &back, "back").is_empty());
    assert!(finish(&mut manager, &second, "late").is_empty());
}

fn plan(source: &str) -> (ParseResult, Library, Store, Value) {
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap();
    let library = Library::from_json_schema(&schema);
    let program = parse(source, &library, None);
    let mut store = Store::new();
    let _ = store.initialize(
        program
            .state_declarations
            .iter()
            .map(|(k, v)| (k.as_str(), Some(v))),
        [],
    );
    let tree = evaluate_tree(TreeInput::new(&program).with_library(&library), &store);
    let Some(Value::Element(root)) = &tree.root else {
        panic!("no root element")
    };
    let action = root.props.get("a").unwrap().clone();
    (program, library, store, action)
}

#[test]
fn with_query_runs_a_query_run_is_handed_over_in_step_order_instead_of_an_event() {
    let (program, library, mut store, action) = plan(
        "$n = 1\nq = Query(\"search\", {n: $n})\nm = Mutation(\"save\", {n: $n})\nroot = Box(Action([@Run(q), @Set($n, 2), @Run(q), @ToAssistant(\"done\"), @Run(m)]))",
    );
    let input = || TreeInput::new(&program).with_library(&library);
    let handed = run_action(
        input().with_query_runs(),
        &mut store,
        Some(&action),
        "",
        None,
    );
    assert!(handed.failure.is_none());
    assert_eq!(handed.query_runs, ["q", "q"]);
    let kinds = handed
        .events
        .iter()
        .map(|event| event.to_json().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds.len(),
        2,
        "the message and the mutation still emit: {kinds:?}"
    );
    assert!(kinds[1].contains(r#""refType":"mutation""#));
    assert!(handed.is_suspended());
    let (program, library, mut store, action) =
        plan("q = Query(\"search\")\nroot = Box(Action([@Run(q), @Run(q)]))");
    let plain = run_action(
        TreeInput::new(&program).with_library(&library),
        &mut store,
        Some(&action),
        "",
        None,
    );
    assert!(plain.query_runs.is_empty());
    assert_eq!(plain.events.len(), 2);
}
