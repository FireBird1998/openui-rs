//! Each rule is exercised through the public store interface.
use openui_lang::{JsObject, Library, StreamParser, free_json, parse_json};
use openui_runtime::{Ast, Limits, StateValue, Store, Value};
use std::{error::Error, sync::Arc};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);

type TestResult = Result<(), Box<dyn Error>>;
fn shared(v: Value) -> StateValue {
    Some(v)
}
fn num(n: f64) -> StateValue {
    shared(Value::Num(n))
}
fn string(s: &str) -> StateValue {
    shared(Value::Str(s.into()))
}
fn object(entries: &[(&str, Value)]) -> StateValue {
    let mut o = JsObject::new();
    for (k, v) in entries {
        o.define(*k, v.clone());
    }
    shared(Value::Obj(Arc::new(o)))
}
fn map(entries: &[(&str, StateValue)]) -> JsObject<StateValue> {
    let mut o = JsObject::new();
    for (k, v) in entries {
        o.define(*k, v.clone());
    }
    o
}
fn is_num(store: &Store, name: &str, n: f64) {
    assert!(matches!(store.get(name).as_ref(),Some(Value::Num(v)) if *v == n));
}
fn same_handle(a: &StateValue, b: &StateValue) -> bool {
    match (a, b) {
        (None, None) | (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Num(a)), Some(Value::Num(b))) => {
            (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
        }
        (Some(Value::Str(a)), Some(Value::Str(b))) => a == b,
        (Some(Value::Bool(a)), Some(Value::Bool(b))) => a == b,
        (Some(Value::Arr(a)), Some(Value::Arr(b))) => Arc::ptr_eq(a, b),
        (Some(Value::Obj(a)), Some(Value::Obj(b))) => Arc::ptr_eq(a, b),
        (Some(Value::Ast(a)), Some(Value::Ast(b))) => Arc::ptr_eq(a, b),
        (Some(Value::Element(a)), Some(Value::Element(b))) => Arc::ptr_eq(a, b),
        _ => false,
    }
}

// This adapter exercises only public methods. Most rule tests use parsed-map
// shapes; allocation tests below supply borrowed slice iterators directly.
trait InitializeMap {
    fn initialize_map(
        &mut self,
        defaults: &JsObject<StateValue>,
        persisted: &JsObject<StateValue>,
    ) -> Result<bool, openui_runtime::StoreError>;
}
impl InitializeMap for Store {
    fn initialize_map(
        &mut self,
        defaults: &JsObject<StateValue>,
        persisted: &JsObject<StateValue>,
    ) -> Result<bool, openui_runtime::StoreError> {
        let report = self.initialize(
            defaults.iter().map(|(k, v)| (k.as_str(), v.as_ref())),
            persisted.iter().map(|(k, v)| (k.as_str(), v.as_ref())),
        );
        if let Some(refusal) = report.refused.into_iter().next() {
            Err(refusal.error)
        } else {
            Ok(report.changed)
        }
    }
}

#[test]
fn identical_scalars_do_not_raise_version_or_replace_snapshot() -> TestResult {
    let mut store = Store::new();
    for value in [
        shared(Value::Null),
        shared(Value::Bool(true)),
        shared(Value::Bool(false)),
        num(42.0),
        string("hello"),
        string(""),
    ] {
        assert!(store.set("$x", value.clone())?);
        let before = store.snapshot();
        let version = store.version();
        // Owned scalar values compare by value.
        let fresh = value.clone();
        assert!(!store.set("$x", fresh)?);
        assert_eq!(store.version(), version);
        assert!(Arc::ptr_eq(&before, &store.snapshot()));
    }
    Ok(())
}

#[test]
fn object_is_equates_nan_payloads_but_distinguishes_signed_zero() -> TestResult {
    let mut store = Store::new();
    assert!(store.set("$x", num(f64::NAN))?);
    assert!(!store.set("$x", num(f64::from_bits(0x7ff8_0000_0000_0001)))?);
    assert!(store.set("$x", num(0.0))?);
    assert!(store.set("$x", num(-0.0))?);
    assert!(!store.set("$x", num(-0.0))?);
    assert!(store.set("$x", num(0.0))?);
    assert_eq!(store.version(), 4);
    Ok(())
}

