//! A refused reset leaves every target's value, pristine status and version intact.
use openui_lang::{Library, parse, parse_json};
use openui_runtime::{Limits, Store, TreeInput, Value, run_action, run_action_with_limits};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn value(s: &str) -> Value {
    Value::from_json(&parse_json(s).unwrap())
}
#[test]
fn an_invalid_reset_target_does_not_reset_earlier_targets() {
    let p = parse(
        "$a=1\n$b=2\nroot=Box(null)",
        &Library::from_json_schema(&parse_json("{}").unwrap()),
        None,
    );
    let mut s = Store::new();
    s.set("$a", Some(Value::Num(10.))).unwrap();
    s.set("$b", Some(Value::Num(20.))).unwrap();
    let version = s.version();
    let a = value(
        r#"{"steps":[{"type":"reset","targets":["$a",1,"$b"]},{"type":"open_url","url":"after"}]}"#,
    );
    let r = run_action(TreeInput::new(&p), &mut s, Some(&a), "", None);
    assert!(r.failure.is_some());
    assert!(r.events.is_empty());
    assert_eq!(s.get("$a"), Some(Value::Num(10.)));
    assert_eq!(s.get("$b"), Some(Value::Num(20.)));
    assert!(!s.is_pristine("$a") && !s.is_pristine("$b"));
    assert_eq!(s.version(), version);
}
#[test]
fn a_later_oversized_default_does_not_commit_a_reset_prefix() {
    let p = parse(
        "$a=1\n$large=\"abcdefghijklmnopqrstuvwxyz\"\n$b=2\nroot=Box(null)",
        &Library::from_json_schema(&parse_json("{}").unwrap()),
        None,
    );
    let mut s = Store::new();
    s.set("$a", Some(Value::Num(10.))).unwrap();
    s.set("$b", Some(Value::Num(20.))).unwrap();
    let version = s.version();
    let a = value(r#"{"steps":[{"type":"reset","targets":["$a","$large","$b"]}]}"#);
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut s,
        Some(&a),
        "",
        None,
        Limits::DEFAULT.with_result_size(16),
    );
    assert!(r.size_limited);
    assert_eq!(s.get("$a"), Some(Value::Num(10.)));
    assert_eq!(s.get("$b"), Some(Value::Num(20.)));
    assert!(!s.is_pristine("$a"));
    assert_eq!(s.version(), version);
}

#[test]
fn a_failed_reset_keeps_effects_of_completed_steps() {
    let p = parse(
        "$a=1\n$b=2\nroot=Box(null)",
        &Library::from_json_schema(&parse_json("{}").unwrap()),
        None,
    );
    let mut s = Store::new();
    let a = value(
        r#"{"steps":[{"type":"set","target":"$a","valueAST":{"k":"Num","v":10}},{"type":"reset","targets":["$a",1,"$b"]}]}"#,
    );
    let r = run_action(TreeInput::new(&p), &mut s, Some(&a), "", None);
    assert_eq!(r.failure.unwrap().step, Some(1));
    assert_eq!(s.get("$a"), Some(Value::Num(10.)));
    assert_eq!(s.get("$b"), None);
    assert!(!s.is_pristine("$a"));
    assert_eq!(s.version(), 1);
}

#[test]
fn repeated_reset_targets_commit_the_same_default_once() {
    let p = parse(
        "$a=1\nroot=Box(null)",
        &Library::from_json_schema(&parse_json("{}").unwrap()),
        None,
    );
    let mut s = Store::new();
    s.set("$a", Some(Value::Num(10.))).unwrap();
    let a = value(r#"{"steps":[{"type":"reset","targets":["$a","$a"]}]}"#);
    let r = run_action(TreeInput::new(&p), &mut s, Some(&a), "", None);
    assert!(r.failure.is_none());
    assert_eq!(s.get("$a"), Some(Value::Num(1.)));
    assert!(s.is_pristine("$a"));
    assert_eq!(s.version(), 2);
}
