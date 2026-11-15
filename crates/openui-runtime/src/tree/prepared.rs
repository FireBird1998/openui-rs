//! Bounded reference-data preparation, independent of evaluation depth.
use super::*;
use crate::{EvaluationFlags, PreparedReferenceValue};

/// Borrowed reference values measured once for repeated tree evaluations.
///
/// Preparation uses its own bounds. A refused entry earns no input credit and
/// raises its cut flag only if a declared reference actually reads it. Preparing
/// or passing unread data never cuts an unrelated prop. Read representations
/// are cached too: containers remain shared and scalar strings are converted
/// once. Reads grant no extra allowance; output still pays its full cost.
pub struct PreparedReferenceValues<'a> {
    values: &'a JsObject<Value>,
    measurements: BTreeMap<&'a str, Result<PreparedReferenceValue, EvaluationFlags>>,
    pub(super) bytes: usize,
    pub(super) input: openui_lang::PreparedInput,
}
impl<'a> PreparedReferenceValues<'a> {
    /// Prepare with a 32,768-level iterative measurement bound and the default
    /// maximum work/size ceilings. Evaluation keeps its own independent limits.
    pub fn new(values: &'a JsObject<Value>) -> Self {
        Self::with_limits(values, Limits::DEFAULT.with_depth(32_768))
    }
    /// Set preparation bounds; these do not spend an evaluation's allowance.
    pub fn with_limits(values: &'a JsObject<Value>, limits: Limits) -> Self {
        let (measurements, bytes) = PreparedReferenceValue::measure_all(values, limits);
        let input = openui_lang::PreparedInput::from_prepared_references(
            measurements
                .iter()
                .filter_map(|(name, entry)| entry.as_ref().ok().map(|v| (*name, v))),
        );
        Self {
            values,
            measurements,
            bytes,
            input,
        }
    }
    pub(super) fn get(
        &self,
        name: &str,
    ) -> Result<Option<(&Value, &PreparedReferenceValue)>, EvaluationFlags> {
        let Some(entry) = self.measurements.get(name) else {
            return Ok(None);
        };
        let Some(value) = self.values.get(name) else {
            return Ok(None);
        };
        match entry {
            Ok(prepared) => Ok(Some((value, prepared))),
            Err(flags) => Err(*flags),
        }
    }
}