#[test]
fn undefined_is_distinct_from_null_and_absent_set_is_ignored() -> TestResult {
    let mut store = Store::new();
    assert!(store.get("$x").is_none());
    assert!(!store.set("$x", None)?);
    assert!(!store.snapshot().has_own("$x"));
    assert!(store.set("$x", shared(Value::Null))?);
    assert!(store.set("$x", None)?);
    assert!(store.get("$x").is_none());
    assert!(store.snapshot().has_own("$x"));
    assert!(!store.set("$x", None)?);
    assert!(store.initialize_map(&map(&[("$new", None)]), &JsObject::new())?);
    assert!(store.snapshot().has_own("$new"));
    Ok(())
}

#[test]
fn shallow_objects_ignore_key_order_and_keep_the_original_identity() -> TestResult {
    let mut store = Store::new();
    let original = object(&[("a", Value::Num(1.0)), ("b", Value::Null)]);
    store.set("$x", original.clone())?;
    assert!(!store.set("$x", object(&[("b", Value::Null), ("a", Value::Num(1.0))]))?);
    assert!(same_handle(&original, &store.get("$x")));
    assert_eq!(store.version(), 1);
    assert!(store.set("$x", object(&[("a", Value::Num(2.0)), ("b", Value::Null)]))?);
    assert!(store.set("$x", object(&[("a", Value::Num(2.0)), ("c", Value::Null)]))?);
    assert!(store.set("$x", object(&[("a", Value::Num(2.0))]))?);
    assert_eq!(store.version(), 4);
    Ok(())
}

#[test]
fn shallow_comparison_uses_object_is_for_numbers_and_child_identity() -> TestResult {
    let mut store = Store::new();
    store.set("$x", object(&[("a", Value::Num(f64::NAN))]))?;
    assert!(!store.set("$x", object(&[("a", Value::Num(f64::NAN))]))?);
    store.set("$x", object(&[("a", Value::Num(0.0))]))?;
    assert!(store.set("$x", object(&[("a", Value::Num(-0.0))]))?);
    let child = Value::Arr(Arc::new(vec![Value::Num(1.0)]));
    store.set("$x", object(&[("a", child.clone())]))?;
    assert!(!store.set("$x", object(&[("a", child)]))?);
    assert!(store.set(
        "$x",
        object(&[("a", Value::Arr(Arc::new(vec![Value::Num(1.0)])))])
    )?);
    let child = Value::Obj(Arc::new(JsObject::new()));
    store.set("$x", object(&[("a", child.clone())]))?;
    assert!(!store.set("$x", object(&[("a", child)]))?);
    assert!(store.set(
        "$x",
        object(&[("a", Value::Obj(Arc::new(JsObject::new())))])
    )?);
    Ok(())
}

#[test]
fn arrays_use_identity_and_read_back_values_can_be_set_again() -> TestResult {
    let mut store = Store::new();
    let array = Arc::new(vec![Value::Num(1.0)]);
    store.set("$x", shared(Value::Arr(array.clone())))?;
    assert!(!store.set("$x", store.get("$x"))?);
    assert!(!store.set("$x", shared(Value::Arr(array)))?);
    assert!(store.set("$x", shared(Value::Arr(Arc::new(vec![Value::Num(1.0)]))))?);
    assert_eq!(store.version(), 2);
    Ok(())
}

#[test]
fn ignored_plain_sets_clear_pristine_including_shallow_objects() -> TestResult {
    for value in [num(1.0), object(&[("a", Value::Num(1.0))])] {
        let mut store = Store::new();
        store.initialize_map(&map(&[("$x", value.clone())]), &JsObject::new())?;
        let same = value.as_ref().map(|v| match v {
            Value::Obj(o) => Value::Obj(Arc::new((**o).clone())),
            v => v.clone(),
        });
        assert!(!store.set("$x", same)?);
        assert!(!store.initialize_map(&map(&[("$x", num(2.0))]), &JsObject::new())?);
        assert!(same_handle(&value, &store.get("$x")));
        assert_eq!(store.version(), 1);
    }
    Ok(())
}

