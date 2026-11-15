//! Scripted binding calls, compared byte for byte with the public Rust runtime.
use openui_ffi::*;
use openui_lang::{JsObject, Library, ParseResult, Value};
use openui_runtime::{BindingProps, PreparedReferenceValues, Store, TreeInput};
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

fn text(path: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../../conformance/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn json(s: &str) -> Value {
    Value::from_json(&openui_lang::parse_json(s).unwrap())
}
fn value_json(value: &Value) -> String {
    let mut tree =
        openui_runtime::evaluate_tree(TreeInput::new(&ParseResult::default()), &Store::new());
    tree.root = Some(value.clone());
    tree.errors.clear();
    tree.to_json()
        .strip_prefix("{\"root\":")
        .unwrap()
        .strip_suffix(",\"errors\":[]}")
        .unwrap()
        .into()
}
fn snapshot(store: &Store) -> String {
    let mut object = JsObject::new();
    for (k, v) in store.snapshot().iter() {
        if let Some(v) = v {
            object.define(k, v.clone());
        }
    }
    value_json(&Value::Obj(Arc::new(object)))
}
fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).into()).collect()
}
fn at(mut value: Value, path: &[String]) -> Value {
    for key in path {
        value = match &value {
            Value::Element(e) if key == "props" => Value::Obj(Arc::new(e.props.clone())),
            Value::Obj(o) => o.get(key).unwrap().clone(),
            Value::Arr(a) => a[key.parse::<usize>().unwrap()].clone(),
            _ => panic!("bad path {path:?}"),
        };
    }
    value
}

struct Scenario {
    ffi: Arc<RuntimeSession>,
    library: Arc<ComponentLibrary>,
    core_library: Arc<Library>,
    schema: serde_json::Value,
    bindings: BindingProps,
    program: Arc<ParseResult>,
    store: Store,
    refs: JsObject<Value>,
}
impl Scenario {
    fn corpus(schema: &str, spec: Option<&str>, case: &str) -> Self {
        Self::new(
            &text(&format!("schemas/{schema}.json")),
            spec.map(|s| text(&format!("schemas/{s}.json"))),
            &text(&format!("cases/runtime/{case}.oui")),
        )
    }
    fn new(schema: &str, spec: Option<String>, program: &str) -> Self {
        let library = ComponentLibrary::new(schema.into()).unwrap();
        let schema: serde_json::Value = openui_lang::parse_json(schema).unwrap();
        let core_library = Arc::new(Library::from_json_schema(&schema));
        let bindings = spec
            .as_ref()
            .map(|s| {
                let value = json(s);
                let v = match &value {
                    Value::Obj(o) => o.get("library").unwrap_or(&value),
                    _ => &value,
                };
                BindingProps::from_prompt_spec(v)
            })
            .unwrap_or_default();
        let ffi = RuntimeSession::new(library.clone(), spec).unwrap();
        let mut s = Self {
            ffi,
            library,
            core_library,
            schema,
            bindings,
            program: Arc::new(ParseResult::default()),
            store: Store::new(),
            refs: JsObject::new(),
        };
        s.replace(program);
        s
    }
    fn replace(&mut self, program: &str) {
        let output = parse(program.into(), self.library.clone(), None);
        self.ffi.set_program(output.program).unwrap();
        self.program = Arc::new(openui_lang::parse(program, &self.core_library, None));
    }
    fn initialize(&mut self, saved: Option<&str>) {
        let saved_value = saved.map(json);
        let persisted = match &saved_value {
            Some(Value::Obj(o)) => o.as_ref().clone(),
            _ => JsObject::new(),
        };
        let want = self.store.initialize(
            self.program
                .state_declarations
                .iter()
                .map(|(k, v)| (k.as_str(), Some(v))),
            persisted.iter().map(|(k, v)| (k.as_str(), Some(v))),
        );
        let got = self.ffi.initialize(saved.map(str::to_owned)).unwrap();
        assert_eq!(got.changed, want.changed);
        assert_eq!(got.refused.len(), want.refused.len());
        self.check_state();
    }
    fn references(&mut self, s: &str) {
        let Value::Obj(ref object) = json(s) else {
            panic!()
        };
        self.refs = object.as_ref().clone();
        self.ffi.set_reference_values(s.into()).unwrap();
    }
    fn input<'a>(&'a self, prepared: &'a PreparedReferenceValues<'a>) -> TreeInput<'a> {
        let input = TreeInput::from_shared_result(&self.program)
            .with_library(&self.core_library)
            .with_binding_props(&self.bindings)
            .with_prepared_reference_values(prepared);
        self.schema
            .get("actions")
            .map_or(input.clone(), |a| input.with_actions(a))
    }
    fn evaluate(&self) -> (EvaluationOutput, Value) {
        let refs = PreparedReferenceValues::new(&self.refs);
        let want = openui_runtime::evaluate_tree(self.input(&refs), &self.store);
        let got = self.ffi.evaluate().unwrap();
        assert_eq!(got.json, want.to_json());
        assert_eq!(
            (got.size_limited, got.depth_limited),
            (want.size_limited, want.depth_limited)
        );
        (got, want.root.unwrap_or(Value::Null))
    }
    fn check_state(&self) {
        assert_eq!(self.ffi.snapshot().unwrap(), snapshot(&self.store));
        assert_eq!(self.ffi.version().unwrap(), self.store.version());
        for (name, value) in self.store.snapshot().iter() {
            assert_eq!(
                self.ffi.get(name.clone()).unwrap(),
                value.as_ref().map(value_json)
            );
            assert_eq!(
                self.ffi.is_pristine(name.clone()).unwrap(),
                self.store.is_pristine(name)
            );
        }
    }
    fn run(&mut self, parts: &[&str], succeeded: bool) {
        let (got, root) = self.evaluate();
        let path = path(parts);
        let action = at(root, &path);
        let handle = self.ffi.action_at(got.evaluation, path).unwrap();
        let got = self.ffi.run(Some(handle), "clicked".into(), None).unwrap();
        let program = self.program.clone();
        let refs = PreparedReferenceValues::new(&self.refs);
        let mut input = TreeInput::from_shared_result(&program)
            .with_library(&self.core_library)
            .with_binding_props(&self.bindings)
            .with_prepared_reference_values(&refs);
        if let Some(actions) = self.schema.get("actions") {
            input = input.with_actions(actions);
        }
        let want = openui_runtime::run_action(
            input.clone(),
            &mut self.store,
            Some(&action),
            "clicked",
            None,
        );
        compare_run(&got, &want);
        self.check_state();
        if let Some(continuation) = want.continuation.as_ref() {
            let handle = got.continuation.unwrap();
            let got = self.ffi.resume(handle.clone(), succeeded).unwrap();
            let want =
                openui_runtime::resume_action(input, &mut self.store, continuation, succeeded)
                    .unwrap();
            compare_run(&got, &want);
            assert!(matches!(
                self.ffi.resume(handle, true),
                Err(OpenUiError::ConsumedHandle)
            ));
            self.check_state();
        }
    }
}
fn compare_run(got: &ActionOutput, want: &openui_runtime::ActionRun) {
    assert_eq!(
        got.events,
        want.events.iter().map(|v| v.to_json()).collect::<Vec<_>>()
    );
    assert_eq!(
        (got.size_limited, got.depth_limited),
        (want.size_limited, want.depth_limited)
    );
    assert_eq!(got.continuation.is_some(), want.continuation.is_some());
    assert_eq!(
        got.failure.as_ref().map(|f| f.step),
        want.failure.as_ref().map(|f| f.step.map(|i| i as u64))
    );
}

