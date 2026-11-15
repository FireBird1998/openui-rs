//! Bounded admission for configuration and serialization, without evaluation.
use crate::{EvaluationFlags, EvaluationSession, Limits, Value};

/// A work, depth or size refusal outside expression evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimitError {
    /// The same cut flags used by the core evaluator.
    pub flags: EvaluationFlags,
}
impl std::fmt::Display for LimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.flags.depth_limited {
            "depth limit exceeded"
        } else {
            "work or size limit exceeded"
        })
    }
}
impl std::error::Error for LimitError {}

pub(crate) fn size_error() -> LimitError {
    let mut flags = EvaluationFlags::default();
    flags.size_limited = true;
    LimitError { flags }
}
pub(crate) fn fixed(limits: Limits) -> Limits {
    limits
        .with_work(limits.work_for(usize::MAX))
        .with_result_size(limits.result_size())
}
pub(crate) fn measure(value: &Value, limits: Limits) -> Result<usize, LimitError> {
    let session = EvaluationSession::with_limits(&(), 0, fixed(limits));
    session
        .with_evaluator(|e| e.reserve_value_size(value))
        .ok_or(LimitError {
            flags: session.flags(),
        })
}
pub(crate) fn text(text: &str, limits: Limits) -> Result<usize, LimitError> {
    if text.len() > limits.work_for(usize::MAX) || text.len() > limits.result_size() {
        return Err(size_error());
    }
    let bytes = openui_lang::json_string_len(text).saturating_add(2);
    if bytes > limits.result_size() {
        Err(size_error())
    } else {
        Ok(bytes)
    }
}
/// Serialize a value without imposing a second admission limit.
/// Previously admitted state/results keep their original allowance. Use the
/// explicit-limit variant for a new export quota on caller-created data.
pub fn value_to_json(value: &Value) -> Result<String, LimitError> {
    Ok(crate::tree::value_json(value))
}
/// Serialize under explicit ceilings. Refusal allocates no serialized output.
/// Serialized bytes, rather than the representation's in-memory size, are charged.
pub fn value_to_json_with_limits(value: &Value, limits: Limits) -> Result<String, LimitError> {
    measure(value, limits)?;
    Ok(crate::tree::value_json(value))
}