#[test]
fn pristine_sets_restore_replaceable_defaults_even_when_ignored() -> TestResult {
    let mut store = Store::new();
    store.set("$x", num(1.0))?;
    assert!(!store.set_pristine("$x", num(1.0))?);
    assert!(store.initialize_map(&map(&[("$x", num(2.0))]), &JsObject::new())?);
    is_num(&store, "$x", 2.0);
    assert!(store.set_pristine("$x", num(3.0))?);
    assert!(store.initialize_map(&map(&[("$x", num(4.0))]), &JsObject::new())?);
    is_num(&store, "$x", 4.0);
    assert_eq!(store.version(), 4);
    Ok(())
}

#[test]
fn persisted_values_win_and_clear_pristine_even_if_identical() -> TestResult {
    let mut store = Store::new();
    let saved = num(1.0);
    store.initialize_map(&map(&[("$x", saved.clone())]), &JsObject::new())?;
    assert!(!store.initialize_map(&map(&[("$x", num(2.0))]), &map(&[("$x", saved)]))?);
    assert!(!store.initialize_map(&map(&[("$x", num(3.0))]), &JsObject::new())?);
    is_num(&store, "$x", 1.0);
    assert!(store.initialize_map(
        &map(&[("$x", num(4.0))]),
        &map(&[("$x", None), ("$extra", num(5.0))])
    )?);
    assert!(store.snapshot().has_own("$x"));
    assert!(store.get("$x").is_none());
    is_num(&store, "$extra", 5.0);
    Ok(())
}

#[test]
fn initialize_uses_identity_without_shallow_comparison_and_changes_once_per_batch() -> TestResult {
    let mut store = Store::new();
    let first = object(&[("a", Value::Num(1.0))]);
    let second = object(&[("a", Value::Num(1.0))]);
    store.initialize_map(&map(&[("$x", first)]), &JsObject::new())?;
    assert!(store.initialize_map(
        &map(&[("$x", second.clone()), ("$y", num(2.0))]),
        &JsObject::new()
    )?);
    assert_eq!(store.version(), 2);
    assert!(same_handle(&second, &store.get("$x")));
    assert!(!store.initialize_map(&map(&[("$x", second)]), &JsObject::new())?);
    assert!(store.initialize_map(
        &JsObject::new(),
        &map(&[("$x", object(&[("a", Value::Num(1.0))]))])
    )?);
    assert_eq!(store.version(), 3);
    Ok(())
}

#[test]
fn disappearing_declarations_keep_values_and_pristine_status() -> TestResult {
    let mut store = Store::new();
    store.initialize_map(&map(&[("$x", num(1.0))]), &JsObject::new())?;
    assert!(!store.initialize_map(&JsObject::new(), &JsObject::new())?);
    is_num(&store, "$x", 1.0);
    assert!(store.initialize_map(&map(&[("$y", num(3.0))]), &JsObject::new())?);
    is_num(&store, "$x", 1.0);
    assert!(store.initialize_map(
        &map(&[("$x", num(2.0)), ("$y", num(3.0))]),
        &JsObject::new()
    )?);
    is_num(&store, "$x", 2.0);
    is_num(&store, "$y", 3.0);
    Ok(())
}

#[test]
fn snapshot_retains_old_values_and_uses_javascript_own_key_order() -> TestResult {
    let mut store = Store::new();
    for key in ["b", "10", "__proto__", "2", "a", "constructor"] {
        store.set(key, num(1.0))?;
    }
    let old = store.snapshot();
    store.set("b", num(2.0))?;
    assert!(matches!(old.get("b").and_then(Option::as_ref),Some(Value::Num(n)) if *n==1.0));
    assert_eq!(
        store
            .snapshot()
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>(),
        ["2", "10", "b", "__proto__", "a", "constructor"]
    );
    assert!(!Arc::ptr_eq(&old, &store.snapshot()));
    Ok(())
}

#[test]
fn dispose_clears_values_without_resetting_version_and_can_be_reused() -> TestResult {
    let mut store = Store::new();
    store.set_pristine("$ghost", None)?;
    store.set("$x", num(1.0))?;
    let old = store.snapshot();
    assert!(store.dispose());
    assert_eq!(store.version(), 2);
    assert!(store.snapshot().is_empty());
    assert!(old.has_own("$x"));
    assert!(store.get("$x").is_none());
    let empty = store.snapshot();
    assert!(!store.dispose());
    assert!(!Arc::ptr_eq(&empty, &store.snapshot()));
    assert_eq!(store.version(), 2);
    store.set("$x", num(2.0))?;
    assert!(!store.initialize_map(&map(&[("$x", num(3.0))]), &JsObject::new())?);
    is_num(&store, "$x", 2.0);
    assert_eq!(store.version(), 3);
    Ok(())
}

