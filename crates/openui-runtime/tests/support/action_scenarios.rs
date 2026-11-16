//! Replays renderer interaction goldens through the public runtime interface.
use super::*;
use openui_runtime::{
    ActionContinuation, get_field_value, initialize_field_default, resume_action, run_action,
    set_field_value,
};

fn at_path(mut value: Value, path: &Value) -> Value {
    for key in arr(path) {
        let next = match &value {
            Value::Element(e) => match text(key) {
                "props" => Value::Obj(Arc::new(e.props.clone())),
                _ => get(&value, text(key)).clone(),
            },
            Value::Arr(a) => text(key)
                .parse::<usize>()
                .ok()
                .and_then(|i| a.get(i))
                .cloned()
                .unwrap_or(Value::Null),
            _ => get(&value, text(key)).clone(),
        };
        value = next;
    }
    value
}
fn snapshot_value(store: &Store) -> Value {
    let mut out = JsObject::new();
    for (key, value) in store.snapshot().iter() {
        if let Some(v) = value {
            out.define(key, v.clone());
        }
    }
    Value::Obj(Arc::new(out))
}
fn discover(v: &Value, out: &mut usize) {
    if matches!(get(v, "steps"), Value::Arr(_)) {
        *out += 1;
    }
    match v {
        Value::Element(e) => {
            for (_, v) in e.props.iter() {
                discover(v, out);
            }
        }
        Value::Obj(o) => {
            for (_, v) in o.iter() {
                discover(v, out);
            }
        }
        Value::Arr(a) => {
            for v in a.iter() {
                discover(v, out);
            }
        }
        _ => {}
    }
}

