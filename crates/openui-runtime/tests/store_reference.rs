//! Replay the pinned upstream store through the public runtime interface.
use std::{collections::BTreeMap, error::Error, sync::Arc};

use openui_lang::{JsObject, free_json, parse_json};
use openui_runtime::{StateValue, Store, Value};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn array(value: &Value) -> TestResult<&[Value]> {
    if let Value::Arr(v) = value {
        Ok(v)
    } else {
        Err("expected array".into())
    }
}
fn text(value: &Value) -> TestResult<&str> {
    if let Value::Str(v) = value {
        Ok(v)
    } else {
        Err("expected string".into())
    }
}
fn index(value: &Value) -> TestResult<usize> {
    if let Value::Num(n) = value
        && n.is_finite()
        && *n >= 0.0
        && *n < usize::MAX as f64
        && n.fract() == 0.0
    {
        Ok(*n as usize)
    } else {
        Err("expected index".into())
    }
}
fn boolean(value: &Value) -> TestResult<bool> {
    if let Value::Bool(v) = value {
        Ok(*v)
    } else {
        Err("expected boolean".into())
    }
}
fn at(values: &[Value], i: usize) -> TestResult<&Value> {
    values.get(i).ok_or_else(|| "missing item".into())
}
fn field<'a>(value: &'a Value, key: &str) -> TestResult<&'a Value> {
    if let Value::Obj(o) = value {
        o.get(key).ok_or_else(|| format!("missing {key}").into())
    } else {
        Err("expected object".into())
    }
}

// Observation positions are declared in the fixture header, keeping each call
// and all its expected observations on one compact line.
fn observation<'a>(value: &'a Value, name: &str) -> TestResult<&'a Value> {
    let position = match name {
        "changed" => 0,
        "notified" => 1,
        "snapshotChanged" => 2,
        "mappedVersion" => 3,
        "snapshot" => 4,
        "pristine" => 5,
        "read" => 6,
        _ => return Err("unknown observation".into()),
    };
    at(array(value)?, position)
}

fn decode(
    i: usize,
    definitions: &[Value],
    cache: &mut BTreeMap<usize, StateValue>,
) -> TestResult<StateValue> {
    if let Some(value) = cache.get(&i) {
        return Ok(value.clone());
    }
    let definition = array(at(definitions, i)?)?;
    let value = if definition.get(2).map(text).transpose()? == Some("ast") {
        Some(Value::Ast(Arc::new(openui_lang::parse_expression(
            &openui_lang::tokenize(text(at(definition, 3)?)?),
        ))))
    } else {
        match text(at(definition, 0)?)? {
            "undefined" => None,
            "null" => Some(Value::Null),
            "boolean" | "string" => Some(at(definition, 1)?.clone()),
            "number" => Some(Value::Num(text(at(definition, 1)?)?.parse()?)),
            "array" => {
                let items = array(at(definition, 1)?)?
                    .iter()
                    .map(|v| {
                        decode(index(v)?, definitions, cache)?
                            .ok_or_else(|| "nested undefined is outside compared sequences".into())
                    })
                    .collect::<TestResult<Vec<_>>>()?;
                Some(Value::Arr(Arc::new(items)))
            }
            "object" => {
                let mut object = JsObject::new();
                for entry in array(at(definition, 1)?)? {
                    let entry = array(entry)?;
                    object.define(
                        text(at(entry, 0)?)?,
                        decode(index(at(entry, 1)?)?, definitions, cache)?
                            .ok_or("nested undefined is outside compared sequences")?,
                    );
                }
                Some(Value::Obj(Arc::new(object)))
            }
            other => return Err(format!("unknown value {other}").into()),
        }
    };
    cache.insert(i, value.clone());
    Ok(value)
}

