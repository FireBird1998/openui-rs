//! Bounded reference-data preparation, independent of evaluation depth.
use super::*;
use crate::{EvaluationFlags, PreparedReferenceValue};
use std::borrow::Cow;

/// Borrowed or owned reference values measured once for repeated tree evaluations.
///
/// Preparation uses its own bounds. A refused entry earns no input credit and
/// raises its cut flag only if a declared reference actually reads it. Preparing
/// or passing unread data never cuts an unrelated prop. Read representations
/// are cached too: containers remain shared and scalar strings are converted
/// once. Reads grant no extra allowance; output still pays its full cost.
pub struct PreparedReferenceValues<'a> {
    values: Cow<'a, JsObject<Value>>,
    measurements: BTreeMap<Cow<'a, str>, Result<PreparedReferenceValue, EvaluationFlags>>,
    pub(super) bytes: usize,
    tail: EvaluationFlags,
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
        let (measurements, bytes, tail) =
            PreparedReferenceValue::measure_all_with_tail(values, limits);
        let input = openui_lang::PreparedInput::from_prepared_references_with_limits(
            measurements
                .iter()
                .filter_map(|(name, entry)| entry.as_ref().ok().map(|v| (*name, v))),
            // Keys were measured by the core above. Combining their receipt
            // may revisit those bytes, but never refreshes evaluation work.
            limits.with_work(limits.work_for(usize::MAX).max(bytes)),
        );
        Self {
            values: Cow::Borrowed(values),
            measurements: measurements
                .into_iter()
                .map(|(k, v)| (Cow::Borrowed(k), v))
                .collect(),
            bytes,
            tail,
            input,
        }
    }
    /// Own and prepare a supply once for reuse without a self-referential owner.
    /// Measurement, cut flags and input credit are identical to `with_limits`;
    /// callers cannot construct or modify the measurement receipts.
    pub fn owned_with_limits(
        values: JsObject<Value>,
        limits: Limits,
    ) -> PreparedReferenceValues<'static> {
        let prepared = PreparedReferenceValues::with_limits(&values, limits);
        let measurements = prepared
            .measurements
            .into_iter()
            .map(|(name, entry)| (Cow::Owned(name.into_owned()), entry))
            .collect();
        let bytes = prepared.bytes;
        let tail = prepared.tail;
        let input = prepared.input;
        PreparedReferenceValues {
            values: Cow::Owned(values),
            measurements,
            bytes,
            tail,
            input,
        }
    }
    pub(super) fn get(
        &self,
        name: &str,
    ) -> Result<Option<(&Value, &PreparedReferenceValue)>, EvaluationFlags> {
        let Some(entry) = self.measurements.get(name) else {
            return if (self.tail.size_limited || self.tail.depth_limited)
                && self.values.has_own(name)
            {
                Err(self.tail)
            } else {
                Ok(None)
            };
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