#[test]
fn corpus_action_scenarios_match_the_renderer() {
    let dir = root_dir();
    let golden = dir.join("golden/runtime");
    let manifests: Vec<_> = [
        "manifest",
        "imported",
        "fuzz",
        "synthetic",
        "recorded",
        "runtime",
    ]
    .iter()
    .map(|s| load(&dir.join(format!("{s}.json"))))
    .collect();
    let mut schemas = BTreeMap::new();
    let mut cases = Vec::new();
    for m in &manifests {
        for (k, v) in entries(get(m, "schemas")) {
            schemas.insert(k.as_str(), text(v));
        }
        cases.extend(arr(get(m, "cases")));
    }
    let bindings_manifest = load(&dir.join("bindings.json"));
    let mut count_cases = 0;
    let mut count_scenarios = 0;
    let mut count_steps = 0;
    let mut count_plans = 0;
    let mut divergent_cases = 0;
    let mut divergent_steps = 0;
    let mut original_cases = 0;
    let mut failures = String::new();
    let dumping = std::env::var_os("ACTION_DUMP").is_some();
    for case in &cases {
        let name = text(get(case, "name"));
        if std::env::var("RUNTIME_CASE").is_ok_and(|filter| !name.contains(&filter)) {
            continue;
        }
        let path = golden.join(format!("{name}.actions.json"));
        let one = load(&golden.join(format!("{name}.oneshot.json")));
        let schema_name = text(get(case, "schema"));
        let schema = openui_lang::parse_json(
            &std::fs::read_to_string(dir.join(schemas[schema_name])).unwrap(),
        )
        .unwrap();
        let library = Library::from_json_schema(&schema);
        let source = std::fs::read_to_string(dir.join(text(get(case, "file")))).unwrap();
        let mut parsed = parse(&source, &library, nonempty(text(get(case, "root"))));
        if matches!(get(case, "queryComplete"), Value::Bool(false)) {
            for q in &mut parsed.query_statements {
                q.complete = false;
            }
        }
        let bindings = if let Value::Str(p) = get(&bindings_manifest, schema_name) {
            BindingProps::from_prompt_spec(get(&load(&dir.join(p)), "library")).unwrap()
        } else {
            BindingProps::new()
        };
        let expected_fixture = path.exists().then(|| load(&path));
        let pin = match name {
            name if name.starts_with("runtime/action-target-") => {
                let path = dir.join(format!(
                    "divergences/{}-runs.json",
                    name.replace("runtime/", "runtime-")
                ));
                if dumping && !path.exists() {
                    None
                } else {
                    Some(load(&path))
                }
            }
            _ => None,
        };
        if let Some(pin) = &pin {
            assert_eq!(text(get(pin, "case")), name);
            assert_eq!(text(get(pin, "reason")), "non-string-action-target");
            assert_eq!(
                json(get(pin, "reference")),
                json(expected_fixture.as_ref().unwrap()),
                "{name}: pinned reference interactions changed"
            );
        }
        let mut case_diverged = false;
        let mut case_dump = JsObject::new();
        let mut any = false;
        for scenario in ["defaults", "persisted"] {
            let mut result = parsed.clone();
            let mut current_case = if let Value::Obj(o) = case {
                o.as_ref().clone()
            } else {
                JsObject::new()
            };
            let mut store = Store::new();
            let persistence = if scenario == "persisted" {
                get(&one, "saved")
            } else {
                &NULL
            };
            let (_, root) = observe_retained(
                &result,
                &library,
                &mut store,
                persistence,
                &source,
                case,
                &bindings,
                &schema,
            );
            let initial = root.unwrap_or(Value::Null);
            let mut found = 0;
            discover(&initial, &mut found);
            discover(&snapshot_value(&store), &mut found);
            let expected = expected_fixture
                .as_ref()
                .map(|v| get(v, scenario))
                .unwrap_or(&NULL);
            assert_eq!(
                found,
                arr(get(expected, "handles")).len(),
                "{name}/{scenario}: missing generated plans"
            );
            if expected == &NULL {
                assert_eq!(
                    get(case, "actionInteractions"),
                    &NULL,
                    "missing explicit interactions"
                );
                continue;
            }
            any = true;
            count_scenarios += 1;
            count_plans += found;
            let handles: Vec<_> = arr(get(expected, "handles"))
                .iter()
                .map(|h| {
                    at_path(
                        if text(get(h, "source")) == "tree" {
                            initial.clone()
                        } else {
                            snapshot_value(&store)
                        },
                        get(h, "path"),
                    )
                })
                .collect();
            for h in &handles {
                assert!(
                    matches!(get(h, "steps"), Value::Arr(_)),
                    "{name}: path does not identify a plan"
                );
            }
            if pin.is_some() {
                assert_eq!(handles.len(), 1, "target pins have exactly one invalid row");
                let steps = arr(get(expected, "steps"));
                assert_eq!(
                    steps.len(),
                    2,
                    "target pins stop after one run and disposal"
                );
                assert_eq!(text(get(get(&steps[0], "op"), "kind")), "run");
                assert_eq!(text(get(get(&steps[1], "op"), "kind")), "dispose");
                assert!(
                    handles.iter().flat_map(|p| arr(get(p, "steps"))).any(|s| {
                        (text(get(s, "type")) == "set"
                            && !matches!(get(s, "target"), Value::Str(_)))
                            || (text(get(s, "type")) == "reset"
                                && arr(get(s, "targets"))
                                    .iter()
                                    .any(|t| !matches!(t, Value::Str(_))))
                    }),
                    "{name}: non-string-target divergence needs an independent witness"
                );
            }
            let mut continuations: BTreeMap<String, ActionContinuation> = BTreeMap::new();
            let mut used = BTreeSet::new();
            let mut known: BTreeSet<String> =
                store.snapshot().iter().map(|(k, _)| k.clone()).collect();
            let mut actual_steps = Vec::new();
            for (index, step) in arr(get(expected, "steps")).iter().enumerate() {
                count_steps += 1;
                let op = get(step, "op");
                let kind = text(get(op, "kind"));
                if kind == "run" {
                    used.insert(number(get(op, "handle")));
                }
                let counts = Arc::new(std::sync::Mutex::new(BTreeMap::new()));
                let empty = openui_lang::JsonValue::Null;
                let functions = functions::counted_registry(
                    schema.get("functions").unwrap_or(&empty),
                    Some(counts.clone()),
                );
                let mut input = TreeInput::new(&result)
                    .with_library(&library)
                    .with_binding_props(&bindings)
                    .with_functions(&functions);
                if let Some(actions) = schema.get("actions") {
                    input = input.with_actions(actions);
                }
                if let Value::Obj(values) = current_case.get("referenceValues").unwrap_or(&NULL) {
                    input = input.with_reference_values(values);
                }
                let form = match get(op, "formName") {
                    Value::Str(s) => Some(s.as_str()),
                    _ => None,
                };
                let mut events = Vec::new();
                let mut failed = false;
                let mut suspended = None;
                let mut save_requested = false;
                let mut field_value = None;
                match kind {
                    "set" => {
                        failed = store
                            .set(text(get(op, "name")), Some(get(op, "value").clone()))
                            .is_err();
                    }
                    "run" | "runState" | "legacy" | "none" => {
                        let state_plan = store.get("$retained");
                        let action = match kind {
                            "run" => handles.get(number(get(op, "handle"))),
                            "runState" => state_plan.as_ref(),
                            "legacy" => Some(get(op, "action")),
                            _ => None,
                        };
                        let r =
                            run_action(input, &mut store, action, text(get(op, "message")), form);
                        failed = r.failure.is_some();
                        suspended = Some(r.is_suspended());
                        if let Some(c) = r.continuation {
                            let slot = match get(op, "slot") {
                                Value::Str(s) => s.as_str(),
                                _ => "current",
                            };
                            continuations.insert(slot.to_owned(), c);
                        }
                        events = r.events.into_iter().map(|e| e.value().clone()).collect();
                        if let Some(action) = action {
                            for step in arr(get(action, "steps")) {
                                if let Value::Str(target) = get(step, "target") {
                                    known.insert(target.clone());
                                }
                                for target in arr(get(step, "targets")) {
                                    if let Value::Str(t) = target {
                                        known.insert(t.clone());
                                    }
                                }
                            }
                        }
                    }
                    "resume" => {
                        let slot = match get(op, "slot") {
                            Value::Str(s) => s.as_str(),
                            _ => "current",
                        };
                        let into = match get(op, "into") {
                            Value::Str(s) => s.as_str(),
                            _ => slot,
                        };
                        let token = continuations.get(slot).cloned();
                        let r = token.as_ref().map(|c| {
                            resume_action(
                                input,
                                &mut store,
                                c,
                                matches!(get(op, "success"), Value::Bool(true)),
                            )
                        });
                        match r {
                            Some(Ok(r)) => {
                                failed = r.failure.is_some();
                                suspended = Some(r.is_suspended());
                                if let Some(c) = r.continuation {
                                    continuations.insert(into.to_owned(), c);
                                }
                                events = r.events.into_iter().map(|e| e.value().clone()).collect();
                            }
                            _ => {
                                failed = true;
                                suspended = Some(false);
                            }
                        }
                    }
                    "field" | "default" => {
                        let name = text(get(op, "name"));
                        let component = if let Value::Str(s) = get(op, "componentType") {
                            Some(s.as_str())
                        } else {
                            None
                        };
                        let value = if let Value::Obj(o) = op {
                            o.get("value").cloned()
                        } else {
                            None
                        };
                        if kind == "default" {
                            match initialize_field_default(
                                &mut store,
                                form,
                                component,
                                name,
                                value,
                                matches!(get(op, "streaming"), Value::Bool(true)),
                            ) {
                                Ok(Some(r)) => save_requested = r.save_requested,
                                Ok(None) => {}
                                Err(_) => failed = true,
                            }
                        } else {
                            match set_field_value(
                                &mut store,
                                form,
                                component,
                                name,
                                value,
                                !matches!(get(op, "save"), Value::Bool(false)),
                            ) {
                                Ok(r) => save_requested = r.save_requested,
                                Err(_) => failed = true,
                            }
                        }
                        field_value = get_field_value(&store, form, name);
                    }
                    "binding" => {
                        failed = write_binding(
                            input,
                            &mut store,
                            get(op, "marker"),
                            get(op, "value").clone(),
                        )
                        .is_err()
                    }
                    "retain" => {
                        let marker = from_json(
                            r#"{"__reactive":"assign","target":"$retained","expr":{"k":"StateRef","n":"$value"}}"#,
                        );
                        failed = write_binding(
                            input,
                            &mut store,
                            &marker,
                            handles[number(get(op, "handle"))].clone(),
                        )
                        .is_err();
                    }
                    "program" => {
                        result = parse(
                            text(get(op, "program")),
                            &library,
                            nonempty(text(get(case, "root"))),
                        );
                        assert!(
                            store
                                .initialize(
                                    result
                                        .state_declarations
                                        .iter()
                                        .map(|(k, v)| (k.as_str(), Some(v))),
                                    []
                                )
                                .refused
                                .is_empty()
                        );
                    }
                    "references" => {
                        current_case.define("referenceValues", get(op, "values").clone());
                    }
                    "dispose" => {
                        store.dispose();
                    }
                    "redraw" => {}
                    _ => panic!("unknown action interaction {kind}"),
                }
                known.extend(store.snapshot().iter().map(|(k, _)| k.clone()));
                let redraw = from_json(&evaluated_observation(
                    &result,
                    &library,
                    &store,
                    &Value::Obj(Arc::new(current_case.clone())),
                    &bindings,
                    &schema,
                ));
                let mut actual = JsObject::new();
                actual.define("events", Value::Arr(Arc::new(events)));
                actual.define("failed", Value::Bool(failed));
                if let Some(suspended) = suspended {
                    actual.define("suspended", Value::Bool(suspended));
                }
                if save_requested {
                    actual.define("saveRequested", Value::Bool(true));
                }
                if let Some(value) = field_value {
                    actual.define("fieldValue", value);
                }
                let mut pristine: Vec<_> = known.iter().filter(|n| store.is_pristine(n)).collect();
                pristine
                    .sort_by(|a, b| openui_lang::utf16_units(a).cmp(openui_lang::utf16_units(b)));
                actual.define(
                    "pristine",
                    Value::Arr(Arc::new(
                        pristine
                            .into_iter()
                            .map(|n| Value::Str(n.clone()))
                            .collect(),
                    )),
                );
                for (k, v) in entries(&redraw) {
                    actual.define(k, v.clone());
                }
                if let Some(definitions) = schema
                    .get("functions")
                    .and_then(openui_lang::JsonValue::as_object)
                {
                    let mut c = JsObject::new();
                    for name in definitions.keys() {
                        c.define(
                            name,
                            Value::Num(*counts.lock().unwrap().get(name).unwrap_or(&0) as f64),
                        );
                    }
                    actual.define("actionCallCounts", Value::Obj(Arc::new(c)));
                }
                let actual = Value::Obj(Arc::new(actual));
                let expected_observation = expand(
                    get(step, "observation"),
                    get(expected_fixture.as_ref().unwrap(), "values"),
                );
                let different = json(&actual) != json(&expected_observation);
                if different {
                    case_diverged = true;
                    divergent_steps += 1;
                }
                let pinned = pin.as_ref().filter(|_| index == 0).map(|pin| {
                    let rust = get(pin, "rust");
                    let step = &arr(get(get(rust, scenario), "steps"))[index];
                    assert_eq!(json(get(step, "op")), json(op));
                    expand(get(step, "observation"), get(rust, "values"))
                });
                let required = pinned.as_ref().unwrap_or(&expected_observation);
                if json(&actual) != json(required) && failures.len() < 10_000 {
                    let _ = writeln!(
                        failures,
                        "{name}/{scenario} action step {} ({kind}):\n{}",
                        index + 1,
                        diff(&json(required), &json(&actual))
                    );
                }
                if dumping {
                    let mut output = JsObject::new();
                    output.define("op", op.clone());
                    output.define("observation", actual);
                    actual_steps.push(Value::Obj(Arc::new(output)));
                }
            }
            assert_eq!(
                used,
                (0..handles.len()).collect(),
                "{name}/{scenario}: every retained plan must run"
            );
            let mut output = JsObject::new();
            output.define("handles", get(expected, "handles").clone());
            output.define("steps", Value::Arr(Arc::new(actual_steps)));
            case_dump.define(scenario, Value::Obj(Arc::new(output)));
        }
        if any {
            count_cases += 1;
            original_cases += usize::from(!name.starts_with("runtime/"));
            if pin.is_some() {
                assert!(case_diverged, "{name}: remove matching action divergence");
                divergent_cases += 1;
            }
            if dumping {
                let path = dir
                    .join("../target/action-observations")
                    .join(format!("{name}.json"));
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, json(&Value::Obj(Arc::new(case_dump)))).unwrap();
            }
        }
        free_json(schema);
    }
    let line = format!(
        "Runtime action runs: {count_cases} cases ({} matching, {divergent_cases} divergences), {count_scenarios} scenarios, {count_steps} steps ({} matching, {divergent_steps} pinned), {count_plans} retained plans.",
        count_cases - divergent_cases,
        count_steps - divergent_steps
    );
    println!("{line}");
    assert!(failures.is_empty(), "{failures}");
    if std::env::var_os("RUNTIME_CASE").is_none() {
        let coverage = load(&golden.join("action-coverage.json"));
        for (key, count) in [
            ("cases", count_cases),
            ("scenarios", count_scenarios),
            ("steps", count_steps),
            ("plans", count_plans),
            ("originalCases", original_cases),
        ] {
            assert_eq!(number(get(&coverage, key)), count, "action coverage {key}");
        }
        let doc = std::fs::read_to_string(dir.join("../CONFORMANCE.md")).unwrap();
        assert!(doc.contains(&line), "CONFORMANCE.md needs: {line}");
    }
    assert!(failures.is_empty(), "{failures}");
}
fn nonempty(s: &str) -> Option<&str> {
    (!s.is_empty()).then_some(s)
}

fn expand(observation: &Value, values: &Value) -> Value {
    let mut out = JsObject::new();
    for (k, v) in entries(observation) {
        let value = if let Value::Num(n) = get(v, "$value") {
            arr(values)[*n as usize].clone()
        } else {
            v.clone()
        };
        out.define(k, value);
    }
    Value::Obj(Arc::new(out))
}