#[test]
fn dispose_releases_pristine_names_even_when_all_values_were_ignored() -> TestResult {
    let mut store = Store::new();
    let prefix = "x".repeat(512);
    for i in 0..10_000 {
        assert!(!store.set_pristine(&format!("{prefix}{i}"), None)?);
    }
    assert!(store.snapshot().is_empty());
    test_alloc::reset_peak();
    let held = test_alloc::peak();
    assert!(!store.dispose());
    test_alloc::reset_peak();
    let released = test_alloc::peak();
    assert!(
        held.saturating_sub(released) > 5_000_000,
        "pristine names retained: before={held}, after={released}"
    );
    assert_eq!(store.version(), 0);
    Ok(())
}

#[test]
fn streamed_parse_defaults_grow_while_user_edits_survive() -> TestResult {
    let program = "root = Col([$title,$items])\n$title = \"Complete title\"\n$items = [1,2,3]\n";
    let schema = parse_json("{}")?;
    let library = Arc::new(Library::from_json_schema(&schema));
    free_json(schema);
    let mut parser = StreamParser::new(library, Some("Col"));
    let mut store = Store::new();
    for end in (4..program.len())
        .step_by(4)
        .chain(std::iter::once(program.len()))
    {
        let parsed = parser.set(program.get(..end).ok_or("prefix boundary")?);
        let report = store.initialize(
            parsed
                .state_declarations
                .iter()
                .map(|(k, v)| (k.as_str(), Some(v))),
            std::iter::empty(),
        );
        assert!(report.refused.is_empty());
        if end == 40 {
            store.set("$items", string("user"))?;
        }
    }
    assert!(matches!(store.get("$title").as_ref(),Some(Value::Str(s)) if s=="Complete title"));
    assert!(matches!(store.get("$items").as_ref(),Some(Value::Str(s)) if s=="user"));
    Ok(())
}

#[test]
fn reads_and_writes_share_deep_container_bodies_on_a_small_stack() -> TestResult {
    std::thread::Builder::new()
        .name("store::worker".into())
        .stack_size(64 * 1024)
        .spawn(|| -> Result<(), String> {
            let mut value = Value::Str("x".repeat(1_000_000));
            for _ in 0..50_000 {
                value = Value::Arr(Arc::new(vec![value]));
            }
            let handle = shared(value);
            let limits = Limits::DEFAULT
                .with_result_size(4_000_000)
                .with_depth(60_000)
                .with_work(5_000_000);
            let mut store = Store::with_limits(limits);
            let text = shared(Value::Str("t".repeat(1_000_000)));
            store
                .set("$text", text.clone())
                .map_err(|e| e.to_string())?;
            assert!(same_handle(&store.get("$text"), &text));
            store
                .set("$deep", handle.clone())
                .map_err(|e| e.to_string())?;
            let read = store.get("$deep");
            assert!(same_handle(&read, &handle));
            let before = store.snapshot();
            store.set("$other", num(1.0)).map_err(|e| e.to_string())?;
            assert!(same_handle(&store.get("$deep"), &handle));
            assert!(same_handle(
                &before.get("$deep").cloned().flatten(),
                &handle
            ));
            assert!(!store.set("$deep", read).map_err(|e| e.to_string())?);
            store.dispose();
            Ok(())
        })?
        .join()
        .map_err(|_| "worker panicked")?
        .map_err(|e| -> Box<dyn Error> { e.into() })?;
    Ok(())
}

