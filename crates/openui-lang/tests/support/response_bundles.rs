use openui_lang::{
    Limits, ResponseBundle, ResponseBundleError, ScriptRevision, parse_response_bundle,
    parse_response_bundle_utf16,
};
use serde_json::{Value, json};

pub const CASES: &str = include_str!("../../../../conformance/response-bundles.json");

pub fn units(value: &Value) -> Option<Vec<u16>> {
    if let Some(raw) = value.as_str() {
        Some(raw.encode_utf16().collect())
    } else {
        value
            .as_array()?
            .iter()
            .map(|v| u16::try_from(v.as_u64()?).ok())
            .collect()
    }
}

pub fn complete(input: Option<&[u16]>) -> Result<ResponseBundle, ResponseBundleError> {
    let Some(input) = input else {
        return parse_response_bundle(None, false, Limits::DEFAULT);
    };
    match String::from_utf16(input) {
        Ok(text) => parse_response_bundle(Some(&text), false, Limits::DEFAULT),
        Err(_) => parse_response_bundle_utf16(input, false, Limits::DEFAULT),
    }
}

pub fn observe(bundle: &ResponseBundle, revisions: &mut Vec<ScriptRevision>) -> String {
    let serialized = bundle.to_json();
    let Some((prefix, wire_revision)) = serialized.rsplit_once(",\"scriptRevision\":") else {
        return serialized;
    };
    let Some(revision) = &bundle.script_revision else {
        return serialized;
    };
    // The revision field must be a JSON string, but its bytes are opaque.
    if !wire_revision
        .strip_suffix('}')
        .is_some_and(|text| serde_json::from_str::<String>(text).is_ok())
    {
        return serialized;
    }
    let id = match revisions.iter().position(|known| known == revision) {
        Some(id) => id,
        None => {
            revisions.push(revision.clone());
            revisions.len() - 1
        }
    };
    format!("{prefix},\"scriptRevision\":{id}}}")
}

pub fn read(
    input: Option<&[u16]>,
    revisions: &mut Vec<ScriptRevision>,
) -> Result<Value, ResponseBundleError> {
    let full = observe(&complete(input)?, revisions);
    let input = input.unwrap_or_default();
    let mut cuts = Vec::with_capacity(input.len() + 1);
    for cut in 0..=input.len() {
        if let Some(prefix) = input.get(..cut) {
            let bundle = parse_response_bundle_utf16(prefix, true, Limits::DEFAULT)?;
            cuts.push(observe(&bundle, revisions));
        }
    }
    Ok(json!({"complete": full, "cuts": cuts}))
}