#[test]
fn corpus_state_bindings_and_query_data_match_runtime_bytes() {
    for saved in [None, Some(r#"{"$n":19,"$total":6}"#)] {
        let mut s = Scenario::corpus("bindings", Some("bindings-spec"), "binding-plain");
        s.initialize(saved);
        let (output, root) = s.evaluate();
        let marker = at(root, &path(&["props", "value"]));
        let refs = PreparedReferenceValues::new(&s.refs);
        let expected = openui_runtime::write_binding(
            s.input(&refs),
            &mut Store::new(),
            &marker,
            Value::Num(4.),
        );
        assert!(expected.is_ok());
        let program = s.program.clone();
        let want = openui_runtime::write_binding(
            TreeInput::from_shared_result(&program)
                .with_library(&s.core_library)
                .with_binding_props(&s.bindings),
            &mut s.store,
            &marker,
            Value::Num(4.),
        )
        .unwrap();
        assert_eq!(
            s.ffi
                .write_binding(value_json(&marker), "4".into())
                .unwrap(),
            want
        );
        assert!(output.json.contains("__reactive"));
        s.check_state();
        s.evaluate();
    }
    let mut s = Scenario::corpus("bindings", Some("bindings-spec"), "binding-write-supplied");
    s.initialize(None);
    s.references(r#"{"q":{"n":20},"m":{"status":"done"}}"#);
    let (_, root) = s.evaluate();
    let marker = at(root, &path(&["props", "value"]));
    let program = s.program.clone();
    let refs = PreparedReferenceValues::new(&s.refs);
    let want = openui_runtime::write_binding(
        TreeInput::from_shared_result(&program)
            .with_library(&s.core_library)
            .with_binding_props(&s.bindings)
            .with_prepared_reference_values(&refs),
        &mut s.store,
        &marker,
        Value::Num(4.),
    )
    .unwrap();
    assert_eq!(
        s.ffi
            .write_binding(value_json(&marker), "4".into())
            .unwrap(),
        want
    );
    s.check_state();
    s.evaluate();
}

#[test]
fn reference_preparation_uses_its_own_allowance_like_the_rust_runtime() {
    let mut s = Scenario::new(
        &text("schemas/runtime.json"),
        None,
        "q=Query(\"q\",{})\nroot=Box(q.length)",
    );
    s.references(&format!(
        "{{\"q\":{}1{}}}",
        "[".repeat(500),
        "]".repeat(500)
    ));
    let (tree, _) = s.evaluate();
    assert!(!tree.depth_limited && !tree.size_limited);
    // Failed replacement must preserve the already prepared supply too.
    assert!(matches!(
        s.ffi.set_reference_values("[]".into()),
        Err(OpenUiError::InvalidJson { .. })
    ));
    s.evaluate();
    for work in [0, 30, 64_000_000] {
        let session = RuntimeSession::with_limits(
            s.library.clone(),
            None,
            RuntimeLimits {
                reference_preparation: ParseLimits {
                    depth: Some(32_768),
                    work: Some(work),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        session
            .set_program(
                parse(
                    "q=Query(\"q\",{})\nroot=Box(q.length)".into(),
                    s.library.clone(),
                    None,
                )
                .program,
            )
            .unwrap();
        session
            .set_reference_values(format!("{{\"q\":{}1{}}}", "[".repeat(500), "]".repeat(500)))
            .unwrap();
        let prepared = PreparedReferenceValues::with_limits(
            &s.refs,
            openui_lang::Limits::DEFAULT
                .with_depth(32_768)
                .with_work(work as usize),
        );
        let expected = openui_runtime::evaluate_tree(s.input(&prepared), &s.store);
        let got = session.evaluate().unwrap();
        assert_eq!(got.json, expected.to_json());
        assert_eq!(
            (got.size_limited, got.depth_limited),
            (expected.size_limited, expected.depth_limited)
        );
    }
}

#[test]
fn corpus_actions_cover_every_step_and_both_mutation_outcomes() {
    for success in [false, true] {
        let mut s = Scenario::corpus("runtime-actions", None, "action-steps");
        s.initialize(None);
        s.run(&["props", "a"], success);
    }
    let mut s = Scenario::new(
        &text("schemas/runtime-actions.json"),
        None,
        "$n=3\nroot=Box(Action([@Go(\"ok\"),@Set($n,$n+1),@OpenUrl(\"url\")]))",
    );
    s.initialize(None);
    s.run(&["props", "a"], true);
    assert_eq!(s.ffi.get("$n".into()).unwrap().as_deref(), Some("4"));
}

#[test]
fn corpus_each_output_and_nested_array_plans_are_addressable() {
    let mut s = Scenario::corpus("runtime-actions", None, "action-loop-capture");
    s.initialize(None);
    s.references("{}");
    s.ffi
        .set("$rows".into(), Some(r#"[{"id":2,"text":"row"}]"#.into()))
        .unwrap();
    s.store
        .set("$rows", Some(json(r#"[{"id":2,"text":"row"}]"#)))
        .unwrap();
    s.run(&["props", "a", "0", "props", "c"], true);
    s.run(&["props", "a", "0", "props", "d", "1"], true);
}

#[test]
fn corpus_form_fields_save_flags_and_events_match_runtime() {
    let mut s = Scenario::corpus("runtime-runner", Some("runtime-runner-spec"), "run-forms");
    s.initialize(None);
    for form in [None, Some("form")] {
        for save in [false, true] {
            let want = openui_runtime::set_field_value(
                &mut s.store,
                form,
                Some("Field"),
                "name",
                Some(json("\"answer\"")),
                save,
            )
            .unwrap();
            let got = s
                .ffi
                .set_field(
                    form.map(str::to_owned),
                    Some("Field".into()),
                    "name".into(),
                    Some("\"answer\"".into()),
                    save,
                )
                .unwrap();
            assert_eq!(
                (got.changed, got.save_requested),
                (want.changed, want.save_requested)
            );
            assert_eq!(
                s.ffi
                    .get_field(form.map(str::to_owned), "name".into())
                    .unwrap(),
                openui_runtime::get_field_value(&s.store, form, "name")
                    .as_ref()
                    .map(value_json)
            );
            s.check_state();
        }
        for streaming in [true, false, false] {
            let want = openui_runtime::initialize_field_default(
                &mut s.store,
                form,
                Some("Field"),
                "default",
                Some(Value::Num(7.)),
                streaming,
            )
            .unwrap();
            let got = s
                .ffi
                .initialize_field(
                    form.map(str::to_owned),
                    Some("Field".into()),
                    "default".into(),
                    Some("7".into()),
                    streaming,
                )
                .unwrap();
            assert_eq!(
                got.map(|v| (v.changed, v.save_requested)),
                want.map(|v| (v.changed, v.save_requested))
            );
            s.check_state();
        }
        let (tree, root) = s.evaluate();
        let action = at(root, &path(&["props", "a"]));
        let handle = s
            .ffi
            .action_at(tree.evaluation, path(&["props", "a"]))
            .unwrap();
        let got = s
            .ffi
            .run(Some(handle), "form".into(), form.map(str::to_owned))
            .unwrap();
        let program = s.program.clone();
        let want = openui_runtime::run_action(
            TreeInput::from_shared_result(&program).with_library(&s.core_library),
            &mut s.store,
            Some(&action),
            "form",
            form,
        );
        compare_run(&got, &want);
    }
}

#[test]
fn streaming_shares_real_programs_and_replacement_keeps_user_state() {
    let source = text("cases/runtime/binding-plain.oui");
    let mut s = Scenario::corpus("bindings", Some("bindings-spec"), "binding-plain");
    let stream = StreamSession::new(s.library.clone(), None);
    let mut core = openui_lang::StreamParser::new(s.core_library.clone(), None);
    for (i, chunk) in source.as_bytes().chunks(5).enumerate() {
        let chunk = std::str::from_utf8(chunk).unwrap();
        let mut output = stream.push(chunk.into());
        // The record's JSON and flags are data, never the session program.
        output.json = "not JSON".into();
        output.size_limited = true;
        s.ffi.set_program(output.program).unwrap();
        s.program = Arc::new(core.push(chunk));
        s.initialize(None);
        s.evaluate();
        if i == 9 {
            s.ffi.set("$n".into(), Some("123".into())).unwrap();
            s.store.set("$n", Some(Value::Num(123.))).unwrap();
        }
    }
    s.ffi.set_program(stream.finish().program).unwrap();
    s.program = Arc::new(core.finish());
    s.initialize(None);
    s.evaluate();
    s.replace("$n=999\nroot=Field($n)");
    s.initialize(None);
    s.evaluate();
    assert_eq!(s.ffi.get("$n".into()).unwrap().as_deref(), Some("123"));
}

#[test]
fn suspended_runs_use_new_program_state_and_reference_inputs() {
    let schema = text("schemas/runtime-actions.json");
    let source = "$n=0\nm=Mutation(\"m\",{})\nq=Query(\"q\",{},{x:1})\nroot=Box(Action([@Run(m),@Set($n,q.x+$edit),@OpenUrl(\"after\")]))";
    let mut s = Scenario::new(&schema, None, source);
    s.initialize(None);
    let (tree, root) = s.evaluate();
    let handle = s
        .ffi
        .action_at(tree.evaluation, path(&["props", "a"]))
        .unwrap();
    let action = at(root, &path(&["props", "a"]));
    let got = s.ffi.run(Some(handle), "".into(), None).unwrap();
    let old_program = s.program.clone();
    let want = openui_runtime::run_action(
        TreeInput::from_shared_result(&old_program).with_library(&s.core_library),
        &mut s.store,
        Some(&action),
        "",
        None,
    );
    compare_run(&got, &want);
    s.replace("$n=500\nq=Query(\"new\",{},{x:2})\nroot=Box($n)");
    s.initialize(None);
    s.references(r#"{"q":{"x":41}}"#);
    s.ffi.set("$edit".into(), Some("1".into())).unwrap();
    s.store.set("$edit", Some(Value::Num(1.))).unwrap();
    let got = s.ffi.resume(got.continuation.unwrap(), true).unwrap();
    let refs = PreparedReferenceValues::new(&s.refs);
    let program = s.program.clone();
    let want = openui_runtime::resume_action(
        TreeInput::from_shared_result(&program)
            .with_library(&s.core_library)
            .with_prepared_reference_values(&refs),
        &mut s.store,
        want.continuation.as_ref().unwrap(),
        true,
    )
    .unwrap();
    compare_run(&got, &want);
    s.check_state();
    assert_eq!(s.ffi.get("$n".into()).unwrap().as_deref(), Some("42"));
}

#[test]
fn legacy_and_no_action_events_match_runtime() {
    let mut s = Scenario::corpus("runtime-actions", None, "action-steps");
    s.initialize(None);
    for raw in [
        None,
        Some(r#"{"type":"open_url","url":"https://example.test","context":{"n":1}}"#),
        Some(r#"{"type":"Go","params":{"x":"forged"}}"#),
    ] {
        let value = raw.map(json);
        let program = s.program.clone();
        let input = TreeInput::from_shared_result(&program)
            .with_library(&s.core_library)
            .with_actions(s.schema.get("actions").unwrap());
        let want = openui_runtime::run_action(input, &mut s.store, value.as_ref(), "message", None);
        let got = if let Some(raw) = raw {
            s.ffi.run_legacy(raw.into(), "message".into(), None)
        } else {
            s.ffi.run(None, "message".into(), None)
        }
        .unwrap();
        compare_run(&got, &want);
        s.check_state();
    }
}

#[test]
fn custom_plans_written_by_bindings_keep_identity_and_json_copies_do_not() {
    let mut s = Scenario::new(
        &text("schemas/runtime-actions.json"),
        None,
        "root=Box($saved=@Go(\"real\"),$saved,$copy)",
    );
    s.ffi.mark_binding("Box".into(), "a".into()).unwrap();
    s.bindings.insert("Box", "a");
    s.initialize(None);
    let (_, root) = s.evaluate();
    let marker = at(root, &path(&["props", "a"]));
    s.ffi
        .write_binding(value_json(&marker), "null".into())
        .unwrap();
    let program = s.program.clone();
    openui_runtime::write_binding(
        TreeInput::from_shared_result(&program)
            .with_library(&s.core_library)
            .with_actions(s.schema.get("actions").unwrap()),
        &mut s.store,
        &marker,
        Value::Null,
    )
    .unwrap();
    let serialized = s.ffi.get("$saved".into()).unwrap().unwrap();
    s.ffi.set("$copy".into(), Some(serialized.clone())).unwrap();
    s.store.set("$copy", Some(json(&serialized))).unwrap();
    s.run(&["props", "b"], true);
    let tree = s.ffi.evaluate().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&tree.json).unwrap()["root"]["props"]["c"]["steps"],
        serde_json::json!([])
    );
    assert!(matches!(
        s.ffi.run_legacy(serialized, "".into(), None),
        Err(OpenUiError::InvalidJson { .. })
    ));
}

#[test]
fn prompt_binding_uncertainty_and_unimplemented_functions_are_visible() {
    let schema = r#"{"$defs":{"Box":{"properties":{"a":{},"b":{}}}},"functions":{"Identity":{"params":{"type":"object","properties":{"value":{}}}}}}"#;
    let spec = r#"{"components":{"Box":"Box(a: $binding<number>, b: \"bad, b: x\")"},"schema":{"$defs":{"Box":{"properties":{"a":{},"b":{}}}}}}"#;
    let s = Scenario::new(schema, Some(spec.into()), "root=Box(@Identity(1),$n)\n$n=2");
    let want: Vec<_> = s
        .bindings
        .undecided()
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .collect();
    let got: Vec<_> = s
        .ffi
        .undecided_bindings()
        .unwrap()
        .into_iter()
        .map(|v| (v.component, v.prop))
        .collect();
    assert_eq!(got, want);
    assert!(!want.is_empty());
    let (tree, _) = s.evaluate();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&tree.json).unwrap()["root"]["props"]["a"],
        serde_json::Value::Null
    );
}

#[test]
fn foreign_stale_disposed_and_consumed_capabilities_have_no_effects() {
    let source = "$n=0\nm=Mutation(\"m\",{})\nroot=Box(Action([@Run(m),@Set($n,7)]))";
    let schema = text("schemas/runtime-actions.json");
    let mut a = Scenario::new(&schema, None, source);
    a.initialize(None);
    let mut b = Scenario::new(&schema, None, source);
    b.initialize(None);
    let tree = a.ffi.evaluate().unwrap();
    let action = a
        .ffi
        .action_at(tree.evaluation.clone(), path(&["props", "a"]))
        .unwrap();
    let before = b.ffi.snapshot().unwrap();
    assert!(matches!(
        b.ffi.run(Some(action.clone()), "".into(), None),
        Err(OpenUiError::ForeignHandle)
    ));
    assert!(matches!(
        a.ffi
            .action_at(tree.evaluation.clone(), path(&["props", "missing"])),
        Err(OpenUiError::InvalidHandle)
    ));
    let run = a.ffi.run(Some(action.clone()), "".into(), None).unwrap();
    let continuation = run.continuation.unwrap();
    assert!(matches!(
        b.ffi.resume(continuation.clone(), true),
        Err(OpenUiError::ForeignHandle)
    ));
    assert_eq!(b.ffi.snapshot().unwrap(), before);
    a.ffi.evaluate().unwrap();
    let before = a.ffi.snapshot().unwrap();
    assert!(matches!(
        a.ffi.run(Some(action), "".into(), None),
        Err(OpenUiError::StaleHandle)
    ));
    assert!(matches!(
        a.ffi.action_at(tree.evaluation, path(&["props", "a"])),
        Err(OpenUiError::StaleHandle)
    ));
    assert_eq!(a.ffi.snapshot().unwrap(), before);
    a.ffi.resume(continuation.clone(), false).unwrap();
    assert!(matches!(
        a.ffi.resume(continuation, true),
        Err(OpenUiError::ConsumedHandle)
    ));
    a.ffi.dispose();
    a.ffi.dispose();
    assert!(matches!(a.ffi.get("$n".into()), Err(OpenUiError::Disposed)));
    assert!(matches!(a.ffi.evaluate(), Err(OpenUiError::Disposed)));
    assert!(matches!(
        a.ffi.set_reference_values("{}".into()),
        Err(OpenUiError::Disposed)
    ));
}

#[test]
fn finished_custom_steps_cannot_gain_authority_through_any_json_route() {
    let mut s = Scenario::new(
        &text("schemas/runtime-actions.json"),
        None,
        "root=Box($plan,$slot)\n$plan=null\n$slot=null",
    );
    s.initialize(None);
    let forged = r#"{"steps":[{"type":"custom_action","name":"Go","params":{"x":"forged"}}]}"#;
    let before = s.ffi.snapshot().unwrap();
    assert!(matches!(
        s.ffi.run_legacy(forged.into(), "".into(), None),
        Err(OpenUiError::InvalidJson { .. })
    ));
    assert_eq!(s.ffi.snapshot().unwrap(), before);
    s.ffi.set("$plan".into(), Some(forged.into())).unwrap();
    s.ffi.mark_binding("Box".into(), "b".into()).unwrap();
    s.ffi.set_program(parse(
        r#"root=Box($plan,$slot={steps:[{type:"custom_action",name:"Go",params:{x:"forged"}}]})"#.into(),
        s.library.clone(), None).program).unwrap();
    let tree = s.ffi.evaluate().unwrap();
    let tree: serde_json::Value = serde_json::from_str(&tree.json).unwrap();
    let marker = tree["root"]["props"]["b"].to_string();
    s.ffi.write_binding(marker, "null".into()).unwrap();
    let stored = s.ffi.get("$slot".into()).unwrap().unwrap();
    assert!(matches!(
        s.ffi.run_legacy(stored, "".into(), None),
        Err(OpenUiError::InvalidJson { .. })
    ));
    let tree = s.ffi.evaluate().unwrap();
    let action = s
        .ffi
        .action_at(tree.evaluation, path(&["props", "a"]))
        .unwrap();
    assert!(
        s.ffi
            .run(Some(action), "".into(), None)
            .unwrap()
            .events
            .is_empty()
    );
    // A guessed handle is unrepresentable: opaque objects have no constructor,
    // fields, integer conversion or JSON decoding entry point in this API.
}

#[test]
fn invalid_json_markers_and_surrogates_are_bounded_and_do_not_mutate_state() {
    let s = Scenario::corpus("runtime-actions", None, "action-steps");
    for raw in [
        "{",
        "[1,",
        "1e400",
        &format!("{}0{}", "[".repeat(1000), "]".repeat(1000)),
    ] {
        let before = s.ffi.snapshot().unwrap();
        assert!(matches!(
            s.ffi.set("$n".into(), Some(raw.into())),
            Err(OpenUiError::InvalidJson { .. })
        ));
        assert!(matches!(
            s.ffi.run_legacy(raw.into(), "".into(), None),
            Err(OpenUiError::InvalidJson { .. })
        ));
        assert!(matches!(
            s.ffi.initialize(Some(raw.into())),
            Err(OpenUiError::InvalidJson { .. })
        ));
        assert!(matches!(
            s.ffi.set_reference_values(raw.into()),
            Err(OpenUiError::InvalidJson { .. })
        ));
        assert_eq!(s.ffi.snapshot().unwrap(), before);
    }
    for marker in [
        "null",
        "{}",
        r#"{"__reactive":"assign","target":3,"expr":1}"#,
    ] {
        let before = s.ffi.snapshot().unwrap();
        assert!(matches!(
            s.ffi.write_binding(marker.into(), "1".into()),
            Err(OpenUiError::InvalidMarker)
        ));
        assert_eq!(s.ffi.snapshot().unwrap(), before);
    }
    // Lone UTF-16 units travel in JSON escapes, as core parse_json specifies.
    for raw in [r#""\ud800""#, r#""\udfff""#, r#""\ud800\udfff""#, "\"�\""] {
        s.ffi.set("$s".into(), Some(raw.into())).unwrap();
        assert_eq!(
            s.ffi.get("$s".into()).unwrap(),
            Some(value_json(&json(raw)))
        );
    }
}

#[test]
fn construction_refuses_invalid_or_non_object_prompt_specs() {
    let library = ComponentLibrary::new(text("schemas/runtime-actions.json")).unwrap();
    for spec in ["{", "[]", "null", r#"{"library":7}"#] {
        assert!(matches!(
            RuntimeSession::new(library.clone(), Some(spec.into())),
            Err(OpenUiError::InvalidJson { .. })
        ));
    }
    let s = RuntimeSession::new(library.clone(), None).unwrap();
    assert!(s.get("$absent".into()).unwrap().is_none());
    assert!(!s.set("$absent".into(), None).unwrap());
    assert!(s.set("$null".into(), Some("null".into())).unwrap());
    assert_eq!(s.get("$null".into()).unwrap().as_deref(), Some("null"));
    assert_eq!(s.snapshot().unwrap(), r#"{"$null":null}"#);
}

#[test]
fn the_default_byte_ceiling_refuses_enormous_host_json_before_parsing() {
    let s = Scenario::corpus("runtime-actions", None, "action-steps");
    let before = s.ffi.snapshot().unwrap();
    let huge = format!("\"{}\"", "x".repeat(8 * 1024 * 1024));
    assert!(matches!(
        s.ffi.set("$huge".into(), Some(huge)),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    assert_eq!(s.ffi.snapshot().unwrap(), before);
    assert!(!s.ffi.is_pristine("$huge".into()).unwrap());
}

#[test]
fn evaluation_flags_match_runtime_at_work_depth_and_room_edges() {
    let schema = text("schemas/runtime-actions.json");
    let library = ComponentLibrary::new(schema.clone()).unwrap();
    let core_library = Library::from_json_schema(&serde_json::from_str(&schema).unwrap());
    let source = "$n=1\nroot=Box(@Each([1,2,3],x,Box(@Set($n,x))),@Go(\"ok\"))";
    let output = parse(source.into(), library.clone(), None);
    let program = openui_lang::parse(source, &core_library, None);
    let schema: serde_json::Value = serde_json::from_str(&schema).unwrap();
    for n in [0, 1, 10, 100, 1000] {
        for (given, limits) in [
            (
                ParseLimits {
                    work: Some(n),
                    ..Default::default()
                },
                openui_lang::Limits::DEFAULT.with_work(n as usize),
            ),
            (
                ParseLimits {
                    result_size: Some(n),
                    ..Default::default()
                },
                openui_lang::Limits::DEFAULT.with_result_size(n as usize),
            ),
            (
                ParseLimits {
                    depth: Some(n),
                    ..Default::default()
                },
                openui_lang::Limits::DEFAULT.with_depth(n as usize),
            ),
        ] {
            let session = RuntimeSession::with_limits(
                library.clone(),
                None,
                RuntimeLimits {
                    evaluation: given,
                    ..Default::default()
                },
            )
            .unwrap();
            session.set_program(output.program.clone()).unwrap();
            let got = session.evaluate().unwrap();
            let want = openui_runtime::evaluate_tree_with_limits(
                TreeInput::new(&program)
                    .with_library(&core_library)
                    .with_actions(&schema["actions"]),
                &Store::new(),
                limits,
            );
            assert_eq!(got.json, want.to_json());
            assert_eq!(
                (got.size_limited, got.depth_limited),
                (want.size_limited, want.depth_limited)
            );
        }
    }
}

#[test]
fn input_limits_refuse_before_effects_and_pending_slots_are_reused() {
    let library = ComponentLibrary::new(text("schemas/runtime-actions.json")).unwrap();
    let limits = RuntimeLimits {
        max_json_bytes: 2048,
        max_continuations: 1,
        ..Default::default()
    };
    let session = RuntimeSession::with_limits(library.clone(), None, limits).unwrap();
    let before = session.snapshot().unwrap();
    assert!(
        session
            .set("a".into(), Some(format!("\"{}\"", "x".repeat(2046))))
            .is_ok()
    );
    let before_large = session.snapshot().unwrap();
    assert!(matches!(
        session.set("a".into(), Some(format!("\"{}\"", "x".repeat(2047)))),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    assert_eq!(session.snapshot().unwrap(), before_large);
    assert_ne!(before, before_large);
    session
        .set_program(
            parse(
                "$n=0\nm=Mutation(\"m\",{})\nroot=Box(Action([@Set($n,1),@Run(m),@Set($n,2)]))"
                    .into(),
                library,
                None,
            )
            .program,
        )
        .unwrap();
    let output = session.evaluate().unwrap();
    let action = session
        .action_at(output.evaluation, path(&["props", "a"]))
        .unwrap();
    let continuation = session
        .run(Some(action.clone()), "".into(), None)
        .unwrap()
        .continuation
        .unwrap();
    let newer = session
        .run(Some(action), "".into(), None)
        .unwrap()
        .continuation
        .unwrap();
    assert!(matches!(
        session.resume(continuation, false),
        Err(OpenUiError::EvictedHandle)
    ));
    session.resume(newer, false).unwrap();
    // Consumed slots must actually be released, not merely marked consumed.
    for _ in 0..4 {
        let tree = session.evaluate().unwrap();
        let action = session
            .action_at(tree.evaluation, path(&["props", "a"]))
            .unwrap();
        let continuation = session
            .run(Some(action), "".into(), None)
            .unwrap()
            .continuation
            .unwrap();
        session.resume(continuation, false).unwrap();
    }
}

#[test]
fn program_admission_refuses_before_revoking_the_preceding_evaluation() {
    let library = ComponentLibrary::new(text("schemas/runtime-actions.json")).unwrap();
    let session = RuntimeSession::with_limits(
        library.clone(),
        None,
        RuntimeLimits {
            max_json_bytes: 2048,
            ..Default::default()
        },
    )
    .unwrap();
    session
        .set_program(parse("root=Box(@Set($n,1))".into(), library.clone(), None).program)
        .unwrap();
    let tree = session.evaluate().unwrap();
    let action = session
        .action_at(tree.evaluation, path(&["props", "a"]))
        .unwrap();
    let before = session.snapshot().unwrap();
    let big = parse(format!("root=Box(\"{}\")", "x".repeat(3000)), library, None);
    assert!(matches!(
        session.set_program(big.program),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    assert_eq!(session.snapshot().unwrap(), before);
    assert!(
        session
            .run(Some(action), "".into(), None)
            .unwrap()
            .failure
            .is_none(),
        "a refused program must not revoke old plans"
    );
}

#[test]
fn cuts_in_actions_binding_writes_and_forms_keep_the_runtime_result() {
    let library = ComponentLibrary::new(text("schemas/runtime-actions.json")).unwrap();
    let schema: serde_json::Value =
        serde_json::from_str(&text("schemas/runtime-actions.json")).unwrap();
    let core_library = Library::from_json_schema(&schema);
    let source = "$n=0\nroot=Box(@Set($n,1))";
    let output = parse(source.into(), library.clone(), None);
    let core_program = openui_lang::parse(source, &core_library, None);
    let session = RuntimeSession::with_limits(
        library,
        None,
        RuntimeLimits {
            evaluation: ParseLimits {
                work: Some(0),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    session.set_program(output.program).unwrap();
    let before = session.snapshot().unwrap();
    let got = session.run(None, "message".into(), None).unwrap();
    let want = openui_runtime::run_action_with_limits(
        TreeInput::new(&core_program).with_library(&core_library),
        &mut Store::new(),
        None,
        "message",
        None,
        openui_lang::Limits::DEFAULT.with_work(0),
    );
    compare_run(&got, &want);
    let marker = r#"{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":"$value"}}"#;
    assert!(matches!(
        session.write_binding(marker.into(), "1".into()),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    assert!(matches!(
        session.set_field(None, None, "name".into(), Some("1".into()), false),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    assert_eq!(session.snapshot().unwrap(), before);
}

#[test]
fn full_pending_tables_allow_completed_runs_and_evict_only_on_new_suspension() {
    let library = ComponentLibrary::new(text("schemas/runtime-actions.json")).unwrap();
    let session = RuntimeSession::with_limits(
        library.clone(),
        None,
        RuntimeLimits {
            max_continuations: 1,
            ..Default::default()
        },
    )
    .unwrap();
    session
        .set_program(
            parse(
                "m=Mutation(\"m\",{})\nroot=Box(@Run(m),@Set($n,1))".into(),
                library,
                None,
            )
            .program,
        )
        .unwrap();
    let tree = session.evaluate().unwrap();
    let suspended = session
        .action_at(tree.evaluation.clone(), path(&["props", "a"]))
        .unwrap();
    let completed = session
        .action_at(tree.evaluation, path(&["props", "b"]))
        .unwrap();
    let first = session
        .run(Some(suspended.clone()), "".into(), None)
        .unwrap()
        .continuation
        .unwrap();
    assert!(
        session
            .run(Some(completed), "".into(), None)
            .unwrap()
            .failure
            .is_none()
    );
    assert_eq!(
        session
            .run(None, "hello".into(), None)
            .unwrap()
            .events
            .len(),
        1
    );
    assert_eq!(
        session
            .run_legacy(
                r#"{"type":"continue_conversation"}"#.into(),
                "".into(),
                None
            )
            .unwrap()
            .events
            .len(),
        1
    );
    assert_eq!(session.pending_continuations().unwrap(), 1);
    let second = session
        .run(Some(suspended), "".into(), None)
        .unwrap()
        .continuation
        .unwrap();
    let error = session.resume(first, true).unwrap_err();
    assert!(matches!(error, OpenUiError::EvictedHandle));
    session.resume(second, false).unwrap();
    assert_eq!(session.pending_continuations().unwrap(), 0);
}

#[test]
fn a_new_suspension_evicts_the_oldest_outcome_and_keeps_newer_outcomes_live() {
    let library = ComponentLibrary::new(text("schemas/runtime-actions.json")).unwrap();
    let session = RuntimeSession::with_limits(
        library.clone(),
        None,
        RuntimeLimits {
            max_continuations: 2,
            ..Default::default()
        },
    )
    .unwrap();
    session
        .set_program(
            parse(
                "m=Mutation(\"m\",{})\nroot=Box(@Run(m))".into(),
                library,
                None,
            )
            .program,
        )
        .unwrap();
    let tree = session.evaluate().unwrap();
    let action = session
        .action_at(tree.evaluation, path(&["props", "a"]))
        .unwrap();
    let mut pending = Vec::new();
    for _ in 0..3 {
        pending.push(
            session
                .run(Some(action.clone()), "".into(), None)
                .unwrap()
                .continuation
                .unwrap(),
        );
    }
    assert_eq!(session.pending_continuations().unwrap(), 2);
    assert!(matches!(
        session.resume(pending[0].clone(), true),
        Err(OpenUiError::EvictedHandle)
    ));
    session.resume(pending[1].clone(), false).unwrap();
    session.resume(pending[2].clone(), false).unwrap();
    assert_eq!(session.pending_continuations().unwrap(), 0);
}

#[test]
fn expression_failures_are_distinct_from_refused_store_writes() {
    let library = ComponentLibrary::new(text("schemas/runtime-actions.json")).unwrap();
    let session = RuntimeSession::new(library, None).unwrap();
    let marker =
        r#"{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":{"toString":3}}}"#;
    let before = session.snapshot().unwrap();
    assert!(matches!(
        session.write_binding(marker.into(), "1".into()),
        Err(OpenUiError::Evaluation { .. })
    ));
    assert_eq!(session.snapshot().unwrap(), before);
}

#[test]
fn equal_json_after_a_stream_restart_preserves_fresh_default_identities() {
    let schema = r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#;
    let library = ComponentLibrary::new(schema.into()).unwrap();
    let stream = StreamSession::new(library.clone(), None);
    let session = RuntimeSession::new(library, None).unwrap();
    let core_library = Arc::new(Library::from_json_schema(
        &openui_lang::parse_json(schema).unwrap(),
    ));
    let mut core_stream = openui_lang::StreamParser::new(core_library, None);
    let mut store = Store::new();
    let mut jsons = Vec::new();
    let mut versions = Vec::new();
    for source in [
        "$s=[[1]]\nroot=Box($s)",
        "$s = [[1]]\nroot=Box($s)",
        "$s = [[1]]\nroot=Box($s)",
    ] {
        let output = stream.set(source.into());
        let core = core_stream.set(source);
        assert_eq!(output.json, core.to_json());
        jsons.push(output.json);
        session.set_program(output.program).unwrap();
        let initialized = session.initialize(None).unwrap();
        let expected = store.initialize(
            core.state_declarations
                .iter()
                .map(|(k, v)| (k.as_str(), Some(v))),
            [],
        );
        assert_eq!(initialized.changed, expected.changed);
        assert_eq!(session.version().unwrap(), store.version());
        versions.push(session.version().unwrap());
    }
    assert_eq!(jsons[0], jsons[1]);
    assert_eq!(versions, vec![1, 2, 2]);
}

#[test]
fn state_value_admission_refuses_before_changing_value_or_pristine_status() {
    let library = ComponentLibrary::new(text("schemas/runtime-actions.json")).unwrap();
    let session = RuntimeSession::with_limits(
        library,
        None,
        RuntimeLimits {
            evaluation: ParseLimits {
                result_size: Some(64),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    let before = session.snapshot().unwrap();
    assert!(matches!(
        session.set("$x".into(), Some(format!("\"{}\"", "x".repeat(65)))),
        Err(OpenUiError::RefusedWrite { .. })
    ));
    assert_eq!(session.snapshot().unwrap(), before);
}

#[test]
fn total_state_admission_refuses_before_changing_state() {
    let library = ComponentLibrary::new(text("schemas/runtime-actions.json")).unwrap();
    let session = RuntimeSession::with_limits(
        library,
        None,
        RuntimeLimits {
            evaluation: ParseLimits {
                result_size: Some(64),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    let mut refused = false;
    for i in 0..100 {
        let before = session.snapshot().unwrap();
        if let Err(error) = session.set(format!("$x{i}"), Some("123".into())) {
            assert!(matches!(error, OpenUiError::RefusedWrite { .. }));
            assert_eq!(session.snapshot().unwrap(), before);
            refused = true;
            break;
        }
    }
    assert!(refused, "total state was never refused");
}
