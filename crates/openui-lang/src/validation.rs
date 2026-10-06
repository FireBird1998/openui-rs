//! Validation messages and component signatures, worded exactly as the
//! reference parser words them.

use serde_json::Value as Json;

use crate::js::json_string;
use crate::library::Param;
use crate::result::ValidationErrorCode;

/// The varying parts of a validation error; the wording lives in [`message`].
pub(crate) enum Issue {
    MissingRequired { signature: String },
    NullRequired { signature: String },
    UnknownComponent { available: Vec<String> },
}

impl Issue {
    pub(crate) fn code(&self) -> ValidationErrorCode {
        match self {
            Issue::MissingRequired { .. } => ValidationErrorCode::MissingRequired,
            Issue::NullRequired { .. } => ValidationErrorCode::NullRequired,
            Issue::UnknownComponent { .. } => ValidationErrorCode::UnknownComponent,
        }
    }

    pub(crate) fn message(&self, component: &str, path: &str) -> String {
        match self {
            Issue::MissingRequired { signature } => {
                format!(
                    "missing required field \"{path}\"{}",
                    signature_hint(signature)
                )
            }
            Issue::NullRequired { signature } => {
                format!(
                    "required field \"{path}\" cannot be null{}",
                    signature_hint(signature)
                )
            }
            Issue::UnknownComponent { available } => {
                let list = if available.is_empty() {
                    String::new()
                } else {
                    format!(". Available components: {}", available.join(", "))
                };
                format!(
                    "Unknown component \"{component}\" — not found in catalog or builtins{list}"
                )
            }
        }
    }
}

fn signature_hint(signature: &str) -> String {
    if signature.is_empty() {
        String::new()
    } else {
        format!(" — signature: {signature}")
    }
}

/// A typed signature such as `Header(title*: string, variant: "a"|"b")`; `*`
/// marks required params.
pub(crate) fn params_signature(component: &str, params: &[Param]) -> String {
    let rendered: Vec<String> = params
        .iter()
        .map(|p| {
            let marked = if p.required {
                format!("{}*", p.name)
            } else {
                p.name.clone()
            };
            match type_from_schema(&p.schema) {
                Some(ty) => format!("{marked}: {ty}"),
                None => marked,
            }
        })
        .collect();
    format!("{component}({})", rendered.join(", "))
}

/// The type hint for a param's schema, as the reference `getTypeFromSchema`.
fn type_from_schema(schema: &Json) -> Option<String> {
    let p = schema.as_object()?;
    if let Some(r) = p.get("$ref").and_then(Json::as_str) {
        return r.rsplit('/').next().map(str::to_owned);
    }
    if let Some(values) = p.get("enum").and_then(Json::as_array) {
        return Some(
            values
                .iter()
                .map(json_literal)
                .collect::<Vec<_>>()
                .join("|"),
        );
    }
    if let Some(value) = p.get("const") {
        return Some(json_literal(value));
    }
    match p.get("type").and_then(Json::as_str) {
        Some("string") => Some("string".to_owned()),
        Some("number" | "integer") => Some("number".to_owned()),
        Some("boolean") => Some("boolean".to_owned()),
        Some(other) => Some(other.to_owned()),
        None => None,
    }
}

/// `JSON.stringify` of an enum or const value. Exact for strings, booleans and
/// null; numbers use serde_json's formatting until the JS number formatter lands.
fn json_literal(value: &Json) -> String {
    match value {
        Json::String(s) => json_string(s),
        other => other.to_string(),
    }
}
