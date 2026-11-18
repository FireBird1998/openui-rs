//! Pure extraction of tool call outcomes from host MCP response JSON.

use std::fmt;

use crate::{LimitError, Limits, Value, admission};
use openui_lang::{JsonError, normalize_js_string, parse_json_value};

/// The outcome of an MCP tool call, as reported by its response.
#[derive(Clone, Debug, PartialEq)]
pub enum McpToolOutcome {
    /// The extracted value, including null when the tool provides no result.
    Result(Value),
    /// The tool reported an error.
    ToolError {
        /// The reference's raw tool error text, including its fallback.
        message: String,
    },
}

/// A refusal to extract an MCP tool call outcome.
#[derive(Debug)]
pub enum McpResponseError {
    /// The host response could not be read as JSON.
    InvalidJson(JsonError),
    /// A response shape or text coercion would throw a JavaScript TypeError.
    InvalidResponse {
        /// An engine-independent description of the failure.
        reason: &'static str,
    },
    /// A supported response crossed a work, depth or size bound.
    Limit(LimitError),
}

impl fmt::Display for McpResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => error.fmt(f),
            Self::InvalidResponse { reason } => f.write_str(reason),
            Self::Limit(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for McpResponseError {}

/// Extract data or the tool-reported error text from MCP callTool response JSON.
///
/// The result-size ceiling and work upper bound each cap the actual UTF-8
/// bytes read and the joined text built. Each open JSON object or array counts
/// toward the depth limit, including in content ignored by the extractor and
/// in JSON parsed from text. The returned data or error text additionally has
/// to fit as JSON under the same byte bounds.
pub fn extract_mcp_tool_result(
    response_json: &str,
    limits: Limits,
) -> Result<McpToolOutcome, McpResponseError> {
    let byte_limit = limits.result_size().min(limits.work_for(usize::MAX));
    if response_json.len() > byte_limit {
        return Err(size_error());
    }
    let response = parse_json_value(response_json, limits.depth()).map_err(|error| {
        if error.depth_limited() {
            McpResponseError::Limit(admission::depth_error())
        } else {
            McpResponseError::InvalidJson(error)
        }
    })?;

    if matches!(response, Value::Null) {
        return Err(invalid_response("cannot read isError from null"));
    }
    let is_error = match property(&response, "isError") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Num(value)) => *value != 0.0 && !value.is_nan(),
        Some(Value::Str(value)) => !value.is_empty(),
        Some(_) => true,
    };

    if !is_error
        && let Some(value) = property(&response, "structuredContent")
        && !matches!(value, Value::Null)
    {
        return result(value.clone(), byte_limit);
    }

    let parts = match property(&response, "content") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Arr(parts)) => parts.as_slice(),
        Some(_) => return Err(invalid_response("content.filter is not a function")),
    };
    // JavaScript completes filter before coercing any of the selected parts.
    if parts.iter().any(|part| matches!(part, Value::Null)) {
        return Err(invalid_response("cannot read a text part's type from null"));
    }
    let mut joined = String::new();
    let mut has_text = false;
    for part in parts {
        if !matches!(property(part, "type"), Some(Value::Str(kind)) if kind == "text") {
            continue;
        }
        if is_error && has_text {
            append(&mut joined, "\n", byte_limit)?;
        }
        has_text = true;
        if let Some(text) = property(part, "text") {
            append_text(&mut joined, text, byte_limit)?;
        }
    }
    let joined = normalize_js_string(joined);

    if is_error {
        let message = if joined.is_empty() {
            "Unknown error".to_owned()
        } else {
            joined
        };
        if openui_lang::json_string_len(&message).saturating_add(2) > byte_limit {
            return Err(size_error());
        }
        return Ok(McpToolOutcome::ToolError { message });
    }
    if !has_text {
        return result(Value::Null, byte_limit);
    }

    let value = match parse_json_value(&joined, limits.depth()) {
        Ok(value) => value,
        Err(error) if error.depth_limited() => {
            return Err(McpResponseError::Limit(admission::depth_error()));
        }
        Err(_) => Value::Str(joined),
    };
    result(value, byte_limit)
}

fn result(value: Value, byte_limit: usize) -> Result<McpToolOutcome, McpResponseError> {
    openui_lang::write_host_json(&value, byte_limit, None).ok_or_else(size_error)?;
    Ok(McpToolOutcome::Result(value))
}

fn property<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Obj(fields) => fields.get(key),
        _ => None,
    }
}

fn size_error() -> McpResponseError {
    McpResponseError::Limit(admission::size_error())
}

fn invalid_response(reason: &'static str) -> McpResponseError {
    McpResponseError::InvalidResponse { reason }
}

fn append(out: &mut String, text: &str, limit: usize) -> Result<(), McpResponseError> {
    if text.len() > limit.saturating_sub(out.len()) {
        return Err(size_error());
    }
    out.push_str(text);
    Ok(())
}

// Array.join coerces its elements recursively and prints null as empty text,
// both at the top level and in nested arrays. Keep the walk off the call stack.
fn append_text(out: &mut String, value: &Value, limit: usize) -> Result<(), McpResponseError> {
    let mut arrays = Vec::new();
    let mut next = Some(value);
    loop {
        if let Some(value) = next {
            match value {
                Value::Null => {}
                Value::Bool(value) => append(out, if *value { "true" } else { "false" }, limit)?,
                Value::Num(value) => append(out, &openui_lang::number_to_string(*value), limit)?,
                Value::Str(text) => append(out, text, limit)?,
                Value::Arr(items) => arrays.push(items.iter().enumerate()),
                Value::Obj(fields) if !fields.has_own("toString") => {
                    append(out, "[object Object]", limit)?;
                }
                _ => {
                    return Err(invalid_response(
                        "cannot convert a text part to a primitive",
                    ));
                }
            }
        }
        let Some(items) = arrays.last_mut() else {
            return Ok(());
        };
        next = if let Some((index, item)) = items.next() {
            if index != 0 {
                append(out, ",", limit)?;
            }
            Some(item)
        } else {
            arrays.pop();
            None
        };
    }
}