#[test]
fn typed_asts_compare_their_own_fields_without_copying_children() -> TestResult {
    let child = Arc::new(Ast::Null);
    let args = Arc::new(vec![Ast::Null]);
    let mut props = JsObject::new();
    props.define("n", Ast::Null);
    let nodes = vec![
        Ast::Str("hi".into()),
        Ast::Num(f64::NAN),
        Ast::Bool(true),
        Ast::Null,
        Ast::Ref("a".into()),
        Ast::Ph("a".into()),
        Ast::StateRef("$a".into()),
        Ast::RuntimeRef {
            n: "q".into(),
            ref_type: openui_lang::RefType::Query,
        },
        Ast::Comp {
            name: "F".into(),
            args: args.clone(),
            function: true,
            mapped_props: Some(Arc::new(props)),
        },
        Ast::Arr(args),
        Ast::Obj(Arc::new(vec![("a".into(), Ast::Null)])),
        Ast::BinOp {
            op: "+",
            left: child.clone(),
            right: child.clone(),
        },
        Ast::UnaryOp {
            op: "!",
            operand: child.clone(),
        },
        Ast::Ternary {
            cond: child.clone(),
            then: child.clone(),
            otherwise: child.clone(),
        },
        Ast::Member {
            obj: child.clone(),
            field: "a".into(),
        },
        Ast::Index {
            obj: child.clone(),
            index: child.clone(),
        },
        Ast::Assign {
            target: "$a".into(),
            value: child,
        },
    ];
    let mut store = Store::new();
    for node in nodes {
        let original = shared(Value::Ast(Arc::new(node.clone())));
        store.set("$x", original.clone())?;
        assert!(!store.set("$x", shared(Value::Ast(Arc::new(node))))?);
        assert!(same_handle(&original, &store.get("$x")));
    }
    store.set("$x", shared(Value::Ast(Arc::new(Ast::Num(1.0)))))?;
    assert!(!store.set(
        "$x",
        object(&[("k", Value::Str("Num".into())), ("v", Value::Num(1.0))])
    )?);
    store.set(
        "$x",
        object(&[("k", Value::Str("Num".into())), ("v", Value::Num(2.0))]),
    )?;
    assert!(!store.set("$x", shared(Value::Ast(Arc::new(Ast::Num(2.0)))))?);
    assert!(store.set("$x", shared(Value::Ast(Arc::new(Ast::Num(3.0)))))?);
    let node = || Ast::Member {
        obj: Arc::new(Ast::Null),
        field: "x".into(),
    };
    store.set("$x", shared(Value::Ast(Arc::new(node()))))?;
    assert!(store.set("$x", shared(Value::Ast(Arc::new(node()))))?);
    Ok(())
}

#[test]
fn typed_elements_keep_identity_and_distinct_props_are_a_change() -> TestResult {
    let element = Arc::new(openui_lang::Element {
        type_name: "Text".into(),
        props: JsObject::new(),
        partial: false,
        has_dynamic_props: false,
        statement_id: None,
    });
    let mut store = Store::new();
    store.set("$x", shared(Value::Element(element.clone())))?;
    assert!(!store.set("$x", shared(Value::Element(element.clone())))?);
    assert!(store.set("$x", shared(Value::Element(Arc::new((*element).clone()))))?);
    Ok(())
}

#[test]
fn pristine_read_includes_absent_names_and_tracks_ignored_writes() -> TestResult {
    let mut store = Store::new();
    assert!(!store.is_pristine("$ghost"));
    assert!(!store.set_pristine("$ghost", None)?);
    assert!(store.is_pristine("$ghost"));
    assert!(!store.snapshot().has_own("$ghost"));
    assert!(!store.set("$ghost", None)?);
    assert!(!store.is_pristine("$ghost"));
    let defaults = [("$x", num(1.0))];
    let report = store.initialize(
        defaults.iter().map(|(k, v)| (*k, v.as_ref())),
        std::iter::empty(),
    );
    assert!(report.changed && report.refused.is_empty());
    assert!(store.is_pristine("$x"));
    assert!(!store.set("$x", num(1.0))?);
    assert!(!store.is_pristine("$x"));
    assert!(!store.set_pristine("$x", num(1.0))?);
    assert!(store.is_pristine("$x"));
    let report = store.initialize(
        std::iter::empty(),
        defaults.iter().map(|(k, v)| (*k, v.as_ref())),
    );
    assert!(!report.changed && report.refused.is_empty());
    assert!(!store.is_pristine("$x"));
    store.set_pristine("$ghost", None)?;
    store.dispose();
    assert!(!store.is_pristine("$ghost"));
    assert!(!store.is_pristine("$x"));
    Ok(())
}

