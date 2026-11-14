//! Every parse corpus case, both state scenarios, and every core stream plan.
use openui_lang::{Element, Library, ParseResult, StreamParser, free_json, parse, parse_json};
use openui_runtime::{
    BindingProps, BindingWriteError, JsObject, Store, TreeInput, Value, evaluate_tree,
    write_binding,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
static NULL: Value = Value::Null;
fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    if let Value::Obj(o) = v {
        o.get(k).unwrap_or(&NULL)
    } else {
        &NULL
    }
}
fn text(v: &Value) -> &str {
    if let Value::Str(s) = v { s } else { "" }
}
fn arr(v: &Value) -> &[Value] {
    if let Value::Arr(a) = v { a } else { &[] }
}
// Canonical RLE, borrowed hashes: no repeated strings in the fixture.
fn fingerprints(v: &Value) -> Vec<&str> {
    let mut out = Vec::new();
    let mut previous = "";
    for run in arr(v) {
        let (hash, count) = match run {
            Value::Str(s) => (s.as_str(), 1),
            Value::Arr(pair) => {
                assert_eq!(pair.len(), 2, "invalid fingerprint run");
                let count = number(&pair[1]);
                assert!((2..=4_000_000).contains(&count));
                assert!(matches!(&pair[1],Value::Num(n) if n.fract()==0.0));
                (text(&pair[0]), count)
            }
            _ => panic!("invalid fingerprint run"),
        };
        assert!(hash.len() == 8 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(hash, previous, "adjacent runs must be combined");
        previous = hash;
        assert!(out.len().saturating_add(count) <= 4_000_000);
        out.extend(std::iter::repeat_n(hash, count));
    }
    out
}
fn entries(v: &Value) -> Vec<(&String, &Value)> {
    if let Value::Obj(o) = v {
        o.iter().collect()
    } else {
        vec![]
    }
}
fn number(v: &Value) -> usize {
    if let Value::Num(n) = v {
        *n as usize
    } else {
        0
    }
}
fn load(p: &Path) -> Value {
    let j = parse_json(&std::fs::read_to_string(p).unwrap()).unwrap();
    let v = Value::from_json(&j);
    free_json(j);
    v
}
fn json(v: &Value) -> String {
    let mut props = JsObject::new();
    props.define("v", v.clone());
    let p = ParseResult {
        root: Some(Element {
            type_name: String::new(),
            props,
            partial: false,
            has_dynamic_props: false,
            statement_id: None,
        }),
        ..ParseResult::default()
    }
    .to_json();
    let s = p
        .strip_prefix("{\"root\":{\"type\":\"element\",\"typeName\":\"\",\"props\":{\"v\":")
        .unwrap();
    s[..s
        .rfind("},\"partial\":false,\"hasDynamicProps\":false},\"meta\":")
        .unwrap()]
        .to_owned()
}
fn fp(s: &str) -> String {
    let mut h = 0x811c9dc5u32;
    for b in s.bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(0x01000193);
    }
    format!("{h:08x}")
}
#[allow(clippy::too_many_arguments)]
fn observe(
    result: &ParseResult,
    library: &Library,
    store: &mut Store,
    saved: &Value,
    source: &str,
    case_input: &Value,
    bindings: &BindingProps,
    actions: Option<&openui_lang::JsonValue>,
) -> String {
    let p = entries(saved);
    // JSON omits top-level undefined declarations. Preserve their *presence*
    // for initialize using public statement stages, without changing the core.
    let (closed, _) = openui_lang::auto_close(&openui_lang::preprocess(source));
    let statements = if source.contains('$') {
        openui_lang::split(&openui_lang::tokenize(&closed))
    } else {
        Vec::new()
    };
    let runtime_names: BTreeSet<_> = result
        .query_statements
        .iter()
        .map(|q| q.statement_id.as_str())
        .chain(
            result
                .mutation_statements
                .iter()
                .map(|m| m.statement_id.as_str()),
        )
        .collect();
    let mut defaults = JsObject::new();
    for statement in &statements {
        if statement.id.starts_with('$') && !runtime_names.contains(statement.id.as_str()) {
            defaults.define(
                statement.id.clone(),
                result.state_declarations.get(&statement.id),
            );
        }
    }
    for (k, v) in result.state_declarations.iter() {
        defaults.define(k.clone(), Some(v));
    }
    let initialized = store.initialize(
        defaults.iter().map(|(k, v)| (k.as_str(), *v)),
        p.iter().map(|(k, v)| (k.as_str(), Some(*v))).chain(
            entries(get(case_input, "stateValues"))
                .iter()
                .map(|(k, v)| (k.as_str(), Some(*v))),
        ),
    );
    assert!(initialized.refused.is_empty());
    evaluated_observation(result, library, store, case_input, bindings, actions)
}
fn evaluated_observation(
    result: &ParseResult,
    library: &Library,
    store: &Store,
    case_input: &Value,
    bindings: &BindingProps,
    actions: Option<&openui_lang::JsonValue>,
) -> String {
    let mut completion_override;
    let result = if matches!(get(case_input, "queryComplete"), Value::Bool(false)) {
        completion_override = result.clone();
        for q in &mut completion_override.query_statements {
            q.complete = false;
        }
        &completion_override
    } else {
        result
    };

    let mut input = TreeInput::new(result)
        .with_library(library)
        .with_binding_props(bindings);
    if let Some(actions) = actions {
        input = input.with_actions(actions);
    }
    if let Value::Obj(values) = get(case_input, "referenceValues") {
        input = input.with_reference_values(values);
    }
    let evaluated = evaluate_tree(input, store);
    assert!(
        !evaluated.size_limited && !evaluated.depth_limited,
        "corpus evaluation cut"
    );
    let mut j = evaluated.to_json();
    j.pop();
    j.push_str(",\"snapshot\":");
    let mut snapshot = JsObject::new();
    for (k, v) in store.snapshot().iter() {
        if let Some(v) = v {
            snapshot.define(k.clone(), v.clone());
        }
    }
    j.push_str(&json(&Value::Obj(Arc::new(snapshot))));
    j.push('}');
    j
}
fn diff(expected: &str, actual: &str) -> String {
    let n = expected
        .bytes()
        .zip(actual.bytes())
        .position(|(a, b)| a != b)
        .unwrap_or(expected.len().min(actual.len()));
    format!("first difference at byte {n}\n- reference: {expected}\n+ runtime:   {actual}\n")
}
fn root_dir() -> PathBuf {
    std::env::var_os("OPENUI_CONFORMANCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance"))
}
fn live(case: &str, scenario: &str, chunking: &str, step: usize) -> String {
    if std::env::var_os("OPENUI_NO_LIVE_DIFFS").is_some() {
        return format!(
            "run (cd reference && bun --no-install runtime.ts show {case} {scenario} {chunking} {step})"
        );
    }
    std::process::Command::new("bun")
        .args([
            "--no-install",
            "runtime.ts",
            "show",
            case,
            scenario,
            chunking,
            &step.to_string(),
        ])
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}
#[test]
fn corpus_trees_snapshots_and_json_match_the_reference() {
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
    let binding_sources = load(&dir.join("bindings.json"));
    let binding_oracle = load(&golden.join("bindings.json"));
    let pending_path = dir.join("pending/runtime.json");
    let pending = load(&pending_path);
    let divergences = load(&dir.join("pending/runtime-divergences.json"));
    let coverage = load(&golden.join("coverage.json"));
    let evidence = load(&golden.join("evidence.json"));
    let mut failing = BTreeSet::new();
    let mut report = String::new();
    let mut recorded = 0usize;
    let mut recorded_state = 0usize;
    let mut streamed = 0usize;
    let mut scenario_differences = 0usize;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut witnessed = BTreeMap::new();
    let mut write_cases = 0usize;
    let mut write_steps = 0usize;
    let mut partial_observations = 0usize;
    let mut partial_props = 0usize;
    for c in &cases {
        let name = text(get(c, "name"));
        if std::env::var("RUNTIME_CASE").is_ok_and(|filter| !name.contains(&filter)) {
            continue;
        }
        let input = std::fs::read_to_string(dir.join(text(get(c, "file")))).unwrap();
        let schema = parse_json(
            &std::fs::read_to_string(dir.join(schemas[text(get(c, "schema"))])).unwrap(),
        )
        .unwrap();
        let library = Arc::new(Library::from_json_schema(&schema));
        let spec_path = text(get(&binding_sources, text(get(c, "schema"))));
        let bindings = if spec_path.is_empty() {
            BindingProps::new()
        } else {
            BindingProps::from_prompt_spec(get(&load(&dir.join(spec_path)), "library"))
        };
        assert_eq!(
            bindings.undecided().collect::<Vec<_>>(),
            Vec::<(&str, &str)>::new(),
            "{name}: binding configuration must be complete"
        );
        let oracle = arr(get(&binding_oracle, text(get(c, "schema"))));
        for component in library.component_names() {
            let definition = library.component(component).unwrap();
            for prop in &definition.params {
                assert_eq!(
                    bindings.contains(component, &prop.name),
                    oracle
                        .iter()
                        .any(|p| text(&arr(p)[0]) == component && text(&arr(p)[1]) == prop.name),
                    "binding mark {component}.{}",
                    prop.name
                );
            }
        }
        let root = match text(get(c, "root")) {
            "" => None,
            s => Some(s),
        };
        let one = load(&golden.join(format!("{name}.oneshot.json")));
        let streams = load(&golden.join(format!("{name}.steps.json")));
        let saved = get(&one, "saved");
        let core_path = dir.join("golden").join(format!("{name}.steps.json"));
        let core_streams = core_path.exists().then(|| load(&core_path));
        assert!(
            core_streams.is_some() || name.starts_with("runtime/"),
            "{name}: missing core chunk plans"
        );
        if text(get(c, "source")) == "recorded" {
            recorded += 1;
            if !entries(saved).is_empty() {
                recorded_state += 1;
            }
        }
        let mut features = BTreeSet::new();
        let parsed_full = parse(&input, &library, root);
        features_in(&parsed_full, &library, &mut features);
        witnessed.insert(name.to_owned(), features);
        let mut mismatches = String::new();
        let listed = entries(&pending)
            .iter()
            .chain(entries(&divergences).iter())
            .any(|(k, _)| k.as_str() == name);
        let expected_chunkings = if matches!(get(c, "chunkings"), Value::Arr(_)) {
            arr(get(c, "chunkings"))
        } else {
            arr(get(&manifests[0], "chunkings"))
        };
        let mut names: BTreeSet<_> = expected_chunkings.iter().map(text).collect();
        if matches!(get(c, "recorded"), Value::Arr(_)) {
            names.insert("recorded");
        }
        assert_eq!(
            names,
            entries(&streams).iter().map(|(k, _)| k.as_str()).collect(),
            "{name}: missing/extra chunking"
        );
        for (k, stream) in entries(&streams) {
            let defaults = fingerprints(get(stream, "defaults"));
            let persisted = if text(get(stream, "persisted")) == "defaults" {
                defaults.clone()
            } else {
                fingerprints(get(stream, "persisted"))
            };
            assert_eq!(
                defaults.len(),
                persisted.len(),
                "{name}/{k}: scenario step counts"
            );
            streamed += defaults.len();
            scenario_differences += defaults
                .iter()
                .zip(&persisted)
                .filter(|(a, b)| a != b)
                .count();
            if let Some(core) = &core_streams {
                let core = get(core, k);
                assert_eq!(
                    arr(get(core, "steps")).len(),
                    defaults.len(),
                    "{name}/{k}: core plan step count"
                );
                for field in ["sizes", "ops"] {
                    assert!(
                        matches!(get(stream, field), Value::Null),
                        "{name}/{k}: duplicate {field}"
                    );
                }
            }
        }

        let divergence = text(get(&divergences, name));
        match divergence {
            "" => {}
            "nested-undefined" => {
                assert!(input.contains("toString: $missing.x"));
                assert!(json(get(get(&one, "defaults"), "errors")).contains("No default value"));
            }
            "locale-sort" => assert!(input.contains("@Sort") && !input.is_ascii()),
            "root-expression-throw" => {
                assert!(parsed_full.root_expression.is_some());
                assert_eq!(
                    text(get(get(get(&one, "defaults"), "throw"), "message")),
                    "No default value"
                );
            }
            "parser-state-descriptor-default" => {
                assert_eq!(name, "runtime/state-descriptor-default");
                let value = parsed_full.state_declarations.get("$x").unwrap();
                assert_eq!(text(get(value, "k")), "Num");
                assert_eq!(get(value, "v"), &Value::Num(5.0));
                assert_eq!(
                    get(get(get(&one, "defaults"), "snapshot"), "$x"),
                    &Value::Num(5.0)
                );
            }
            "substitution-map-target" => {
                assert_eq!(name, "runtime/substitution-map-target");
                let root = parsed_full.root.as_ref().unwrap();
                for (key, field) in [("a", "els"), ("b", "entries"), ("c", "args")] {
                    let raw = root.props.get(key).unwrap();
                    assert_eq!(text(get(raw, "name")), "Each");
                    let template = arr(get(raw, "args")).get(2).unwrap();
                    assert_eq!(get(get(template, field), "map"), &Value::Num(3.0));
                }
            }
            "non-string-binding-target" => {
                assert_eq!(name, "runtime/binding-non-string-target");
                assert!(matches!(get(c, "stateValues"), Value::Obj(_)));
            }
            "function-state-value" => {
                assert_eq!(name, "runtime/binding-function-value");
                assert!(input.contains("$n={}.constructor"));
            }
            "descriptor-map-target" => {
                assert_eq!(name, "runtime/descriptor-map-target");
                let raw = parsed_full.root.as_ref().unwrap().props.get("a").unwrap();
                assert_eq!(text(get(raw, "k")), "Arr");
                assert_eq!(get(get(raw, "els"), "map"), &Value::Num(3.0));
            }
            _ => panic!("{name}: unknown divergence {divergence}"),
        }
        let non_string_pin = (divergence == "non-string-binding-target")
            .then(|| load(&dir.join("divergences/runtime-non-string-binding-target.json")));
        let write_override = if matches!(get(c, "queryComplete"), Value::Bool(false)) {
            let mut result = parsed_full.clone();
            for q in &mut result.query_statements {
                q.complete = false;
            }
            Some(result)
        } else {
            None
        };
        let write_result = write_override.as_ref().unwrap_or(&parsed_full);
        let mut counted_writes = false;
        for scenario in ["defaults", "persisted"] {
            let persistence = if scenario == "persisted" {
                saved
            } else {
                &NULL
            };
            let expected = json(get(&one, scenario));
            let actual = observe(
                &parsed_full,
                &library,
                &mut Store::new(),
                persistence,
                &input,
                c,
                &bindings,
                schema.get("actions"),
            );
            if !divergence.is_empty() {
                let pin = load(&dir.join(format!("divergences/runtime-{divergence}.json")));
                assert_eq!(text(get(&pin, "case")), name);
                assert_eq!(
                    json(get(get(&pin, "reference"), scenario)),
                    expected,
                    "{name}/{scenario}: pinned reference divergence output"
                );
                assert_eq!(
                    json(get(get(&pin, "rust"), scenario)),
                    actual,
                    "{name}/{scenario}: pinned Rust divergence output"
                );
                if divergence == "descriptor-map-target" {
                    compare_map_target_wording(
                        name,
                        divergence,
                        get(&one, scenario),
                        &from_json(&actual),
                    );
                }
            }
            if get(&pending, name) != &NULL {
                let checked = compare_pending_props(
                    &parsed_full,
                    &library,
                    get(&one, scenario),
                    &from_json(&actual),
                    name,
                    scenario,
                );
                partial_observations += usize::from(parsed_full.root.is_some());
                partial_props += checked;
            }
            if expected != actual {
                let _ = writeln!(
                    mismatches,
                    "case {name}, scenario {scenario}, oneshot step 1\n{}",
                    diff(&expected, &actual)
                );
            }
            let writes_path = golden.join(format!("{name}.writes.json"));
            let writes = writes_path.exists().then(|| load(&writes_path));
            let steps = writes
                .as_ref()
                .map(|w| arr(get(w, scenario)))
                .unwrap_or(&[]);
            let initial_markers = marker_paths(get(&from_json(&actual), "root"));
            assert_eq!(
                steps.len(),
                initial_markers.len() * 9,
                "{name}/{scenario}: missing scripted writes"
            );
            if !steps.is_empty() {
                write_cases += usize::from(!counted_writes);
                counted_writes = true;
                write_steps += steps.len();
                let mut store = Store::new();
                let first = observe(
                    &parsed_full,
                    &library,
                    &mut store,
                    persistence,
                    &input,
                    c,
                    &bindings,
                    schema.get("actions"),
                );
                let initial = from_json(&first);
                let mut visits: BTreeMap<Vec<String>, Vec<&str>> = initial_markers
                    .into_iter()
                    .map(|p| (p, Vec::new()))
                    .collect();
                for (step, write) in steps.iter().enumerate() {
                    let path: Vec<_> = arr(get(write, "path"))
                        .iter()
                        .map(|k| text(k).to_owned())
                        .collect();
                    let kind = match get(write, "value") {
                        Value::Null => "null",
                        Value::Bool(false) => "false",
                        Value::Bool(true) => "true",
                        Value::Num(_) => "number",
                        Value::Str(_) => "string",
                        Value::Arr(a) if a.is_empty() => "empty array",
                        Value::Arr(_) => "array",
                        Value::Obj(o) if o.is_empty() => "empty object",
                        Value::Obj(_) => "object",
                        _ => panic!("not JSON data"),
                    };
                    visits
                        .get_mut(&path)
                        .expect("write path from evaluated tree")
                        .push(kind);
                    let mut marker = get(&initial, "root");
                    for key in arr(get(write, "path")) {
                        marker = match marker {
                            Value::Obj(o) => o.get(text(key)).unwrap_or(&NULL),
                            Value::Arr(a) => text(key)
                                .parse::<usize>()
                                .ok()
                                .and_then(|i| a.get(i))
                                .unwrap_or(&NULL),
                            _ => &NULL,
                        };
                    }
                    assert_eq!(
                        json(marker),
                        json(get(write, "marker")),
                        "{name}/{scenario}/write {step}: marker from tree"
                    );
                    let mut write_input = TreeInput::new(write_result)
                        .with_library(&library)
                        .with_binding_props(&bindings);
                    if let Some(actions) = schema.get("actions") {
                        write_input = write_input.with_actions(actions);
                    }
                    if let Value::Obj(values) = get(c, "referenceValues") {
                        write_input = write_input.with_reference_values(values);
                    }
                    let before = store.snapshot();
                    let version = store.version();
                    let pristine: Vec<_> = before
                        .iter()
                        .map(|(name, _)| (name.clone(), store.is_pristine(name)))
                        .collect();
                    let result =
                        write_binding(write_input, &mut store, marker, get(write, "value").clone());
                    let write_error = match result {
                        Ok(_) => None,
                        Err(
                            error @ (BindingWriteError::Evaluation(_)
                            | BindingWriteError::InvalidMarker),
                        ) => {
                            assert!(
                                Arc::ptr_eq(&store.snapshot(), &before),
                                "{name}/{scenario}/write {step}: error snapshot identity"
                            );
                            assert_eq!(store.version(), version);
                            for (target, was_pristine) in &pristine {
                                assert_eq!(store.is_pristine(target), *was_pristine);
                            }
                            let error_name = if matches!(error, BindingWriteError::InvalidMarker) {
                                assert_eq!(divergence, "non-string-binding-target");
                                assert!(!matches!(get(marker, "target"), Value::Str(_)));
                                "InvalidMarker"
                            } else {
                                "TypeError"
                            };
                            let mut error_value = JsObject::new();
                            error_value.define("name", Value::Str(error_name.into()));
                            error_value.define("message", Value::Str(error.to_string()));
                            Some(Value::Obj(Arc::new(error_value)))
                        }
                        Err(error) => panic!("{name}/{scenario}/write {step}: {error}"),
                    };
                    let mut actual = evaluated_observation(
                        &parsed_full,
                        &library,
                        &store,
                        c,
                        &bindings,
                        schema.get("actions"),
                    );
                    if let Some(error) = write_error {
                        let mut observation_value = from_json(&actual);
                        let Value::Obj(observation) = &mut observation_value else {
                            panic!("observation")
                        };
                        Arc::make_mut(observation).define("writeError", error);
                        actual = json(&observation_value);
                    }
                    let expected = get(write, "observation");
                    if divergence == "non-string-binding-target" {
                        let pin = non_string_pin.as_ref().unwrap();
                        let wanted = arr(get(get(get(pin, "reference"), "writes"), scenario));
                        let pinned = arr(get(get(get(pin, "rust"), "writes"), scenario));
                        assert_eq!(wanted.len(), steps.len());
                        assert_eq!(pinned.len(), steps.len());
                        assert_eq!(json(&wanted[step]), json(write), "reference full write pin");
                        let mut actual_step = write.clone();
                        let Value::Obj(fields) = &mut actual_step else {
                            panic!("write object")
                        };
                        Arc::make_mut(fields).define("observation", from_json(&actual));
                        assert_eq!(
                            json(&pinned[step]),
                            json(&actual_step),
                            "Rust full write pin"
                        );
                        assert_ne!(
                            actual,
                            json(expected),
                            "refused write must remain a divergence"
                        );
                        let _ = writeln!(
                            mismatches,
                            "{name}/{scenario}/write {step}: pinned non-string target refusal"
                        );
                    } else if divergence == "function-state-value" {
                        let pin = load(&dir.join("divergences/runtime-function-state-value.json"));
                        let wanted = arr(get(get(get(&pin, "reference"), "writes"), scenario));
                        let pinned = arr(get(get(get(&pin, "rust"), "writes"), scenario));
                        assert_eq!(wanted.len(), steps.len());
                        assert_eq!(pinned.len(), steps.len());
                        assert_eq!(
                            json(&wanted[step]),
                            json(expected),
                            "{name}/{scenario}/write {step}: reference pin"
                        );
                        assert_eq!(
                            json(&pinned[step]),
                            actual,
                            "{name}/{scenario}/write {step}: Rust pin"
                        );
                        compare_function_state_value(expected, &from_json(&actual));
                        if actual != json(expected) {
                            let _ = writeln!(
                                mismatches,
                                "{name}/{scenario}/write {step}: pinned function-state-value divergence"
                            );
                        }
                    } else if listed {
                        compare_pending_props(
                            &parsed_full,
                            &library,
                            expected,
                            &from_json(&actual),
                            name,
                            scenario,
                        );
                    } else {
                        assert_eq!(actual, json(expected), "{name}/{scenario}/write {step}");
                    }
                }
                let mut kinds = vec![
                    "null",
                    "false",
                    "true",
                    "number",
                    "string",
                    "empty array",
                    "array",
                    "empty object",
                    "object",
                ];
                kinds.sort();
                for (path, mut actual) in visits {
                    actual.sort();
                    assert_eq!(
                        actual, kinds,
                        "{name}/{scenario}/{path:?}: each value kind through each marker"
                    );
                }
            }
            for (k, stream) in entries(&streams) {
                let plan = core_streams.as_ref().map(|c| get(c, k)).unwrap_or(stream);
                let mut store = Store::new();
                let mut parser = StreamParser::new(library.clone(), root);
                let cps: Vec<_> = input.chars().collect();
                let mut other = cps.clone();
                let mid = other.len() / 2;
                if let Some(c) = other.get_mut(mid) {
                    *c = if *c == 'Z' { 'Y' } else { 'Z' };
                }
                let mut pos = 0;
                let mut source = String::new();
                let fixed_sizes;
                let sizes: &[Value] = if matches!(get(plan, "sizes"), Value::Arr(_)) {
                    arr(get(plan, "sizes"))
                } else if let Ok(n) = k.parse::<usize>() {
                    fixed_sizes = (0..cps.len())
                        .step_by(n)
                        .map(|p| Value::Num(n.min(cps.len() - p) as f64))
                        .collect::<Vec<_>>();
                    &fixed_sizes
                } else {
                    &[]
                };
                let ops = arr(get(plan, "ops"));
                let expected = fingerprints(get(
                    stream,
                    if text(get(stream, scenario)) == "defaults" {
                        "defaults"
                    } else {
                        scenario
                    },
                ));
                assert_eq!(
                    expected.len(),
                    if !ops.is_empty() {
                        ops.len()
                    } else {
                        sizes.len() + 1
                    },
                    "{name}/{k}: plan step count"
                );
                let mut reported = false;
                for (step, want) in expected.iter().enumerate() {
                    let parsed = if !ops.is_empty() {
                        let op = &ops[step];
                        let n = number(&arr(op)[1]);
                        match text(&arr(op)[0]) {
                            "e" => parser.finish(),
                            "r" => parser.result(),
                            kind => {
                                let s: String = if kind == "p" { &cps } else { &other }
                                    .iter()
                                    .take(n)
                                    .collect();
                                source = s;
                                parser.set(&source)
                            }
                        }
                    } else if let Some(n) = sizes.get(step) {
                        let n = number(n);
                        let s: String = cps.iter().skip(pos).take(n).collect();
                        pos += n;
                        source.push_str(&s);
                        parser.push(&s)
                    } else {
                        parser.finish()
                    };
                    // A feature may occur only in a streaming prefix (the final
                    // parse can have no root at all). Witness that actual AST.
                    if scenario == "defaults" {
                        let reason = text(get(&pending, name));
                        if !reason.is_empty() && !witnessed[name].contains(reason) {
                            features_in(&parsed, &library, witnessed.get_mut(name).unwrap());
                        }
                    }
                    let actual = observe(
                        &parsed,
                        &library,
                        &mut store,
                        persistence,
                        &source,
                        c,
                        &bindings,
                        schema.get("actions"),
                    );
                    if fp(&actual) != *want && !reported {
                        reported = true;
                        let detail = if !listed && mismatches.is_empty() {
                            let reference = live(name, scenario, k, step + 1);
                            diff(&reference, &actual)
                        } else {
                            format!("fingerprint {} != {}", want, fp(&actual))
                        };
                        let _ = writeln!(
                            mismatches,
                            "case {name}, scenario {scenario}, chunking {k}, step {}, sizes {}, ops {}\n{detail}",
                            step + 1,
                            json(&Value::Arr(Arc::new(sizes.to_vec()))),
                            json(get(plan, "ops"))
                        );
                    }
                }
            }
        }
        free_json(schema);
        if !mismatches.is_empty() {
            failing.insert(name.to_owned());
            if !listed {
                report.push_str(&mismatches);
            }
        }
    }
    if std::env::var_os("RUNTIME_CASE").is_some() {
        assert!(report.is_empty(), "{report}");
        for (name, _) in entries(&pending) {
            if witnessed.contains_key(name) {
                assert!(
                    failing.contains(name),
                    "{name} matches now: remove its pending entry"
                );
            }
        }
        return;
    }
    println!(
        "Matching pending: {:?}",
        entries(&pending)
            .iter()
            .filter(|(n, _)| !failing.contains(n.as_str()))
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
    );
    for (name, reason) in entries(&pending) {
        assert!(
            cases.iter().any(|c| text(get(c, "name")) == name),
            "unknown pending {name}"
        );
        assert!(
            failing.contains(name),
            "{name} matches now: remove its pending entry"
        );
        let feature = text(reason);
        assert!(
            witnessed.get(name).is_some_and(|f| f.contains(feature)),
            "{name}: no independent AST/plan witness for {feature}"
        );
        assert!(
            arr(get(get(&evidence, name), "features"))
                .iter()
                .any(|f| text(f) == feature),
            "{name}: unsupported pending reason {feature}"
        );
        *counts.entry(feature.to_owned()).or_default() += 1;
    }
    for (name, _) in entries(&divergences) {
        assert!(
            cases.iter().any(|c| text(get(c, "name")) == name),
            "unknown divergence {name}"
        );
        assert!(
            failing.contains(name),
            "{name} now matches: remove its divergence entry"
        );
        assert!(!entries(&pending).iter().any(|(n, _)| *n == name));
    }
    assert_eq!(entries(&evidence).len(), cases.len());
    assert_eq!(number(get(&coverage, "total")), cases.len());
    assert_eq!(number(get(&coverage, "steps")), streamed);
    assert_eq!(number(get(&coverage, "recorded")), recorded);
    assert_eq!(number(get(&coverage, "recordedState")), recorded_state);
    assert_eq!(
        number(get(&coverage, "scenarioDifferences")),
        scenario_differences
    );
    for key in ["dynamic", "state", "references", "errors"] {
        let total = entries(&evidence)
            .iter()
            .filter(|(_, v)| matches!(get(v, key), Value::Bool(true)))
            .count();
        assert_eq!(number(get(&coverage, key)), total, "coverage {key}");
        let original = entries(&evidence)
            .iter()
            .filter(|(n, v)| !n.starts_with("runtime/") && matches!(get(v, key), Value::Bool(true)))
            .count();
        assert_eq!(
            number(get(get(&coverage, "original"), key)),
            original,
            "original {key}"
        );
    }
    let mut tickets = BTreeMap::new();
    for (feature, count) in &counts {
        let ticket = feature.split(':').next().unwrap();
        *tickets.entry(ticket).or_insert(0usize) += count;
    }
    let line = format!(
        "Runtime trees: {} total, {} matching, {} pending, {} divergences.",
        cases.len(),
        cases.len() - failing.len(),
        entries(&pending).len(),
        entries(&divergences).len()
    );
    let pending_line = format!(
        "Runtime pending: #34={}, #35={}, #36={}, #38={}.",
        tickets.get("34").unwrap_or(&0),
        tickets.get("35").unwrap_or(&0),
        tickets.get("36").unwrap_or(&0),
        tickets.get("38").unwrap_or(&0)
    );
    let coverage_line = format!(
        "Runtime coverage: {} dynamic, {} reading state, {} referencing statements, {} with runtime errors; {} streaming steps per scenario, {} differing observations, {} differing trees.",
        number(get(&coverage, "dynamic")),
        number(get(&coverage, "state")),
        number(get(&coverage, "references")),
        number(get(&coverage, "errors")),
        streamed,
        scenario_differences,
        number(get(&coverage, "treeDifferences"))
    );
    let original = get(&coverage, "original");
    let original_line = format!(
        "Original runtime coverage: {} cases, {} dynamic, {} reading state, {} referencing statements, {} with runtime errors; recorded: {recorded} cases, {recorded_state} declaring state.",
        cases
            .iter()
            .filter(|c| !text(get(c, "name")).starts_with("runtime/"))
            .count(),
        number(get(original, "dynamic")),
        number(get(original, "state")),
        number(get(original, "references")),
        number(get(original, "errors"))
    );
    let write_line = format!(
        "Runtime field writes: {write_cases} cases, {write_steps} steps across both scenarios; nine values per marker per scenario."
    );
    let partial_line = format!(
        "Runtime pending partial comparison: {partial_observations} one-shot observations, {partial_props} props."
    );
    println!("{line}\n{pending_line}\n{coverage_line}\n{partial_line}\n{write_line}");
    assert!(report.is_empty(), "{report}");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../CONFORMANCE.md");
    let mut doc = std::fs::read_to_string(&path).unwrap();
    if std::env::var_os("UPDATE_REPORT").is_some() {
        let start = "<!-- runtime-counts -->";
        let end = "<!-- /runtime-counts -->";
        let a = doc.find(start).unwrap() + start.len();
        let b = doc[a..].find(end).unwrap() + a;
        doc.replace_range(
            a..b,
            &format!("\n{line}\n\n{pending_line}\n\n{coverage_line}\n\n{original_line}\n\n{partial_line}\n\n{write_line}\n"),
        );
        std::fs::write(&path, &doc).unwrap();
    }
    for needed in [
        &original_line,
        &pending_line,
        &coverage_line,
        &partial_line,
        &write_line,
    ] {
        assert!(doc.contains(needed), "CONFORMANCE.md needs: {needed}");
    }
    assert!(doc.contains(&line), "CONFORMANCE.md needs: {line}");
}
fn ast_features(a: &openui_lang::Ast, l: &Library, out: &mut BTreeSet<String>) {
    a.walk(&mut |a| {
        use openui_lang::{Ast, CallKind};
        let reason = match a {
            Ast::Assign { .. } => Some("35:bindings"),
            Ast::RuntimeRef { .. } => Some("34:query"),
            Ast::Comp { name, .. } if name == "Each" => Some("34:each"),
            Ast::Comp { name, .. } => match l.call(name) {
                Some(c) if c.kind == CallKind::Action => Some("36:actions"),
                Some(c) if !c.builtin => Some("38:functions"),
                _ => {
                    if l.component(name).is_some() {
                        Some("34:components")
                    } else {
                        None
                    }
                }
            },
            _ => None,
        };
        if let Some(reason) = reason {
            out.insert(reason.to_owned());
        }
    });
}
fn features_in(p: &ParseResult, l: &Library, out: &mut BTreeSet<String>) {
    if let Some(e) = &p.root {
        for v in e.props.values() {
            features_value(v, l, out);
        }
    }
    if let Some(a) = &p.root_expression {
        out.insert("34:root".into());
        ast_features(a, l, out);
    }
}
fn features_value(value: &Value, l: &Library, out: &mut BTreeSet<String>) {
    let mut todo = vec![value];
    while let Some(v) = todo.pop() {
        match v {
            Value::Ast(a) => ast_features(a, l, out),
            Value::Element(e) => todo.extend(e.props.values()),
            Value::Arr(a) => todo.extend(a.iter()),
            Value::Obj(o) => {
                match text(o.get("k").unwrap_or(&NULL)) {
                    "RuntimeRef" => {
                        out.insert("34:query".into());
                    }
                    "Comp" => {
                        let name = text(o.get("name").unwrap_or(&NULL));
                        let reason = if name == "Each" {
                            Some("34:each")
                        } else {
                            match l.call(name) {
                                Some(c) if c.kind == openui_lang::CallKind::Action => {
                                    Some("36:actions")
                                }
                                Some(c) if !c.builtin => Some("38:functions"),
                                _ if l.component(name).is_some() => Some("34:components"),
                                _ => None,
                            }
                        };
                        if let Some(reason) = reason {
                            out.insert(reason.into());
                        }
                    }
                    _ => {}
                }
                if matches!(o.get("k"),Some(Value::Str(k)) if k=="Assign") {
                    out.insert("35:bindings".into());
                    out.insert("35:assign-descriptor".into());
                }
                if o.has_own("__reactive") {
                    out.insert("35:bindings".into());
                }
                if matches!(o.get("steps"), Some(Value::Arr(_))) {
                    out.insert("36:actions".into());
                }
                todo.extend(o.values());
            }
            _ => {}
        }
    }
}

