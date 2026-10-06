//! Validation errors with fix-it hints, as the reference parser's deprecated
//! `enrichErrors`.

use serde_json::Value as Json;

use crate::js::{json_entries, truthy, write_json_string};
use crate::result::{ValidationError, ValidationErrorCode};

/// A validation error in the shape the reference's `OpenUIError` takes for
/// parser errors, with a hint where one helps.
#[derive(Clone, Debug, PartialEq)]
pub struct EnrichedError {
    pub code: ValidationErrorCode,
    pub message: String,
    pub component: String,
    /// The prop path; `None` where the error has none.
    pub path: Option<String>,
    pub statement_id: Option<String>,
    pub hint: Option<String>,
}

impl EnrichedError {
    /// The error as the reference's `JSON.stringify` writes it.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\"source\":\"parser\",\"code\":");
        write_json_string(&mut out, self.code.as_str());
        out.push_str(",\"message\":");
        write_json_string(&mut out, &self.message);
        out.push_str(",\"component\":");
        write_json_string(&mut out, &self.component);
        for (key, value) in [
            ("path", &self.path),
            ("statementId", &self.statement_id),
            ("hint", &self.hint),
        ] {
            if let Some(value) = value {
                out.push_str(",\"");
                out.push_str(key);
                out.push_str("\":");
                write_json_string(&mut out, value);
            }
        }
        out.push('}');
        out
    }
}

/// A list of enriched errors as the reference's `JSON.stringify` writes it.
pub fn enriched_errors_to_json(errors: &[EnrichedError]) -> String {
    let items: Vec<String> = errors.iter().map(EnrichedError::to_json).collect();
    format!("[{}]", items.join(","))
}

/// Add hints to validation errors, as the reference's `enrichErrors`:
/// unknown components list `component_names`, missing or null required props
/// show the component's signature from `schema` (the library's JSON Schema),
/// and inline Query/Mutation calls say to declare them as statements.
pub fn enrich_errors(
    errors: &[ValidationError],
    schema: &Json,
    component_names: &[String],
) -> Vec<EnrichedError> {
    errors
        .iter()
        .map(|e| {
            let hint = match e.code {
                ValidationErrorCode::UnknownComponent if !component_names.is_empty() => Some(
                    format!("Available components: {}", component_names.join(", ")),
                ),
                ValidationErrorCode::MissingRequired | ValidationErrorCode::NullRequired => {
                    signature_hint(&e.component, schema)
                }
                ValidationErrorCode::InlineReserved => Some(format!(
                    "Declare as a top-level statement: myVar = {}(...)",
                    e.component
                )),
                _ => None,
            };
            EnrichedError {
                code: e.code,
                message: e.message.clone(),
                component: e.component.clone(),
                path: (!e.path.is_empty()).then(|| e.path.clone()),
                statement_id: e.statement_id.clone(),
                hint,
            }
        })
        .collect()
}

/// `$defs[name]` the way JavaScript reads it from parsed JSON: an own key of
/// an object, or an index of an array.
fn member<'a>(value: Option<&'a Json>, name: &str) -> Option<&'a Json> {
    match value? {
        Json::Object(map) => map.get(name),
        Json::Array(items) => {
            let i: usize = name.parse().ok()?;
            (i.to_string() == name).then(|| items.get(i)).flatten()
        }
        _ => None,
    }
}

/// `Signature: Header(title*, subtitle) — * marks required`, from the
/// component's own schema; `None` without properties.
fn signature_hint(component: &str, schema: &Json) -> Option<String> {
    let def = member(member(Some(schema), "$defs"), component)?;
    let Json::Object(def) = def else { return None };
    let properties = def.get("properties").filter(|p| truthy(Some(p)))?;
    // `new Set(required ?? [])`: an array's elements, or a string's code
    // points. (Anything else makes the reference throw; it counts as none here.)
    let required: Vec<String> = match def.get("required") {
        Some(Json::Array(items)) => items
            .iter()
            .filter_map(|r| r.as_str().map(str::to_owned))
            .collect(),
        Some(Json::String(s)) => s.chars().map(String::from).collect(),
        _ => Vec::new(),
    };
    // `Object.keys(properties)`.
    let keys: Vec<String> = match properties {
        Json::Object(map) => json_entries(map)
            .into_iter()
            .map(|(k, _)| k.clone())
            .collect(),
        Json::Array(items) => (0..items.len()).map(|i| i.to_string()).collect(),
        Json::String(s) => (0..s.encode_utf16().count())
            .map(|i| i.to_string())
            .collect(),
        _ => Vec::new(),
    };
    let params: Vec<String> = keys
        .into_iter()
        .map(|k| {
            if required.contains(&k) {
                format!("{k}*")
            } else {
                k
            }
        })
        .collect();
    Some(format!(
        "Signature: {component}({}) — * marks required",
        params.join(", ")
    ))
}