fn map(value: &Value, pool: &BTreeMap<usize, StateValue>) -> TestResult<JsObject<StateValue>> {
    let mut map = JsObject::new();
    for entry in array(value)? {
        let entry = array(entry)?;
        map.define(text(at(entry, 0)?)?, pooled(at(entry, 1)?, pool)?);
    }
    Ok(map)
}
fn entries(
    value: &Value,
    pool: &BTreeMap<usize, StateValue>,
) -> TestResult<Vec<(String, StateValue)>> {
    array(value)?
        .iter()
        .map(|entry| {
            let pair = array(entry)?;
            Ok((text(at(pair, 0)?)?.to_owned(), pooled(at(pair, 1)?, pool)?))
        })
        .collect()
}
fn pooled(value: &Value, pool: &BTreeMap<usize, StateValue>) -> TestResult<StateValue> {
    pool.get(&index(value)?)
        .cloned()
        .ok_or_else(|| "unknown value id".into())
}
fn apply(store: &mut Store, step: &Value, pool: &BTreeMap<usize, StateValue>) -> TestResult<bool> {
    match text(field(step, "op")?)? {
        "set" | "read-set" => {
            let name = text(field(step, "name")?)?;
            let value = if text(field(step, "op")?)? == "set" {
                pooled(field(step, "value")?, pool)?
            } else {
                store.get(name)
            };
            let pristine = match step {
                Value::Obj(o) => o.get("pristine").map(boolean).transpose()?.unwrap_or(false),
                _ => false,
            };
            Ok(if pristine {
                store.set_pristine(name, value)?
            } else {
                store.set(name, value)?
            })
        }
        "initialize" => {
            let defaults = entries(field(step, "defaults")?, pool)?;
            let persisted = entries(field(step, "persisted")?, pool)?;
            let result = store.initialize(
                defaults.iter().map(|(k, v)| (k.as_str(), v.as_ref())),
                persisted.iter().map(|(k, v)| (k.as_str(), v.as_ref())),
            );
            assert!(result.refused.is_empty(), "unexpected refusals: {result:?}");
            Ok(result.changed)
        }
        "read" => {
            store.get(text(field(step, "name")?)?);
            Ok(false)
        }
        "dispose" => Ok(store.dispose()),
        other => Err(format!("unknown operation {other}").into()),
    }
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

type ContainerKey = (u8, usize);
fn container_key(value: &Value) -> Option<ContainerKey> {
    match value {
        Value::Arr(v) => Some((0, Arc::as_ptr(v) as usize)),
        Value::Obj(v) => Some((1, Arc::as_ptr(v) as usize)),
        Value::Ast(v) => Some((2, Arc::as_ptr(v) as usize)),
        Value::Element(v) => Some((3, Arc::as_ptr(v) as usize)),
        _ => None,
    }
}

// Reference ids are the fixture's pool ids. Keep actual bodies alive so allocator
// address reuse cannot masquerade as an identity change or an alias.
struct Identities {
    reference_ids: BTreeMap<ContainerKey, usize>,
    actual_to_reference: BTreeMap<ContainerKey, usize>,
    reference_to_actual: BTreeMap<usize, ContainerKey>,
    held: Vec<Value>,
}
impl Identities {
    fn new(pool: &BTreeMap<usize, StateValue>) -> Self {
        Self {
            reference_ids: pool
                .iter()
                .filter_map(|(id, v)| v.as_ref().and_then(container_key).map(|key| (key, *id)))
                .collect(),
            actual_to_reference: BTreeMap::new(),
            reference_to_actual: BTreeMap::new(),
            held: Vec::new(),
        }
    }
    fn compare(&mut self, actual: Option<&Value>, reference: Option<&Value>) -> Result<(), String> {
        if let (Some(a), Some(r)) = (actual, reference)
            && let (Some(ak), Some(rk)) = (container_key(a), container_key(r))
        {
            let id = *self
                .reference_ids
                .get(&rk)
                .ok_or("unknown reference container")?;
            if let Some(previous) = self.actual_to_reference.get(&ak) {
                return if *previous == id {
                    Ok(())
                } else {
                    Err(format!(
                        "actual container {ak:?} maps to reference {previous}, not {id}"
                    ))
                };
            }
            if let Some(previous) = self.reference_to_actual.get(&id) {
                return Err(format!(
                    "reference container {id} maps to actual {previous:?}, not {ak:?}"
                ));
            }
            self.actual_to_reference.insert(ak, id);
            self.reference_to_actual.insert(id, ak);
            self.held.push(a.clone());
            match (a, r) {
                (Value::Arr(a), Value::Arr(r)) if a.len() == r.len() => {
                    for (a, r) in a.iter().zip(r.iter()) {
                        self.compare(Some(a), Some(r))?;
                    }
                    return Ok(());
                }
                (Value::Obj(a), Value::Obj(r)) if a.len() == r.len() => {
                    for ((ak, a), (rk, r)) in a.iter().zip(r.iter()) {
                        if ak != rk {
                            return Err(format!("object key diff: {ak} != {rk}"));
                        }
                        self.compare(Some(a), Some(r))?;
                    }
                    return Ok(());
                }
                (Value::Ast(a), Value::Ast(r)) if a == r => return Ok(()),
                (Value::Element(a), Value::Element(r)) if a == r => return Ok(()),
                _ => return Err(format!("container content diff: {a:?} != {r:?}")),
            }
        }
        if same_handle(&actual.cloned(), &reference.cloned()) {
            Ok(())
        } else {
            Err(format!("scalar diff: {actual:?} != {reference:?}"))
        }
    }
}

#[test]
fn seeded_and_recorded_sequences_match_upstream_at_every_step() -> TestResult {
    replay(false)
}

#[test]
fn synthetic_streaming_pristine_defaults_match_upstream() -> TestResult {
    replay(true)
}

fn replay(synthetic_only: bool) -> TestResult {
    let json = parse_json(include_str!("store.json"))?;
    let fixture = Value::from_json(&json);
    free_json(json);
    assert_eq!(
        text(field(&fixture, "versionMapping")?)?,
        "upstream notifications plus nonempty dispose"
    );
    assert_eq!(
        array(field(&fixture, "observations")?)?
            .iter()
            .map(text)
            .collect::<TestResult<Vec<_>>>()?,
        [
            "changed",
            "notified",
            "snapshotChanged",
            "mappedVersion",
            "snapshot",
            "pristine",
            "read"
        ]
    );
    let cases = array(field(&fixture, "sequences")?)?;
    for case in cases {
        let synthetic = if let Value::Obj(o) = case {
            o.get("synthetic").map(text).transpose()?
        } else {
            None
        };
        if synthetic.is_some() != synthetic_only {
            continue;
        }
        let name = text(field(case, "name")?)?;
        let definitions = array(field(case, "pool")?)?;
        let mut pool = BTreeMap::new();
        for i in 0..definitions.len() {
            decode(i, definitions, &mut pool)?;
        }
        let steps = array(field(case, "steps")?)?;
        let snapshots = array(field(case, "snapshots")?)?;
        let mut store = Store::new();
        let mut identities = Identities::new(&pool);
        let mut replacements = std::collections::BTreeSet::new();
        let mut user_set = false;
        let mut user_followups = 0usize;
        let mut disappeared = false;
        let mut returned = false;
        let mut parser = None;
        if let Value::Obj(o) = case
            && let Some(schema_file) = o.get("schema")
        {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
            let schema = parse_json(&std::fs::read_to_string(root.join(text(schema_file)?))?)?;
            parser = Some(openui_lang::StreamParser::new(
                Arc::new(openui_lang::Library::from_json_schema(&schema)),
                None,
            ));
            free_json(schema);
            if let Some(recording) = o.get("recording") {
                let mut declares_null = false;
                for step in steps {
                    if let Value::Obj(o) = step
                        && let Some(defaults) = o.get("defaults")
                    {
                        for entry in array(defaults)? {
                            let entry = array(entry)?;
                            assert_eq!(
                                text(at(entry, 0)?)?,
                                "$period",
                                "recordings have only the null binding default"
                            );
                            assert!(
                                matches!(pooled(at(entry, 1)?, &pool)?, Some(Value::Null)),
                                "recorded default must be null"
                            );
                            declares_null = true;
                        }
                    }
                }
                assert_eq!(
                    declares_null,
                    text(recording)?.starts_with("batch-104-"),
                    "only the museum recording declares state"
                );
                let json = parse_json(&std::fs::read_to_string(
                    root.join("recordings").join(text(recording)?),
                )?)?;
                let recording = Value::from_json(&json);
                free_json(json);
                let recorded_chunks = array(field(&recording, "chunks")?)?
                    .iter()
                    .map(|c| text(field(c, "text")?))
                    .collect::<TestResult<Vec<_>>>()?;
                let fixture_chunks = steps
                    .iter()
                    .filter_map(|s| {
                        if let Value::Obj(o) = s {
                            o.get("chunk")
                        } else {
                            None
                        }
                    })
                    .map(text)
                    .collect::<TestResult<Vec<_>>>()?;
                assert_eq!(
                    fixture_chunks, recorded_chunks,
                    "actual recorded chunks: {name}"
                );
            }
        }
        for (i, step) in steps.iter().enumerate() {
            let expected = field(step, "expected")?;
            let context = format!(
                "sequence {name}, step {i}: {step:?}\nsequence: {steps:?}\nexpected: {expected:?}"
            );
            let before = store.snapshot();
            let pristine_before: Vec<_> = before
                .iter()
                .filter(|(k, _)| store.is_pristine(k))
                .map(|(k, _)| k.clone())
                .collect();
            let changed = if let Some(parser) = parser.as_mut()
                && let Value::Obj(o) = step
                && (o.has_own("chunk") || o.has_own("source"))
            {
                let pushed = if let Some(source) = o.get("source") {
                    parser.set(text(source)?)
                } else {
                    let chunk = text(field(step, "chunk")?)?;
                    if synthetic.is_some() {
                        assert_eq!(chunk.len(), 1, "byte chunk: {context}");
                    }
                    parser.push(chunk)
                };
                let parsed = if boolean(field(step, "final")?)? {
                    parser.finish()
                } else {
                    pushed
                };
                assert!(!parsed.size_limited && !parsed.depth_limited, "{context}");
                let expected_defaults = map(field(step, "defaults")?, &pool)?;
                assert_eq!(
                    parsed
                        .state_declarations
                        .iter()
                        .map(|(k, _)| k)
                        .collect::<Vec<_>>(),
                    expected_defaults.iter().map(|(k, _)| k).collect::<Vec<_>>(),
                    "parsed declaration keys\n{context}"
                );
                // Inputs that a non-pristine name ignores are not store values.
                // Check their content and aliases within this parse result;
                // maintain temporal identity mapping for snapshots/reads below.
                let mut declaration_identities = Identities::new(&pool);
                for ((_, actual), (_, reference)) in parsed
                    .state_declarations
                    .iter()
                    .zip(expected_defaults.iter())
                {
                    declaration_identities
                        .compare(Some(actual), reference.as_ref())
                        .map_err(|diff| format!("parsed declarations: {diff}\n{context}"))?;
                }
                if synthetic == Some("rewind")
                    && before.has_own("$gone")
                    && !parsed.state_declarations.has_own("$gone")
                {
                    disappeared = true;
                }
                if disappeared && parsed.state_declarations.has_own("$gone") {
                    returned = true;
                }
                if user_set {
                    user_followups = user_followups.saturating_add(1);
                }
                let persisted = map(field(step, "persisted")?, &pool)?;
                let result = store.initialize(
                    parsed
                        .state_declarations
                        .iter()
                        .map(|(k, v)| (k.as_str(), Some(v))),
                    persisted.iter().map(|(k, v)| (k.as_str(), v.as_ref())),
                );
                assert!(result.refused.is_empty(), "{context}");
                result.changed
            } else {
                apply(&mut store, step, &pool)?
            };
            assert_eq!(
                changed,
                boolean(observation(expected, "changed")?)?,
                "{context}"
            );
            assert_eq!(
                store.version(),
                index(observation(expected, "mappedVersion")?)? as u64,
                "mapped version diff\n{context}"
            );
            let op = text(field(step, "op")?)?;
            assert_eq!(
                index(observation(expected, "notified")?)?,
                usize::from(changed && op != "dispose"),
                "notification mapping\n{context}"
            );
            let snapshot = store.snapshot();
            let expected_snapshot = map(
                at(snapshots, index(observation(expected, "snapshot")?)?)?,
                &pool,
            )?;
            assert_eq!(
                snapshot.iter().map(|(k, _)| k).collect::<Vec<_>>(),
                expected_snapshot.iter().map(|(k, _)| k).collect::<Vec<_>>(),
                "snapshot key diff\n{context}"
            );
            for ((key, actual), (_, expected_value)) in
                snapshot.iter().zip(expected_snapshot.iter())
            {
                identities
                    .compare(actual.as_ref(), expected_value.as_ref())
                    .map_err(|diff| format!("snapshot {key}: {diff}\n{context}"))?;
                if pristine_before.contains(key)
                    && !same_handle(&before.get(key).cloned().flatten(), actual)
                {
                    let kind = match actual {
                        Some(Value::Str(_)) => "string",
                        Some(Value::Arr(_)) => "array",
                        Some(Value::Obj(_)) => "object",
                        _ => "scalar",
                    };
                    replacements.insert(kind);
                }
            }
            assert_eq!(
                !Arc::ptr_eq(&before, &snapshot),
                boolean(observation(expected, "snapshotChanged")?)?,
                "snapshot identity diff\n{context}"
            );
            let read_name = match step {
                Value::Obj(o) => o.get("name").map(text).transpose()?.unwrap_or("$missing"),
                _ => "$missing",
            };
            identities
                .compare(
                    store.get(read_name).as_ref(),
                    pooled(observation(expected, "read")?, &pool)?.as_ref(),
                )
                .map_err(|diff| format!("read: {diff}\n{context}"))?;
            if synthetic.is_some() && text(field(step, "op")?)? == "set" {
                user_set = true;
            }
            let actual_pristine: Vec<_> = snapshot
                .iter()
                .filter(|(k, _)| store.is_pristine(k))
                .map(|(k, _)| k.as_str())
                .collect();
            let expected_pristine = array(observation(expected, "pristine")?)?
                .iter()
                .map(text)
                .collect::<TestResult<Vec<_>>>()?;
            assert_eq!(
                actual_pristine, expected_pristine,
                "pristine diff\n{context}"
            );
        }
        if let Some(kind) = synthetic {
            assert!(
                user_set && user_followups > 0,
                "user edit must precede later initialization: {name}"
            );
            if kind == "object" || kind == "array" {
                assert!(
                    matches!(store.get("$edit"),Some(Value::Str(ref s)) if s=="user"),
                    "user edit did not survive: {name}"
                );
                let container = if kind == "object" { "object" } else { "array" };
                assert!(
                    replacements.contains(container) && replacements.contains("string"),
                    "missing pristine {container}/string replacements: {name}: {replacements:?}"
                );
                let default_name = if kind == "object" {
                    "$config"
                } else {
                    "$items"
                };
                assert!(
                    store.is_pristine(default_name),
                    "completed default must remain pristine: {name}"
                );
                if kind == "array" {
                    assert!(
                        matches!(store.get("$twice"),Some(Value::Num(n)) if n==14.0),
                        "expression default: {name}"
                    );
                    assert!(
                        matches!(store.get("$items"),Some(Value::Arr(ref items)) if items.len()==3 && items.iter().zip([1.0,2.0,3.0]).all(|(v,n)|matches!(v,Value::Num(x) if *x==n))),
                        "completed array default: {name}"
                    );
                }
            } else {
                assert!(
                    matches!(store.get("$rows"),Some(Value::Arr(ref items)) if items.len()==1 && matches!(items.first(),Some(Value::Str(s)) if s=="user")),
                    "user edit did not survive: {name}"
                );
                assert!(
                    replacements.contains("array"),
                    "missing pristine array replacement: {name}"
                );
                assert!(
                    disappeared && returned && store.snapshot().has_own("$gone"),
                    "declaration must disappear and return: {name}"
                );
            }
        }
    }
    // Both test selections validate the complete fixture's documentation count.
    let step_count = cases
        .iter()
        .map(|c| array(field(c, "steps")?).map(|s| s.len()))
        .collect::<TestResult<Vec<_>>>()?
        .into_iter()
        .fold(0usize, usize::saturating_add);
    let report = format!(
        "Store: {} sequences, {step_count} steps, no pending sequences.",
        cases.len()
    );
    assert!(
        include_str!("../../../CONFORMANCE.md").contains(&report),
        "CONFORMANCE.md must contain {report}"
    );
    Ok(())
}

#[test]
fn nested_undefined_normalization_can_suppress_one_store_change() -> TestResult {
    let json = parse_json(include_str!("store.json"))?;
    let fixture = Value::from_json(&json);
    free_json(json);
    let divergence = field(&fixture, "divergence")?;
    assert_eq!(
        text(field(divergence, "name")?)?,
        "nested-undefined-is-normalized-before-the-store"
    );
    assert!(boolean(field(field(divergence, "upstream")?, "changed")?)?);
    struct Scope;
    impl openui_runtime::EvaluationScope for Scope {
        fn state(&self, _: &str, _: &openui_runtime::ExpressionEvaluator<'_>) -> StateValue {
            Some(Value::Obj(Arc::new(JsObject::new())))
        }
    }
    let ast = openui_lang::parse_expression(&openui_lang::tokenize(text(field(
        divergence,
        "expression",
    )?)?));
    let evaluated = openui_runtime::evaluate_expression(&ast, &Scope);
    assert!(!evaluated.size_limited && !evaluated.depth_limited);
    let mut store = Store::new();
    store.set("$f", Some(Value::Obj(Arc::new(JsObject::new()))))?;
    let version = store.version();
    let changed = store.set("$f", evaluated.value)?;
    assert_eq!(
        changed,
        boolean(field(field(divergence, "normalized")?, "changed")?)?
    );
    assert!(!changed);
    assert_eq!(store.version(), version);
    let mut result = openui_lang::ParseResult::default();
    result
        .state_declarations
        .define("$f", store.get("$f").ok_or("$f missing")?);
    let json = parse_json(&result.to_json())?;
    let snapshot_json =
        openui_lang::stringify_json(json.get("stateDeclarations").ok_or("snapshot JSON")?);
    free_json(json);
    assert_eq!(
        snapshot_json,
        text(field(field(divergence, "normalized")?, "json")?)?
    );
    assert_eq!(
        snapshot_json,
        text(field(field(divergence, "upstream")?, "json")?)?
    );
    assert!(include_str!("../../../CONFORMANCE.md").contains("Nested undefined in runtime state"));
    Ok(())
}
