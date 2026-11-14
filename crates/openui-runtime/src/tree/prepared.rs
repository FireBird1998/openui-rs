//! Bounded reference-data preparation, independent of evaluation depth.
use super::*;
use crate::{EvaluationFlags, PreparedReferenceValue};
use std::collections::HashSet;

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
}
impl<'a> PreparedReferenceValues<'a> {
    /// Prepare with a 32,768-level iterative measurement bound and the default
    /// maximum work/size ceilings. Evaluation keeps its own independent limits.
    pub fn new(values: &'a JsObject<Value>) -> Self {
        Self::with_limits(values, Limits::DEFAULT.with_depth(32_768))
    }
    /// Set preparation bounds; these do not spend an evaluation's allowance.
    pub fn with_limits(values: &'a JsObject<Value>, limits: Limits) -> Self {
        let mut seen = HashSet::new();
        let mut work = limits.work_for(usize::MAX);
        let mut bytes = 2usize;
        let mut measurements = BTreeMap::new();
        for (name, value) in values.iter() {
            match input::value_size(value, limits, &mut seen, &mut work) {
                Ok(size) => {
                    measurements
                        .insert(name.as_str(), Ok(PreparedReferenceValue::new(value, size)));
                    bytes = bytes
                        .saturating_add(text_bytes(name))
                        .saturating_add(size)
                        .saturating_add(2)
                }
                Err(depth) => {
                    let mut flags = EvaluationFlags::default();
                    flags.depth_limited = depth;
                    flags.size_limited = !depth;
                    measurements.insert(name.as_str(), Err(flags));
                }
            }
        }
        Self {
            values,
            measurements,
            bytes,
        }
    }
    pub(super) fn get(
        &self,
        name: &str,
    ) -> Result<Option<(&Value, &PreparedReferenceValue)>, EvaluationFlags> {
        match self.measurements.get(name) {
            Some(Err(flags)) => Err(*flags),
            Some(Ok(prepared)) => Ok(self.values.get(name).map(|v| (v, prepared))),
            None => Ok(None),
        }
    }
}
