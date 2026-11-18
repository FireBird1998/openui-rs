use openui_lang::{Limits, ResponseBundleError, response_bundle_json_utf16};
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

pub fn observe(serialized: String, revisions: &mut Vec<String>) -> String {
    let Some((prefix, wire_revision)) = serialized.rsplit_once(",\"scriptRevision\":") else {
        return serialized;
    };
    // The revision field must be a JSON string, but its bytes are opaque.
    let Some(revision) = wire_revision
        .strip_suffix('}')
        .and_then(|text| serde_json::from_str::<String>(text).ok())
    else {
        return serialized;
    };
    let id = match revisions.iter().position(|known| known == &revision) {
        Some(id) => id,
        None => {
            let id = revisions.len();
            revisions.push(revision);
            id
        }
    };
    format!("{prefix},\"scriptRevision\":{id}}}")
}

pub fn read(
    input: Option<&[u16]>,
    revisions: &mut Vec<String>,
) -> Result<Value, ResponseBundleError> {
    read_with(input, revisions, |input, streaming| {
        response_bundle_json_utf16(input, streaming, Limits::DEFAULT)
    })
}

pub fn read_with(
    input: Option<&[u16]>,
    revisions: &mut Vec<String>,
    parse: impl Fn(&[u16], bool) -> Result<String, ResponseBundleError>,
) -> Result<Value, ResponseBundleError> {
    let input = input.unwrap_or_default();
    let full = observe(parse(input, false)?, revisions);
    let mut cuts = Vec::with_capacity(input.len() + 1);
    for cut in 0..=input.len() {
        if let Some(prefix) = input.get(..cut) {
            let bundle = parse(prefix, true)?;
            cuts.push(observe(bundle, revisions));
        }
    }
    Ok(json!({"complete": full, "cuts": cuts}))
}
