//! Runtime mode of the differential fuzzer: SplitMix64 cases, public runtime
//! calls, the existing reference golden adapter, bounded children and shrinking.
use crate::{MAX_SAVES, findings, fingerprint, rng::Rng};
use openui_lang::{Library, ParseResult, parse};
use openui_runtime::{
    ActionContinuation, BindingProps, JsObject, Store, TreeInput, Value as RuntimeValue,
    evaluate_tree, get_field_value, initialize_field_default, resume_action, run_action,
    set_field_value, value_to_json, write_binding,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const MAX_INPUT: usize = 65536;
const MAX_OPS: usize = 64;
const MAX_SOURCE: usize = 4096;
fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}
fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}
fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn object(value: &Value) -> impl Iterator<Item = (&String, &Value)> {
    value.as_object().into_iter().flatten()
}
fn runtime(value: &Value) -> RuntimeValue {
    RuntimeValue::from_json(value)
}
fn json_value(value: &RuntimeValue) -> Result<Value> {
    Ok(serde_json::from_str(&value_to_json(value)?)?)
}
fn load(path: &Path) -> Result<Value> {
    // Corpus manifests include intentional lone UTF-16 surrogates. Use the
    // public reader used by the conformance suite, rather than serde's refusal.
    Ok(openui_lang::parse_json(&std::fs::read_to_string(path)?)?)
}

