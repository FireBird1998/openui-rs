//! Publication keeps shared result bodies until a host asks for the flat view.
use super::*;
use std::sync::OnceLock;

#[derive(Default)]
pub(super) struct Snapshot {
    fields: JsObject<Arc<Value>>,
    values: OnceLock<Arc<JsObject<Value>>>,
    json: Arc<OnceLock<String>>,
}

impl Snapshot {
    pub(super) fn values(&self) -> &Arc<JsObject<Value>> {
        self.values.get_or_init(|| {
            let mut values = JsObject::with_capacity(self.fields.len());
            for (id, value) in self.fields.iter_unordered() {
                values.define(id, value.as_ref().clone());
            }
            Arc::new(values)
        })
    }

    pub(super) fn json(&self) -> &str {
        self.json
            .get_or_init(|| crate::tree::value_json(&Value::Obj(self.values().clone())))
    }

    // The changed flag is required even when no work is left to flatten the
    // snapshot. Compare the small status lists and shared bodies first; only
    // genuinely distinct data with an ambiguous projection needs a byte scan.
    pub(super) fn changed(
        &mut self,
        previous: &Self,
        limits: Limits,
        work: &mut work::Allowance,
    ) -> Result<bool, QueryCommandError> {
        if self.fields.len() != previous.fields.len() {
            return Ok(true);
        }
        let mut ambiguous = Vec::new();
        for ((id, a), (old_id, b)) in self.fields.iter().zip(previous.fields.iter()) {
            if id != old_id {
                return Ok(true);
            }
            if Arc::ptr_eq(a, b) {
                continue;
            }
            if matches!(id.as_str(), "__openui_loading" | "__openui_refetching") {
                if a != b {
                    return Ok(true);
                }
            } else if id == "__openui_errors" {
                let (Value::Arr(a), Value::Arr(b)) = (a.as_ref(), b.as_ref()) else {
                    unreachable!()
                };
                if a.len() != b.len() {
                    return Ok(true);
                }
                for (a, b) in a.iter().zip(b.iter()) {
                    if !shared_value(a, b) {
                        ambiguous.push((a, b));
                    }
                }
            } else if !shared_value(a, b) {
                ambiguous.push((a.as_ref(), b.as_ref()));
            }
        }
        let changed = different_values(ambiguous, limits, work)?;
        if !changed {
            // The raw view can still differ (for example, -0 and +0).
            self.json = previous.json.clone();
        }
        Ok(changed)
    }

    // Eager serialization is optional. Keep the shared projection on a cut;
    // the first external read can still materialize the exact same snapshot.
    pub(super) fn prepare_json(&self, limits: Limits, work: &mut work::Allowance) {
        let (fits, flags) = work.phase(limits, |eval| {
            self.fields
                .iter_unordered()
                .all(|(_, value)| eval.reserve_value(value))
        });
        if fits && check_cuts(flags).is_ok() {
            self.json();
        }
    }
}

pub(super) fn different_values<'a>(
    pairs: impl IntoIterator<Item = (&'a Value, &'a Value)>,
    limits: Limits,
    work: &mut work::Allowance,
) -> Result<bool, QueryCommandError> {
    // These values already fit on their respective deliveries. Comparing two
    // projections does not admit their combined size as another tool result.
    // A mutation status or error row can wrap the admitted data.
    let limits = limits
        .with_result_size(limits.work_for(usize::MAX))
        .with_depth(limits.depth().saturating_add(3));
    let (changed, flags) = work.phase(limits, |eval| {
        for (a, b) in pairs {
            if shared_value(a, b) {
                continue;
            }
            if let (Value::Str(a), Value::Str(b)) = (a, b) {
                if a.len() != b.len() || !eval.charge_work(a.len()) || a != b {
                    return true;
                }
                continue;
            }
            // Mutation status transitions are visible before their data.
            if let (Value::Obj(a), Value::Obj(b)) = (a, b)
                && let (Some(Value::Str(a)), Some(Value::Str(b))) =
                    (a.get("status"), b.get("status"))
                && (a.len() != b.len() || a.as_bytes().first() != b.as_bytes().first())
            {
                return true;
            }
            if !eval.reserve_value(a) || !eval.reserve_value(b) {
                return true;
            }
            let a = crate::tree::value_json(a);
            let b = crate::tree::value_json(b);
            if a.len() != b.len() || !eval.charge_work(a.len()) || a != b {
                return true;
            }
        }
        false
    });
    check_cuts(flags)?;
    Ok(changed)
}

fn shared_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Obj(a), Value::Obj(b)) => Arc::ptr_eq(a, b),
        (Value::Arr(a), Value::Arr(b)) => Arc::ptr_eq(a, b),
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Num(a), Value::Num(b)) => a == b || (!a.is_finite() && !b.is_finite()),
        _ => false,
    }
}

impl QueryManager {
    /// Test-only: replace one published snapshot field, clearing the cached
    /// flat view, so audit tests can corrupt the published state directly.
    #[cfg(test)]
    pub(super) fn set_snapshot_field_for_test(&mut self, id: &str, value: Value) {
        let mut fields = self.snapshot.fields.clone();
        fields.set(id, Arc::new(value));
        self.snapshot = Arc::new(Snapshot {
            fields,
            values: OnceLock::new(),
            json: Arc::new(OnceLock::new()),
        });
    }

    pub(super) fn project_snapshot(&self) -> Snapshot {
        let pending = self.pending_queries();
        let mut loading = Vec::new();
        let mut refetching = Vec::new();
        let mut errors = Vec::new();
        for id in &self.order {
            let Some(query) = self.queries.get(id) else {
                continue;
            };
            if pending.contains(id.as_str()) {
                loading.push(Value::Str(id.clone()));
                if query.ever_fetched {
                    refetching.push(Value::Str(id.clone()));
                }
            }
            if let Some(error) = query.error.as_ref().filter(|_| !query.withheld) {
                errors.push(error.clone());
            }
        }
        errors.extend(self.withheld_diagnostic());
        self.mutations.append_errors(&mut errors);
        let mut snapshot = JsObject::new();
        snapshot.define("__openui_loading", Arc::new(Value::Arr(Arc::new(loading))));
        snapshot.define(
            "__openui_refetching",
            Arc::new(Value::Arr(Arc::new(refetching))),
        );
        snapshot.define("__openui_errors", Arc::new(Value::Arr(Arc::new(errors))));
        for id in &self.order {
            if snapshot.get(id).is_none()
                && let Some(query) = self.queries.get(id)
                && let Some(result) = self
                    .data_entry(query)
                    .and_then(|e| e.data.as_ref())
                    .or(query.defaults.as_ref())
            {
                snapshot.set(id, result.clone());
            }
        }
        self.mutations.write_snapshot(&mut snapshot);
        Snapshot {
            fields: snapshot,
            ..Snapshot::default()
        }
    }
}
