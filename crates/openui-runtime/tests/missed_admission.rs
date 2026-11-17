//! Admission edges and Store identity observable through the public API (#40).
use openui_runtime::*;
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

#[test]
fn refusal_messages_identify_depth_size_and_every_store_error() {
    for (error, text) in [
        (StoreError::VersionExhausted, "store version exhausted"),
        (StoreError::WorkLimited, "state measurement work exhausted"),
        (StoreError::DepthLimited, "state value depth exceeded"),
        (
            StoreError::ValueTooLarge,
            "state value size or measurement work exceeded",
        ),
        (StoreError::StateTooLarge, "total state size exceeded"),
    ] {
        assert_eq!(error.to_string(), text);
    }
    for (limits, text) in [
        (Limits::DEFAULT.with_depth(0), "depth limit exceeded"),
        (
            Limits::DEFAULT.with_result_size(0),
            "work or size limit exceeded",
        ),
    ] {
        assert_eq!(
            value_to_json_with_limits(&Value::Null, limits)
                .unwrap_err()
                .to_string(),
            text
        );
    }
}

#[test]
fn binding_names_accept_exact_work_and_quoted_room_and_refuse_either_shortfall() {
    // Retained registry overhead is separate from each quoted name's admission.
    for (name, work, room, accepted) in [
        ("abc", 3, 100, true),
        ("abc", 2, 100, false),
        ("\u{1}", 100, 7, false),
        ("\u{1}", 100, 8, false),
        ("abc", 100, 3, false),
        ("abc", 100, 4, false),
        ("abc", 100, 10, true),
        ("abc", 100, 9, false),
        ("\u{1}", 100, 13, true),
        ("\u{1}", 100, 12, false),
    ] {
        let mut bindings =
            BindingProps::with_limits(Limits::DEFAULT.with_work(work).with_result_size(room));
        assert_eq!(
            bindings.insert(name, "a").is_ok(),
            accepted,
            "{name:?} work={work} room={room}"
        );
    }
}

#[test]
fn store_refusals_distinguish_name_work_value_work_depth_and_size_without_writes() {
    for (name, value, work, room, depth, expected) in [
        (
            "ab",
            Value::Null,
            1,
            100,
            10,
            "state measurement work exhausted",
        ),
        (
            "ab",
            Value::Str("".into()),
            2,
            100,
            10,
            "state value size or measurement work exceeded",
        ),
        (
            "a",
            Value::Str("".into()),
            100,
            1,
            10,
            "state value size or measurement work exceeded",
        ),
        (
            "a",
            Value::Str("".into()),
            100,
            100,
            0,
            "state value depth exceeded",
        ),
    ] {
        let mut store = Store::with_limits(
            Limits::DEFAULT
                .with_work(work)
                .with_result_size(room)
                .with_depth(depth),
        );
        let before = store.snapshot();
        assert_eq!(
            store.set(name, Some(value)).unwrap_err().to_string(),
            expected
        );
        assert!(Arc::ptr_eq(&before, &store.snapshot()));
        assert_eq!(store.version(), 0);
        assert!(!store.is_pristine(name));
    }
    // At exactly value work the scalar is accepted, and at exactly result room
    // it reaches the distinct aggregate-state refusal instead of value refusal.
    for value in [Value::Null, Value::Bool(true), Value::Bool(false)] {
        let bytes = value_to_json(&value).unwrap().len();
        let mut store =
            Store::with_limits(Limits::DEFAULT.with_work(bytes + 2).with_result_size(100));
        assert!(store.set("a", Some(value.clone())).unwrap());
        let mut store = Store::with_limits(Limits::DEFAULT.with_result_size(bytes));
        assert!(matches!(
            store.set("a", Some(value)),
            Err(StoreError::StateTooLarge)
        ));
    }
}

#[test]
fn scalar_admission_keeps_null_undefined_and_booleans_as_allocation_light_as_numbers() {
    let calls = |value: Option<Value>| {
        let mut store = Store::new();
        // Replacing a retained scalar needs no map growth.
        store.set("a", Some(Value::Num(9.))).unwrap();
        let start = test_alloc::allocation_calls();
        store.set("a", value).unwrap();
        test_alloc::allocation_calls() - start
    };
    let control = calls(Some(Value::Num(8.)));
    for value in [
        None,
        Some(Value::Null),
        Some(Value::Bool(false)),
        Some(Value::Bool(true)),
    ] {
        assert_eq!(calls(value), control);
    }
}

#[test]
fn typed_boolean_and_ast_fields_compare_by_value_and_child_identity() {
    let mut store = Store::new();
    let ast = Arc::new(Ast::Bool(true));
    store.set("a", Some(Value::Ast(ast.clone()))).unwrap();
    let snapshot = store.snapshot();
    let start = test_alloc::allocation_calls();
    assert!(!store.set("a", Some(Value::Ast(ast))).unwrap());
    assert_eq!(test_alloc::allocation_calls() - start, 0);
    assert!(Arc::ptr_eq(&snapshot, &store.snapshot()));
    let data = |fields: &[(&str, Value)]| {
        let mut o = JsObject::new();
        for (key, value) in fields {
            o.define(*key, value.clone());
        }
        Value::Obj(Arc::new(o))
    };
    assert!(
        !store
            .set(
                "a",
                Some(data(&[
                    ("k", Value::Str("Bool".into())),
                    ("v", Value::Bool(true))
                ]))
            )
            .unwrap()
    );
    let child = Arc::new(Ast::Null);
    let typed = Value::Ast(Arc::new(Ast::UnaryOp {
        op: "!",
        operand: child.clone(),
    }));
    store.set("a", Some(typed)).unwrap();
    assert!(
        !store
            .set(
                "a",
                Some(data(&[
                    ("k", Value::Str("UnaryOp".into())),
                    ("op", Value::Str("!".into())),
                    ("operand", Value::Ast(child))
                ]))
            )
            .unwrap()
    );
}

#[test]
fn configured_store_limits_refuse_before_retaining_any_state() {
    let mut store = Store::with_limits(Limits::DEFAULT.with_result_size(0));
    assert!(matches!(
        store.set("a", Some(Value::Num(1.))),
        Err(StoreError::ValueTooLarge)
    ));
    assert!(store.snapshot().is_empty());
    assert_eq!(store.version(), 0);
    assert!(!store.is_pristine("a"));
}

#[cfg(unix)]
#[test]
fn configured_store_constructor_finishes_with_its_limits_in_a_cpu_bounded_child() {
    // The with_limits -> Default mutation recursively calls itself. The test
    // profile optimizes that recursion to a loop. Bound child CPU resources,
    // not elapsed time; assert only successful completion and state behavior.
    // Both macOS and Linux provide the POSIX shell resource limit.
    if std::env::var_os("OPENUI_STORE_CONSTRUCTOR_CHILD").is_some() {
        configured_store_limits_refuse_before_retaining_any_state();
        return;
    }
    let status = std::process::Command::new("sh")
        .args([
            "-c",
            "ulimit -c 0; ulimit -t 1; exec \"$@\"",
            "store-constructor",
        ])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "configured_store_constructor_finishes_with_its_limits_in_a_cpu_bounded_child",
            "--test-threads=1",
        ])
        .env("OPENUI_STORE_CONSTRUCTOR_CHILD", "1")
        .status()
        .unwrap();
    assert!(
        status.success(),
        "constructor child did not complete successfully: {status}"
    );
}
