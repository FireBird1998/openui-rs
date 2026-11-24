//! One tool delivery spends one work allowance (#222). Raw result validation, snapshot admission,
//! reference preparation and the reader cascade each keep their own result
//! room, but all of them draw on the same work. A phase with room of its own
//! starts on the remaining balance. Reader evaluation also keeps its
//! program-scaled allowance. Each newly suspended reader gets one error row.
use super::*;

pub(super) struct Allowance {
    left: usize,
    #[cfg(feature = "query-audit")]
    audit: DeliveryWork,
}

/// The work one delivery measured on its own counters, for tests. Outside the
/// supported interface.
#[cfg(feature = "query-audit")]
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeliveryWork {
    /// The ordinary delivery allowance, capped at the work limit for any program size.
    pub limit: usize,
    /// Ordinary work debits across all phases.
    pub spent: usize,
    /// Portion of ordinary work spent comparing and preparing the published snapshot.
    pub publication: usize,
    /// Admission work for newly suspended readers, one error row each.
    pub listing: usize,
    /// Supplied bytes of the prepared references, measured apart from the debits.
    pub prepared_bytes: usize,
}

impl Allowance {
    pub(super) fn new(limits: Limits) -> Self {
        let left = limits.work_for(usize::MAX);
        Self {
            left,
            #[cfg(feature = "query-audit")]
            audit: DeliveryWork {
                limit: left,
                ..DeliveryWork::default()
            },
        }
    }

    /// Run a phase with room of its own on the work that is left.
    pub(super) fn phase<T>(
        &mut self,
        limits: Limits,
        f: impl FnOnce(&ExpressionEvaluator<'_>) -> T,
    ) -> (T, EvaluationFlags) {
        let session = EvaluationSession::with_limits(
            &(),
            0,
            crate::admission::fixed(limits).with_work(self.left),
        );
        let out = session.with_evaluator(|eval| {
            let out = f(eval);
            self.left = eval.remaining_work();
            #[cfg(feature = "query-audit")]
            {
                self.audit.spent = self.audit.spent.saturating_add(eval.work_debits());
            }
            out
        });
        (out, session.flags())
    }

    /// Reader evaluation has its own input credit and local allowance.
    /// Pass the delivery balance through it alongside those local limits.
    pub(super) fn cascade<T>(
        &mut self,
        session: &EvaluationSession<'_>,
        run: impl FnOnce() -> T,
    ) -> T {
        #[cfg(feature = "query-audit")]
        let start = session.with_evaluator(|eval| eval.work_debits());
        let result = session.with_command_work(&mut self.left, run);
        #[cfg(feature = "query-audit")]
        {
            let spent = session.with_evaluator(|eval| eval.work_debits()) - start;
            self.audit.spent = self.audit.spent.saturating_add(spent);
        }
        result
    }

    /// A cut replaces a previously admitted value with null and exactly one
    /// object of string fields. Its construction bounds the separate allowance.
    /// Count the same object visit, text walks and JSON bytes as reserve_value.
    #[cfg(feature = "query-audit")]
    pub(super) fn listing_row(&mut self, fields: &[(&str, &str)]) {
        let mut cost = 3usize.saturating_add(fields.len().saturating_sub(1));
        for &(key, value) in fields {
            cost = cost
                .saturating_add(7) // two text visits, four quotes and a colon
                .saturating_add(key.len())
                .saturating_add(value.len())
                .saturating_add(openui_lang::json_string_len(key))
                .saturating_add(openui_lang::json_string_len(value));
        }
        self.audit.listing = self.audit.listing.saturating_add(cost);
    }

    #[cfg(feature = "query-audit")]
    pub(super) fn prepared(&mut self, bytes: usize) {
        self.audit.prepared_bytes = self.audit.prepared_bytes.saturating_add(bytes);
    }

    #[cfg(feature = "query-audit")]
    pub(super) fn audit(&self) -> DeliveryWork {
        self.audit
    }
}