fn from_json(text: &str) -> Value {
    let j = parse_json(text).unwrap();
    let v = Value::from_json(&j);
    free_json(j);
    v
}

// Full JSON is already committed for one-shot observations. Compare every
// independent prop, including nested elements under a prop containing actions.
// Per-step goldens are fingerprints: adding masked per-step results would add
// a second large oracle. Whole-result streaming comparisons remain in place.
fn compare_pending_props(
    raw: &ParseResult,
    library: &Library,
    expected: &Value,
    actual: &Value,
    case: &str,
    scenario: &str,
) -> usize {
    fn own<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
        match v {
            Value::Obj(o) => o.get(key),
            Value::Arr(a) => key.parse::<usize>().ok().and_then(|i| a.get(i)),
            _ => None,
        }
    }
    fn compare(want: Option<&Value>, got: Option<&Value>, context: &str) {
        assert_eq!(want.map(json), got.map(json), "{context}");
    }
    fn walk(
        raw: &Value,
        want: Option<&Value>,
        got: Option<&Value>,
        l: &Library,
        path: &str,
        n: &mut usize,
        owners: &mut BTreeMap<(String, Option<String>, String), bool>,
    ) {
        let props = match raw {
            Value::Element(e) => Some(e.props.clone()),
            Value::Obj(o) if text(get(raw, "type")) == "element" && o.has_own("props") => {
                match o.get("props") {
                    Some(Value::Obj(p)) => Some((**p).clone()),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(props) = props {
            let wp = want.and_then(|v| own(v, "props"));
            let gp = got.and_then(|v| own(v, "props"));
            for (k, v) in props.iter() {
                let context = format!("{path}.props.{k}");
                let mut features = BTreeSet::new();
                features_value(v, l, &mut features);
                features.retain(|f| f.starts_with("38:"));
                let w = wp.and_then(|o| own(o, k));
                let g = gp.and_then(|o| own(o, k));
                let (component, statement) = match raw {
                    Value::Element(e) => (e.type_name.clone(), e.statement_id.clone()),
                    _ => (
                        text(get(raw, "typeName")).to_owned(),
                        match get(raw, "statementId") {
                            Value::Str(id) => Some(id.clone()),
                            _ => None,
                        },
                    ),
                };
                owners
                    .entry((component, statement, k.clone()))
                    .and_modify(|safe| *safe &= features.is_empty())
                    .or_insert(features.is_empty());
                if features.is_empty() {
                    compare(w, g, &context);
                    *n += 1;
                }
                walk(v, w, g, l, &context, n, owners);
            }
        } else {
            match raw {
                Value::Arr(a) => {
                    for (i, v) in a.iter().enumerate() {
                        let k = i.to_string();
                        walk(
                            v,
                            want.and_then(|o| own(o, &k)),
                            got.and_then(|o| own(o, &k)),
                            l,
                            &format!("{path}[{i}]"),
                            n,
                            owners,
                        );
                    }
                }
                Value::Obj(o)
                    if !o.has_own("k") && !o.has_own("steps") && !o.has_own("__reactive") =>
                {
                    for (k, v) in o.iter() {
                        walk(
                            v,
                            want.and_then(|o| own(o, k)),
                            got.and_then(|o| own(o, k)),
                            l,
                            &format!("{path}.{k}"),
                            n,
                            owners,
                        );
                    }
                }
                _ => {}
            }
        }
    }
    compare(
        Some(get(expected, "snapshot")),
        Some(get(actual, "snapshot")),
        &format!("{case}/{scenario}: pending snapshot"),
    );
    let mut checked = 0;
    let mut owners = BTreeMap::new();
    if let Some(root) = &raw.root {
        let w = get(expected, "root");
        let g = get(actual, "root");
        for key in [
            "type",
            "typeName",
            "partial",
            "hasDynamicProps",
            "statementId",
        ] {
            compare(
                own(w, key),
                own(g, key),
                &format!("{case}/{scenario}: root.{key}"),
            );
        }
        walk(
            &Value::Element(Arc::new(root.clone())),
            Some(w),
            Some(g),
            library,
            &format!("{case}/{scenario}: pending root"),
            &mut checked,
            &mut owners,
        );
    }
    // Diagnostics have an owner and prop but no tree path. If distinct raw
    // elements collide on that identity, compare only when all are in scope.
    let errors = |observation: &Value| {
        arr(get(observation, "errors"))
            .iter()
            .filter(|error| {
                let Some(prop) = text(get(error, "hint"))
                    .strip_prefix("Check the expression used for prop \"")
                    .and_then(|s| s.strip_suffix('"'))
                else {
                    return false;
                };
                let statement = match get(error, "statementId") {
                    Value::Str(id) => Some(id.clone()),
                    _ => None,
                };
                owners.get(&(
                    text(get(error, "component")).to_owned(),
                    statement,
                    prop.to_owned(),
                )) == Some(&true)
            })
            .map(json)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        errors(expected),
        errors(actual),
        "{case}/{scenario}: pending in-scope errors"
    );
    checked
}

// This is deliberately a named, structural exception, never a general error
// normalizer. Both complete outputs are committed; exactly one message differs.
fn compare_map_target_wording(case: &str, reason: &str, expected: &Value, actual: &Value) {
    assert_eq!(
        (case, reason),
        ("runtime/descriptor-map-target", "descriptor-map-target")
    );
    let expected_errors = arr(get(expected, "errors"));
    let actual_errors = arr(get(actual, "errors"));
    assert_eq!(expected_errors.len(), 1);
    assert_eq!(actual_errors.len(), 1);
    assert!(text(get(&expected_errors[0], "message")).ends_with("'node.els.map' is 3)"));
    assert!(text(get(&actual_errors[0], "message")).ends_with("'node.els.map' is undefined)"));
    let Value::Obj(root) = actual else {
        panic!("observation")
    };
    let Value::Obj(error) = &actual_errors[0] else {
        panic!("error")
    };
    let mut error = (**error).clone();
    error.define("message", get(&expected_errors[0], "message").clone());
    let mut normalized = (**root).clone();
    normalized.define(
        "errors",
        Value::Arr(Arc::new(vec![Value::Obj(Arc::new(error))])),
    );
    assert_eq!(
        json(expected),
        json(&Value::Obj(Arc::new(normalized))),
        "{case}: only the pinned call-target wording may differ"
    );
}

fn marker_paths(root: &Value) -> Vec<Vec<String>> {
    fn walk(v: &Value, path: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
        match v {
            Value::Obj(_) if text(get(v, "__reactive")) == "assign" => out.push(path.clone()),
            Value::Obj(o) => {
                for (k, v) in o.iter() {
                    path.push(k.clone());
                    walk(v, path, out);
                    path.pop();
                }
            }
            Value::Arr(a) => {
                for (i, v) in a.iter().enumerate() {
                    path.push(i.to_string());
                    walk(v, path, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let mut result = Vec::new();
    walk(root, &mut Vec::new(), &mut result);
    result
}

// The public core Value erases functions to undefined. Pin this single
// consequence: after storing Object, !state is false upstream and true here.
fn compare_function_state_value(expected: &Value, actual: &Value) {
    assert_eq!(
        get(get(get(expected, "root"), "props"), "display"),
        &Value::Bool(false)
    );
    assert_eq!(
        get(get(get(actual, "root"), "props"), "display"),
        &Value::Bool(true)
    );
    for observation in [expected, actual] {
        let Value::Obj(snapshot) = get(observation, "snapshot") else {
            panic!("snapshot")
        };
        assert!(!snapshot.has_own("$n"));
    }
    let Value::Obj(observation) = actual else {
        panic!("observation")
    };
    let Value::Obj(root) = get(actual, "root") else {
        panic!("root")
    };
    let Value::Obj(props) = get(get(actual, "root"), "props") else {
        panic!("props")
    };
    let mut props = (**props).clone();
    props.define("display", Value::Bool(false));
    let mut root = (**root).clone();
    root.define("props", Value::Obj(Arc::new(props)));
    let mut observation = (**observation).clone();
    observation.define("root", Value::Obj(Arc::new(root)));
    assert_eq!(
        json(expected),
        json(&Value::Obj(Arc::new(observation))),
        "only function-state truthiness may differ"
    );
}