#[test]
fn unchanged_initialization_and_ignored_sets_allocate_nothing_with_many_names() -> TestResult {
    for count in [1, 4096] {
        let before_setup = test_alloc::allocation_calls();
        let mut defaults: Vec<_> = (0..count)
            .map(|i| {
                (
                    format!("$state_{i}_{}", "x".repeat(64)),
                    string(&format!("default {i}")),
                )
            })
            .collect();
        defaults.push(("$counter".into(), num(0.0)));
        defaults.push(("$pristine".into(), num(0.0)));
        let mut store = Store::new();
        let report = store.initialize(
            defaults.iter().map(|(k, v)| (k.as_str(), v.as_ref())),
            std::iter::empty(),
        );
        assert!(report.changed && report.refused.is_empty());
        assert!(
            test_alloc::allocation_calls() > before_setup,
            "counter must observe setup allocations"
        );
        let snapshot = store.snapshot(); // any accidental COW is expensive
        let before = test_alloc::allocation_calls();
        for _ in 0..8 {
            let report = store.initialize(
                defaults.iter().map(|(k, v)| (k.as_str(), v.as_ref())),
                std::iter::empty(),
            );
            assert!(!report.changed && report.refused.is_empty());
            assert!(!store.set("$counter", num(0.0))?);
            assert!(!store.set_pristine("$pristine", num(0.0))?);
        }
        let calls = test_alloc::allocation_calls().saturating_sub(before);
        assert_eq!(calls, 0, "{count} names: no-op calls allocated");
        assert!(Arc::ptr_eq(&snapshot, &store.snapshot()));
        drop(snapshot);
        // With no retained snapshot, replacing one scalar must not copy the
        // other names or their strings either.
        let before = test_alloc::allocation_calls();
        assert!(store.set("$counter", num(1.0))?);
        let calls = test_alloc::allocation_calls().saturating_sub(before);
        assert_eq!(calls, 0, "{count} names: real set copied unrelated names");
        is_num(&store, "$counter", 1.0);
    }
    Ok(())
}

#[test]
fn reads_clone_scalar_strings_and_share_container_bodies() -> TestResult {
    let mut store = Store::new();
    let text = "long scalar ".repeat(1000);
    store.set("$s", Some(Value::Str(text.clone())))?;
    let snapshot = store.snapshot();
    let Some(Some(Value::Str(stored))) = snapshot.get("$s") else {
        return Err("string missing".into());
    };
    let read_value = store.get("$s");
    let Some(Value::Str(read)) = &read_value else {
        return Err("string missing".into());
    };
    assert_eq!(read, &text);
    assert_ne!(read.as_ptr(), stored.as_ptr());
    let object = object(&[("n", Value::Num(1.0))]);
    store.set("$o", object.clone())?;
    assert!(same_handle(&store.get("$o"), &object));
    Ok(())
}

#[test]
fn duplicate_initialization_entries_use_only_the_last_value() -> TestResult {
    let values = [
        ("$a", num(1.0)),
        ("$a", num(2.0)),
        ("$b", num(0.0)),
        ("$b", None),
    ];
    let mut store = Store::new();
    let report = store.initialize(
        values.iter().map(|(k, v)| (*k, v.as_ref())),
        std::iter::empty(),
    );
    assert!(report.changed && report.refused.is_empty());
    let before = store.snapshot();
    let version = store.version();
    let report = store.initialize(
        values.iter().map(|(k, v)| (*k, v.as_ref())),
        std::iter::empty(),
    );
    assert!(!report.changed && report.refused.is_empty());
    assert_eq!(store.version(), version);
    assert!(Arc::ptr_eq(&before, &store.snapshot()));
    is_num(&store, "$a", 2.0);
    assert!(store.snapshot().has_own("$b") && store.get("$b").is_none());
    assert!(store.is_pristine("$a") && store.is_pristine("$b"));
    for _ in 0..2 {
        let report = store.initialize(
            std::iter::empty(),
            values.iter().map(|(k, v)| (*k, v.as_ref())),
        );
        assert!(!report.changed && report.refused.is_empty());
        assert_eq!(store.version(), version);
        assert!(Arc::ptr_eq(&before, &store.snapshot()));
        assert!(!store.is_pristine("$a") && !store.is_pristine("$b"));
    }
    Ok(())
}