fn conformance(root: &Path) -> PathBuf {
    std::env::var_os("OPENUI_CONFORMANCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("conformance"))
}

/// At most 40 nodes per value, depth <= 3, width <= 3, strings <= 24 bytes.
fn state(rng: &mut Rng, depth: usize) -> Value {
    match rng.below(if depth == 0 { 5 } else { 8 }) {
        0 => Value::Null,
        1 => json!(rng.chance(50)),
        2 => json!(rng.below(101) as i64 - 50),
        3 => json!(rng.pick(&["", "0", "a\"b", "line\nend", "λ😀", "saved"])),
        4 => json!(rng.below(1000) as f64 / 4.0),
        5 => Value::Array((0..rng.below(4)).map(|_| state(rng, depth - 1)).collect()),
        _ => {
            let mut map = serde_json::Map::new();
            for name in ["value", "nested", "active"].iter().take(rng.below(4)) {
                map.insert((*name).into(), state(rng, depth - 1));
            }
            Value::Object(map)
        }
    }
}
fn expression(rng: &mut Rng) -> String {
    match rng.below(6) {
        0 => "$n + 1".into(),
        1 => "$n ? $data : $draft".into(),
        2 => "{n:$n, data:$data}".into(),
        3 => "[$draft,$n,$data]".into(),
        4 => "@Sum([1,2,3])".into(),
        _ => format!("{}", rng.below(100)),
    }
}
fn marker() -> Value {
    json!({"__reactive":"assign","target":"$draft","expr":{"k":"StateRef","n":"$value"}})
}
fn generated(seed: u64, index: u64, root: &Path) -> Result<Value> {
    let mut rng = Rng::for_program(seed, index);
    // Exercise already-pinned divergences too, with no broad source-based skip.
    if index % 32 == 31 {
        let manifest = load(&conformance(root).join("runtime.json"))?;
        let known: Vec<_> = array(get(&manifest, "cases"))
            .iter()
            .filter(|c| text(get(c, "name")).starts_with("runtime/action-target-"))
            .collect();
        if let Some(c) = known.get(rng.below(known.len().max(1))) {
            let mut c = (*c).clone();
            let source = std::fs::read_to_string(conformance(root).join(text(get(&c, "file"))))?;
            c.as_object_mut()
                .ok_or("case object")?
                .insert("fuzzInput".into(), json!(source));
            return Ok(c);
        }
    }
    let expr = expression(&mut rng);
    let initial = state(&mut rng, 2);
    let source = format!(
        "$n=0\n$draft=\"initial\"\n$data={initial}\n$seen=null\nq=Query(\"q\",{{}},{{count:1}})\nm=Mutation(\"m\",{{draft:$draft,n:$n}})\nroot=Box(Box($n,$data,$seen,$draft),Field($draft=$value,$draft),Action([@Set($seen,{expr}),@Run(m),@Set($seen,$draft),@Run(m),@Set($seen,$data),@Run(q),@ToAssistant(\"sent\",$seen)]),Action([@Set($n,$n+1),@Reset($data),@OpenUrl(\"https://example.test/\"),Go(\"go\")]))"
    );
    let mut ops = vec![json!({"kind":"redraw"})];
    for _ in 0..4 + rng.below(9) {
        let value = state(&mut rng, 3);
        let form = rng.pick(&["", "contact"]);
        ops.push(match rng.below(7) {
            0 => json!({"kind":"set","name":rng.pick(&["$data","$draft","$n"]),"value":value}),
            1 => json!({"kind":"binding","marker":marker(),"value":value}),
            2 => json!({"kind":"run","handle":1,"message":"generated","formName":form}),
            3 => json!({"kind":"default","name":rng.pick(&["name","email"]),"componentType":"Input","value":value,"formName":form,"streaming":rng.chance(50)}),
            4 => json!({"kind":"field","name":rng.pick(&["name","email"]),"componentType":"Input","value":value,"formName":form,"save":rng.chance(50)}),
            5 => json!({"kind":"references","values":{"q":value}}),
            _ => json!({"kind":"redraw"}),
        });
    }
    // Every ordinary case covers all operation families and two suspensions,
    // including host edits and a new program between mutation steps.
    ops.extend([
        json!({"kind":"default","name":"settled","value":state(&mut rng,2),"streaming":false}),
        json!({"kind":"field","name":"name","componentType":"Input","value":state(&mut rng,2),"formName":"contact","save":true}),
        json!({"kind":"binding","marker":marker(),"value":state(&mut rng,2)}),
        json!({"kind":"run","handle":0,"message":"suspend","formName":"contact"}),
        json!({"kind":"set","name":"$draft","value":state(&mut rng,3)}),
        json!({"kind":"field","name":"between","value":state(&mut rng,2),"formName":"contact","save":false}),
        json!({"kind":"resume","success":true}),
        json!({"kind":"set","name":"$data","value":state(&mut rng,3)}),
        json!({"kind":"program","program":format!("{source}\n$data={}\n$n={}",state(&mut rng,2),rng.below(20))}),
        json!({"kind":"resume","success":rng.chance(50)}),
        json!({"kind":"resume","success":true}),
        json!({"kind":"run","handle":1,"message":"reset"}),
        json!({"kind":"retain","handle":1}),
        json!({"kind":"runState","message":"retained"}),
        json!({"kind":"dispose"}),
        json!({"kind":"run","handle":1,"message":"revoked"}),
    ]);
    Ok(
        json!({"name":format!("runtime/fuzz-{seed}-{index}"),"schema":"runtime-runner","fuzzInput":source,"stateValues":{"$data":state(&mut rng,3),"$draft":state(&mut rng,3)},"actionSequenceOnly":true,"manualMutationOutcomes":true,"actionInteractions":ops}),
    )
}
fn validate(c: &Value) -> Result<()> {
    if c.to_string().len() > MAX_INPUT
        || text(get(c, "fuzzInput")).len() > MAX_SOURCE
        || array(get(c, "actionInteractions")).len() > MAX_OPS
        || array(get(c, "actionInteractions"))
            .iter()
            .any(|op| text(get(op, "program")).len() > MAX_SOURCE)
    {
        return Err("runtime case bounds exceeded".into());
    }
    Ok(())
}
fn schema(root: &Path, name: &str) -> Result<Value> {
    for manifest in ["runtime", "manifest"] {
        let m = load(&conformance(root).join(format!("{manifest}.json")))?;
        if let Some(path) = get(&m, "schemas").get(name).and_then(Value::as_str) {
            return load(&conformance(root).join(path));
        }
    }
    Err(format!("unknown runtime schema {name}").into())
}
fn bindings(root: &Path, name: &str) -> Result<BindingProps> {
    let manifest = load(&conformance(root).join("bindings.json"))?;
    if let Some(path) = manifest.get(name).and_then(Value::as_str) {
        let spec = load(&conformance(root).join(path))?;
        Ok(BindingProps::from_prompt_spec(&runtime(get(
            &spec, "library",
        )))?)
    } else {
        Ok(BindingProps::new())
    }
}
fn input<'a>(
    result: &'a ParseResult,
    library: &'a Library,
    bindings: &'a BindingProps,
    schema: &'a Value,
    references: &'a JsObject<RuntimeValue>,
) -> TreeInput<'a> {
    let input = TreeInput::new(result)
        .with_library(library)
        .with_binding_props(bindings)
        .with_reference_values(references);
    match schema.get("actions") {
        Some(a) => input.with_actions(a),
        None => input,
    }
}
fn references(value: &Value) -> JsObject<RuntimeValue> {
    let mut out = JsObject::new();
    for (k, v) in object(value) {
        out.define(k.clone(), runtime(v));
    }
    out
}
fn zero_counts(schema: &Value) -> Value {
    object(get(schema, "functions"))
        .map(|(k, _)| (k.clone(), json!(0)))
        .collect()
}
fn observe(
    result: &ParseResult,
    library: &Library,
    store: &Store,
    bindings: &BindingProps,
    schema: &Value,
    refs: &JsObject<RuntimeValue>,
) -> Result<(Value, Option<RuntimeValue>, Value)> {
    let tree = evaluate_tree(input(result, library, bindings, schema, refs), store);
    let flags = json!({"parseSize":result.size_limited,"parseDepth":result.depth_limited,"treeSize":tree.size_limited,"treeDepth":tree.depth_limited,"actionSize":false,"actionDepth":false});
    let tree_json = tree.to_json()?;
    let snapshot_json = store.snapshot_json()?;
    let mut observation: Value = serde_json::from_str(&tree_json)?;
    let map = observation.as_object_mut().ok_or("tree object")?;
    map.insert("snapshot".into(), serde_json::from_str(&snapshot_json)?);
    if schema.get("functions").is_some() {
        map.insert("callCounts".into(), zero_counts(schema));
    }
    map.insert(
        "serialized".into(),
        json!({"tree":tree_json,"snapshot":snapshot_json,"events":"[]"}),
    );
    Ok((observation, tree.root, flags))
}
fn discover(
    value: &RuntimeValue,
    source: &str,
    path: Vec<String>,
    out: &mut Vec<(Value, RuntimeValue)>,
) {
    let children: Vec<_> = match value {
        RuntimeValue::Obj(o) => {
            if matches!(o.get("steps"), Some(RuntimeValue::Arr(_))) {
                out.push((json!({"source":source,"path":path}), value.clone()));
            }
            o.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        }
        RuntimeValue::Element(e) => {
            vec![("props".into(), RuntimeValue::Obj(Arc::new(e.props.clone())))]
        }
        RuntimeValue::Arr(a) => a
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v.clone()))
            .collect(),
        _ => Vec::new(),
    };
    for (key, child) in children {
        let mut p = path.clone();
        p.push(key);
        discover(&child, source, p, out);
    }
}
fn snapshot(store: &Store) -> RuntimeValue {
    let mut object = JsObject::new();
    for (k, v) in store.snapshot().iter() {
        if let Some(v) = v {
            object.define(k.clone(), v.clone());
        }
    }
    RuntimeValue::Obj(Arc::new(object))
}
fn replay(c: &Value, saved: &Value, root: &Path) -> Result<Value> {
    validate(c)?;
    let schema = schema(root, text(get(c, "schema")))?;
    let library = Library::from_json_schema(&schema);
    let bindings = bindings(root, text(get(c, "schema")))?;
    let mut initial = serde_json::Map::new();
    let mut actions = serde_json::Map::new();
    for scenario in ["defaults", "persisted"] {
        let mut result = parse(text(get(c, "fuzzInput")), &library, None);
        let mut store = Store::new();
        let mut refs = references(get(c, "referenceValues"));
        let mut persistence = if scenario == "persisted" {
            saved.clone()
        } else {
            json!({})
        };
        let p = persistence.as_object_mut().ok_or("saved object")?;
        for (k, v) in object(get(c, "stateValues")) {
            p.insert(k.clone(), v.clone());
        }
        let values: Vec<_> = object(&persistence).map(|(k, v)| (k, runtime(v))).collect();
        let initialized = store.initialize(
            result
                .state_declarations
                .iter()
                .map(|(k, v)| (k.as_str(), Some(v))),
            values.iter().map(|(k, v)| (k.as_str(), Some(v))),
        );
        if !initialized.refused.is_empty() {
            return Err("generated initialization refused".into());
        }
        let (mut first, tree, flags) =
            observe(&result, &library, &store, &bindings, &schema, &refs)?;
        first
            .as_object_mut()
            .ok_or("observation")?
            .insert("flags".into(), flags);
        initial.insert(scenario.into(), first);
        let mut handles = Vec::new();
        if let Some(tree) = tree {
            discover(&tree, "tree", vec![], &mut handles);
        }
        discover(&snapshot(&store), "state", vec![], &mut handles);
        let mut tokens: BTreeMap<String, ActionContinuation> = BTreeMap::new();
        let mut steps = Vec::new();
        for op in array(get(c, "actionInteractions")) {
            let form = get(op, "formName").as_str();
            let mut events = Vec::new();
            let mut event_json = Vec::new();
            let mut failed = false;
            let mut suspended = None;
            let mut save = false;
            let mut field = None;
            let mut action_flags = (false, false);
            let tree_input = input(&result, &library, &bindings, &schema, &refs);
            match text(get(op, "kind")) {
                "set" => {
                    failed = store
                        .set(text(get(op, "name")), Some(runtime(get(op, "value"))))
                        .is_err();
                }
                "binding" => {
                    failed = write_binding(
                        tree_input,
                        &mut store,
                        &runtime(get(op, "marker")),
                        runtime(get(op, "value")),
                    )
                    .is_err();
                }
                "field" | "default" => {
                    let value = op.get("value").map(runtime);
                    let r = if text(get(op, "kind")) == "default" {
                        initialize_field_default(
                            &mut store,
                            form,
                            get(op, "componentType").as_str(),
                            text(get(op, "name")),
                            value,
                            get(op, "streaming") == &json!(true),
                        )
                    } else {
                        set_field_value(
                            &mut store,
                            form,
                            get(op, "componentType").as_str(),
                            text(get(op, "name")),
                            value,
                            get(op, "save") != &json!(false),
                        )
                        .map(Some)
                    };
                    match r {
                        Ok(Some(r)) => save = r.save_requested,
                        Ok(None) => {}
                        Err(_) => failed = true,
                    }
                    field = get_field_value(&store, form, text(get(op, "name")));
                }
                "run" | "runState" => {
                    let retained = store.get("$retained");
                    let action = if text(get(op, "kind")) == "runState" {
                        retained.as_ref()
                    } else {
                        Some(
                            get(op, "handle")
                                .as_u64()
                                .and_then(|i| handles.get(i as usize))
                                .map(|(_, v)| v)
                                .ok_or("run handle not discovered")?,
                        )
                    };
                    let r = run_action(
                        tree_input,
                        &mut store,
                        action,
                        text(get(op, "message")),
                        form,
                    );
                    failed = r.failure.is_some();
                    suspended = Some(r.is_suspended());
                    action_flags = (r.size_limited, r.depth_limited);
                    events = r
                        .events
                        .iter()
                        .map(|e| json_value(e.value()))
                        .collect::<Result<_>>()?;
                    event_json = r
                        .events
                        .iter()
                        .map(|e| e.to_json())
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    if let Some(token) = r.continuation {
                        tokens.insert(get(op, "slot").as_str().unwrap_or("current").into(), token);
                    }
                }
                "resume" => {
                    let slot = get(op, "slot").as_str().unwrap_or("current");
                    match tokens.get(slot).map(|t| {
                        resume_action(
                            tree_input,
                            &mut store,
                            t,
                            get(op, "success") == &json!(true),
                        )
                    }) {
                        Some(Ok(r)) => {
                            failed = r.failure.is_some();
                            suspended = Some(r.is_suspended());
                            action_flags = (r.size_limited, r.depth_limited);
                            events = r
                                .events
                                .iter()
                                .map(|e| json_value(e.value()))
                                .collect::<Result<_>>()?;
                            event_json = r
                                .events
                                .iter()
                                .map(|e| e.to_json())
                                .collect::<std::result::Result<Vec<_>, _>>()?;
                            if let Some(token) = r.continuation {
                                tokens
                                    .insert(get(op, "into").as_str().unwrap_or(slot).into(), token);
                            }
                        }
                        _ => {
                            failed = true;
                            suspended = Some(false);
                        }
                    }
                }
                "retain" => {
                    let action = get(op, "handle")
                        .as_u64()
                        .and_then(|i| handles.get(i as usize))
                        .ok_or("missing retained plan")?;
                    let m = json!({"__reactive":"assign","target":"$retained","expr":{"k":"StateRef","n":"$value"}});
                    failed = write_binding(tree_input, &mut store, &runtime(&m), action.1.clone())
                        .is_err();
                }
                "program" => {
                    result = parse(text(get(op, "program")), &library, None);
                    if !store
                        .initialize(
                            result
                                .state_declarations
                                .iter()
                                .map(|(k, v)| (k.as_str(), Some(v))),
                            [],
                        )
                        .refused
                        .is_empty()
                    {
                        return Err("program initialization refused".into());
                    }
                }
                "references" => {
                    refs = references(get(op, "values"));
                }
                "dispose" => {
                    store.dispose();
                }
                "redraw" => {}
                other => return Err(format!("unknown interaction {other}").into()),
            }
            let (draw, _, mut flags) =
                observe(&result, &library, &store, &bindings, &schema, &refs)?;
            let flag_map = flags.as_object_mut().ok_or("flags")?;
            flag_map.insert("actionSize".into(), json!(action_flags.0));
            flag_map.insert("actionDepth".into(), json!(action_flags.1));
            let mut observation = serde_json::Map::new();
            observation.insert("events".into(), json!(events));
            observation.insert("failed".into(), json!(failed));
            if let Some(s) = suspended {
                observation.insert("suspended".into(), json!(s));
            }
            if save {
                observation.insert("saveRequested".into(), json!(true));
            }
            if let Some(v) = field {
                observation.insert("fieldValue".into(), json_value(&v)?);
            }
            let mut pristine: Vec<_> = store
                .snapshot()
                .iter()
                .filter(|(n, _)| store.is_pristine(n))
                .map(|(n, _)| n.clone())
                .collect();
            pristine.sort_by(|a, b| openui_lang::utf16_units(a).cmp(openui_lang::utf16_units(b)));
            observation.insert("pristine".into(), json!(pristine));
            for (k, v) in object(&draw) {
                if k != "serialized" {
                    observation.insert(k.clone(), v.clone());
                }
            }
            if schema.get("functions").is_some() {
                observation.insert("actionCallCounts".into(), zero_counts(&schema));
            }
            let mut serialized = get(&draw, "serialized").clone();
            serialized
                .as_object_mut()
                .ok_or("serialized observation")?
                .insert(
                    "events".into(),
                    json!(format!("[{}]", event_json.join(","))),
                );
            observation.insert("serialized".into(), serialized);
            observation.insert("flags".into(), flags);
            steps.push(json!({"op":op,"observation":observation}));
        }
        actions.insert(
            scenario.into(),
            json!({"handles":handles.iter().map(|(h,_)|h).collect::<Vec<_>>(),"steps":steps}),
        );
    }
    Ok(json!({"initial":initial,"actions":actions}))
}
fn flags() -> Value {
    json!({"parseSize":false,"parseDepth":false,"treeSize":false,"treeDepth":false,"actionSize":false,"actionDepth":false})
}
// Existing divergence pins contain the serializer's canonical output. Live
// observations carry their original strings instead of deriving them here.
fn pinned_serialized(observation: &Value) -> Value {
    let events = observation
        .get("events")
        .cloned()
        .unwrap_or_else(|| json!([]));
    json!({"tree":openui_lang::stringify_json(&json!({"root":get(observation,"root"),"errors":get(observation,"errors")})),"snapshot":openui_lang::stringify_json(get(observation,"snapshot")),"events":openui_lang::stringify_json(&events)})
}
#[derive(Clone, Copy)]
enum Serialization {
    Original,
    Pinned,
}
fn expand_actions(actions: &Value, source: Serialization) -> Result<Value> {
    let mut out = serde_json::Map::new();
    for scenario in ["defaults", "persisted"] {
        let mut steps = Vec::new();
        for step in array(get(get(actions, scenario), "steps")) {
            let mut observation = serde_json::Map::new();
            for (k, v) in object(get(step, "observation")) {
                let v = if let Some(i) = v.get("$value").and_then(Value::as_u64) {
                    get(actions, "values")
                        .get(i as usize)
                        .ok_or("invalid intern id")?
                } else {
                    v
                };
                observation.insert(k.clone(), v.clone());
            }
            match source {
                Serialization::Pinned => {
                    let serialized = pinned_serialized(&Value::Object(observation.clone()));
                    observation.insert("serialized".into(), serialized);
                }
                Serialization::Original if !observation.contains_key("serialized") => {
                    return Err("reference omitted serialized observation".into());
                }
                Serialization::Original => {}
            }
            observation.insert("flags".into(), flags());
            steps.push(json!({"op":get(step,"op"),"observation":observation}));
        }
        out.insert(
            scenario.into(),
            json!({"handles":get(get(actions,scenario),"handles"),"steps":steps}),
        );
    }
    Ok(Value::Object(out))
}
fn normalize(reference: &Value) -> Result<Value> {
    let mut initial = get(reference, "initial").clone();
    for (_, v) in initial.as_object_mut().ok_or("reference initial object")? {
        if v.get("serialized").is_none() {
            return Err("reference omitted initial serialization".into());
        }
        v.as_object_mut()
            .ok_or("initial observation")?
            .insert("flags".into(), flags());
    }
    Ok(
        json!({"initial":initial,"actions":expand_actions(get(reference,"actions"),Serialization::Original)?}),
    )
}
fn same(a: &Value, b: &Value) -> bool {
    // Do not use the serializer under test to compare its own output. Raw
    // serialized fields are strings and keep every byte. Controller numbers
    // have JS double semantics (121 and 121.0 denote the same input).
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|((ka, va), (kb, vb))| ka == kb && same(va, vb))
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| same(a, b))
        }
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Null, Value::Null) => true,
        _ => false,
    }
}
/// Recognize only complete pairs already committed by the conformance suite.
/// A changed output, extra step or near miss is an unexplained finding.
fn explained(c: &Value, reference: &Value, actual: &Value, root: &Path) -> Result<bool> {
    let name = text(get(c, "name"));
    if !name.starts_with("runtime/action-target-") {
        return Ok(false);
    }
    let pin = load(&conformance(root).join(format!(
        "divergences/{}-runs.json",
        name.replace("runtime/", "runtime-")
    )))?;
    Ok(same(get(reference, "initial"), get(actual, "initial"))
        && same(
            get(reference, "actions"),
            &expand_actions(get(&pin, "reference"), Serialization::Pinned)?,
        )
        && same(
            get(actual, "actions"),
            &expand_actions(get(&pin, "rust"), Serialization::Pinned)?,
        ))
}
fn child(root: &Path, command: &[String], request: &Value) -> Result<Value> {
    let mut child = Command::new("python3")
        .arg(root.join("scripts/fuzz-child.py"))
        .args(["--seconds", "2", "--memory-mb", "256", "--"])
        .args(command)
        .current_dir(root.join("reference"))
        .env("OPENUI_TELEMETRY_DISABLED", "1")
        .env("DO_NOT_TRACK", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let write = child
        .stdin
        .take()
        .ok_or("child stdin")?
        .write_all(request.to_string().as_bytes());
    let out = child.wait_with_output()?;
    write?;
    if !out.status.success() {
        return Err("child supervisor failed".into());
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}
struct Comparison {
    reference: Value,
    actual: Value,
    kind: &'static str,
}
fn compare(c: &Value, root: &Path) -> Result<Comparison> {
    validate(c)?;
    let reference = child(
        root,
        &[
            "bun".into(),
            "--no-install".into(),
            "runtime.ts".into(),
            "fuzz".into(),
        ],
        c,
    )?;
    if reference.get("workerError").is_some() {
        return Ok(Comparison {
            reference,
            actual: Value::Null,
            kind: "reference-error",
        });
    }
    let actual = child(
        root,
        &[
            std::env::current_exe()?.to_string_lossy().into_owned(),
            "--runtime-worker".into(),
        ],
        &json!({"case":c,"saved":get(&reference,"saved")}),
    )?;
    let reference = normalize(&reference)?;
    let kind = if actual.get("workerError").is_some() {
        "runtime-error"
    } else if same(&reference, &actual) {
        "match"
    } else if explained(c, &reference, &actual, root)? {
        "explained"
    } else {
        "mismatch"
    };
    Ok(Comparison {
        reference,
        actual,
        kind,
    })
}
pub fn worker() -> Result<()> {
    let mut input = String::new();
    std::io::stdin()
        .take((MAX_INPUT + 1) as u64)
        .read_to_string(&mut input)?;
    if input.len() > MAX_INPUT {
        return Err("worker input too large".into());
    }
    let request: Value = serde_json::from_str(&input)?;
    println!(
        "{}",
        replay(
            get(&request, "case"),
            get(&request, "saved"),
            &crate::repo_root()
        )?
    );
    Ok(())
}
/// Remove interactions first, then source lines and state entries. Keep the
/// same finding kind; every attempt is independently supervised and replayable.
fn shrink(c: &Value, kind: &str, root: &Path) -> Result<(Value, bool)> {
    let mut small = c.clone();
    let mut attempts = 0;
    for key in ["actionInteractions", "fuzzInput", "stateValues"] {
        let length = match key {
            "actionInteractions" => array(get(&small, key)).len(),
            "fuzzInput" => text(get(&small, key)).lines().count(),
            _ => object(get(&small, key)).count(),
        };
        for index in (0..length).rev() {
            if findings::run().out_of_time() || attempts >= 80 {
                return Ok((small, false));
            }
            let mut candidate = small.clone();
            let map = candidate.as_object_mut().ok_or("shrink case")?;
            match key {
                "actionInteractions" => {
                    if let Some(ops) = map.get_mut(key).and_then(Value::as_array_mut)
                        && index < ops.len()
                    {
                        ops.remove(index);
                    }
                }
                "fuzzInput" => {
                    let lines: Vec<_> = text(get(&small, key))
                        .lines()
                        .enumerate()
                        .filter(|(i, _)| *i != index)
                        .map(|(_, s)| s)
                        .collect();
                    map.insert(key.into(), json!(lines.join("\n")));
                }
                _ => {
                    let keys: Vec<_> = object(get(&small, key)).map(|(k, _)| k.clone()).collect();
                    if let Some(k) = keys.get(index)
                        && let Some(o) = map.get_mut(key).and_then(Value::as_object_mut)
                    {
                        o.shift_remove(k);
                    }
                }
            }
            attempts += 1;
            if compare(&candidate, root)?.kind == kind {
                small = candidate;
            }
        }
    }
    Ok((small, true))
}
pub fn run(
    seed: u64,
    batches: impl Iterator<Item = Vec<u64>>,
    root: &Path,
    replay_path: Option<PathBuf>,
) -> Result<()> {
    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    let mut findings_count = 0;
    let mut steps = 0;
    let replay = replay_path.as_deref().map(load).transpose()?;
    for batch in batches {
        for index in batch {
            if findings::run().out_of_time() {
                break;
            }
            let c = match &replay {
                Some(c) => c.clone(),
                None => generated(seed, index, root)?,
            };
            let command = if let Some(path) = &replay_path {
                let quoted = path.to_string_lossy().replace('\'', "'\\''");
                format!("scripts/fuzz.sh --runtime --replay '{quoted}' --only 0 --no-save")
            } else {
                format!("scripts/fuzz.sh --runtime --seed {seed} --only {index} --no-save")
            };
            findings::run().progress("runtime", &command);
            let compared = compare(&c, root)?;
            *tally.entry(compared.kind).or_default() += 1;
            steps += array(get(&c, "actionInteractions")).len() * 2;
            if !matches!(compared.kind, "match" | "explained") && findings_count < MAX_SAVES {
                findings_count += 1;
                let (small, complete) = if compared.kind == "mismatch" {
                    shrink(&c, compared.kind, root)?
                } else {
                    (c.clone(), false)
                };
                let shrunk = compare(&small, root)?;
                let name = format!("runtime-{}", fingerprint(&small.to_string()));
                let dir = findings::run()
                    .dir
                    .clone()
                    .unwrap_or_else(|| root.join("target/runtime-findings"));
                std::fs::create_dir_all(&dir)?;
                let path = dir.join(format!("{name}.json"));
                std::fs::write(
                    &path,
                    format!("{}\n", serde_json::to_string_pretty(&small)?),
                )?;
                let finding = json!({"check":"runtime","kind":compared.kind,"seed":seed,"index":index,"shrunk":complete,"case":path,"replay":format!("scripts/fuzz.sh --runtime --replay {} --only 0 --no-save",path.display()),"generatedReplay":command,"originalInput":c,"input":small,"reference":shrunk.reference,"rust":shrunk.actual});
                std::fs::write(
                    dir.join(format!("{name}.finding.json")),
                    format!("{}\n", serde_json::to_string_pretty(&finding)?),
                )?;
                findings::run().record(&finding);
                println!(
                    "runtime {} seed {seed} case {index}: {}",
                    compared.kind,
                    path.display()
                );
            }
            if replay.is_some() {
                break;
            }
        }
        if replay.is_some() {
            break;
        }
    }
    let summary = json!({"seed":seed,"inputs":tally.values().sum::<usize>(),"steps":steps,"outcomes":tally,"findings":findings_count});
    println!("runtime fuzz: {summary}");
    findings::run().summary("runtime", &summary);
    if findings_count > 0 {
        return Err("unexplained runtime findings (saved for triage)".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_replay_refuses_missing_handles_instead_of_running_a_default_action() {
        let case = json!({"schema":"runtime-runner", "fuzzInput":"", "actionInteractions":[{"kind":"run","handle":0}]});
        let refusal = replay(&case, &json!({}), &crate::repo_root()).unwrap_err();
        assert_eq!(refusal.to_string(), "run handle not discovered");
    }
    #[test]
    fn generated_runtime_interactions_match_pinned_reference_bytes() {
        let root = crate::repo_root();
        let corpus = load(&conformance(&root).join("runtime-fuzz.json")).unwrap();
        let golden = load(&conformance(&root).join("golden/runtime-fuzz.json")).unwrap();
        let cases = array(get(&corpus, "cases"));
        assert_eq!(cases.len(), 3);
        assert_eq!(cases.len(), array(&golden).len());
        for (index, (case, expected)) in cases.iter().zip(array(&golden)).enumerate() {
            if index < 2 {
                assert!(same(case, &generated(40, index as u64, &root).unwrap()));
            }
            let reference = normalize(expected).unwrap();
            let actual = replay(case, get(expected, "saved"), &root).unwrap();
            if index < 2 {
                assert!(same(&reference, &actual), "{}", get(case, "name"));
            } else {
                assert!(explained(case, &reference, &actual, &root).unwrap());
                let mut changed = actual.clone();
                changed["actions"]["defaults"]["steps"][0]["observation"]["failed"] = json!(false);
                assert!(!explained(case, &reference, &changed, &root).unwrap());
            }
        }
    }
    #[test]
    fn runtime_generation_replays_and_stays_within_bounds() {
        for seed in [1, 40, 57] {
            for index in 0..128 {
                let a = generated(seed, index, &crate::repo_root()).unwrap();
                assert_eq!(
                    a.to_string(),
                    generated(seed, index, &crate::repo_root())
                        .unwrap()
                        .to_string()
                );
                validate(&a).unwrap();
            }
        }
    }
    #[test]
    fn generated_state_covers_json_kinds_with_bounded_shapes() {
        fn check(v: &Value, depth: usize, kinds: &mut [bool; 6]) -> usize {
            assert!(depth <= 3);
            match v {
                Value::Null => {
                    kinds[0] = true;
                    1
                }
                Value::Bool(_) => {
                    kinds[1] = true;
                    1
                }
                Value::Number(_) => {
                    kinds[2] = true;
                    1
                }
                Value::String(s) => {
                    kinds[3] = true;
                    assert!(s.len() <= 24);
                    1
                }
                Value::Array(a) => {
                    kinds[4] = true;
                    assert!(a.len() <= 3);
                    1 + a.iter().map(|v| check(v, depth + 1, kinds)).sum::<usize>()
                }
                Value::Object(o) => {
                    kinds[5] = true;
                    assert!(o.len() <= 3);
                    1 + o
                        .values()
                        .map(|v| check(v, depth + 1, kinds))
                        .sum::<usize>()
                }
            }
        }
        let mut kinds = [false; 6];
        for index in 0..1024 {
            let v = state(&mut Rng::for_program(40, index), 3);
            assert!(check(&v, 0, &mut kinds) <= 40);
        }
        assert!(kinds.into_iter().all(|seen| seen));
    }
    #[test]
    fn runtime_observation_comparison_preserves_object_order_and_flags() {
        assert!(!same(&json!({"a":1,"b":2}), &json!({"b":2,"a":1})));
        let a = json!({"flags":flags()});
        let mut b = a.clone();
        b["flags"]["treeSize"] = json!(true);
        assert!(!same(&a, &b));
        // JSON.stringify writes integral numbers without a fractional suffix.
        assert!(same(&json!({"value": 121}), &json!({"value": 121.0})));
    }
    #[test]
    fn runtime_serialized_observations_preserve_number_and_escape_bytes() {
        let actual = json!({"serialized":{"tree":"{\"root\":121.0,\"errors\":[]}"}});
        let reference = json!({"serialized":{"tree":"{\"root\":121,\"errors\":[]}"}});
        assert!(!same(&actual, &reference));
        let actual = json!({"serialized":{"snapshot":"{\"name\":\"\\u03bb\"}"}});
        let reference = json!({"serialized":{"snapshot":"{\"name\":\"λ\"}"}});
        assert!(!same(&actual, &reference));
        let actual = json!({"serialized":{"events":"[{\"params\":1.0}]"}});
        let reference = json!({"serialized":{"events":"[{\"params\":1}]"}});
        assert!(!same(&actual, &reference));
    }
    #[test]
    fn runtime_case_bounds_refuse_oversized_source_and_sequences() {
        let mut c = json!({"fuzzInput":"root=Box(1)","actionInteractions":[]});
        validate(&c).unwrap();
        c["fuzzInput"] = json!("x".repeat(MAX_SOURCE + 1));
        assert!(validate(&c).is_err());
        c["fuzzInput"] = json!("");
        c["actionInteractions"] = json!(vec![json!({"kind":"redraw"}); MAX_OPS + 1]);
        assert!(validate(&c).is_err());
        c["actionInteractions"] = json!([{"kind":"program", "program":"x".repeat(MAX_SOURCE + 1)}]);
        assert!(validate(&c).is_err());
    }
}
