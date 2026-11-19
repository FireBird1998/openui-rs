//! One embedder-supplied clock. Timers retain their installation order when
//! their deadlines tie, including after one statement changes its interval.
use super::*;

#[derive(Clone)]
pub(super) struct Interval {
    raw: Value,
    millis: u64,
}

impl Interval {
    pub(super) fn new(raw: Option<Value>, seconds: f64, minimum_ms: u64) -> Option<Self> {
        if !seconds.is_finite() || seconds <= 0.0 {
            return None;
        }
        // setInterval uses whole milliseconds. Its signed 32-bit delay
        // overflows to one millisecond on the reference platform.
        let delay = seconds * 1000.0;
        let millis = if delay > f64::from(i32::MAX) {
            1
        } else {
            delay as u64
        };
        Some(Self {
            raw: raw?,
            millis: millis.max(minimum_ms),
        })
    }
}

#[derive(Clone)]
struct Timer {
    id: String,
    interval: Interval,
    due: u64,
}

#[derive(Clone)]
pub(super) struct Clock {
    now: u64,
    pub(super) minimum_ms: u64,
    timers: Vec<Timer>,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            now: 0,
            minimum_ms: 1000,
            timers: Vec::new(),
        }
    }
}

impl Clock {
    pub(super) fn reconcile(
        &mut self,
        active: &HashSet<&str>,
        intervals: Vec<(&str, Option<Interval>)>,
    ) {
        if self.timers.is_empty() && intervals.iter().all(|(_, interval)| interval.is_none()) {
            return;
        }
        // Keep the old installation order, then append new timers in node
        // order. A public ParseResult can contain several nodes for one id.
        // Vacating the old slot on each change also handles that case.
        let mut positions = HashMap::new();
        let mut timers = Vec::new();
        for timer in self.timers.drain(..) {
            if let Some(&id) = active.get(timer.id.as_str()) {
                positions.insert(id, timers.len());
                timers.push(Some(timer));
            }
        }
        for (id, interval) in intervals {
            if let Some(slot) = positions.get(id).and_then(|&i| timers.get_mut(i)) {
                if let (Some(old), Some(new)) = (slot.as_ref(), interval.as_ref())
                    && crate::store::same(Some(&old.interval.raw), Some(&new.raw))
                {
                    continue;
                }
                *slot = None;
            }
            if let Some(interval) = interval {
                positions.insert(id, timers.len());
                timers.push(Some(Timer {
                    id: id.to_owned(),
                    due: self.now + interval.millis,
                    interval,
                }));
            }
        }
        self.timers.extend(timers.into_iter().flatten());
    }
}

impl QueryManager {
    pub(super) fn idle_update(&self) -> QueryUpdate {
        QueryUpdate {
            calls: Vec::new(),
            changed: false,
            hidden_change: false,
            next_refresh_due_ms: self.next_refresh_due_ms(),
        }
    }

    /// Construct a manager with a polling floor in whole milliseconds.
    pub fn with_refresh_minimum(
        limits: Limits,
        minimum_ms: u64,
    ) -> Result<Self, QueryCommandError> {
        if !(1..=i32::MAX as u64).contains(&minimum_ms) {
            return Err(QueryCommandError::InvalidClock);
        }
        let mut manager = Self::with_limits(limits);
        manager.clock.minimum_ms = minimum_ms;
        Ok(manager)
    }

    /// The polling floor selected when this manager was constructed.
    pub fn refresh_minimum_ms(&self) -> u64 {
        self.clock.minimum_ms
    }

    /// Earliest absolute deadline, in milliseconds on the supplied clock.
    pub fn next_refresh_due_ms(&self) -> Option<u64> {
        self.clock.timers.iter().map(|t| t.due).min()
    }

    /// Whether registrations and calls are suspended until activation.
    pub fn is_disposed(&self) -> bool {
        self.disposed
    }

    pub(super) fn stop(&mut self) {
        self.lifecycle = Arc::new(());
        self.disposed = true;
        self.clock.timers.clear();
        self.mutations = Mutations::default();
        for entry in self.cache.values_mut() {
            entry.pending = None;
            entry.run_again = None;
        }
    }

    pub(super) fn advance(&mut self, now: u64) -> Result<Vec<ToolCall>, QueryCommandError> {
        // Reserve room for a full platform interval beyond every accepted time.
        if now < self.clock.now || now > u64::MAX - i32::MAX as u64 {
            return Err(QueryCommandError::InvalidClock);
        }
        self.clock.now = now;
        let mut ticks = Vec::new();
        for timer in &mut self.clock.timers {
            if timer.due <= now {
                ticks.push((timer.due, timer.id.clone()));
                timer.due = now + timer.interval.millis;
            }
        }
        // A late host runs each overdue callback once, earliest deadline first.
        // Re-arming precedes the fetch checks so a skipped tick also waits a full period.
        ticks.sort_by_key(|(due, _)| *due);
        let mut calls = Vec::new();
        for (_, id) in ticks {
            if self.waits(&id) {
                continue;
            }
            if let Some(query) = self.queries.get(&id)
                && let Some(entry) = self.cache.get_mut(&query.key)
                && entry.pending.is_none()
            {
                calls.push(runs::start_call(
                    &self.owner,
                    entry,
                    query.key.clone(),
                    &query.request,
                ));
            }
        }
        Ok(calls)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluating_an_interval_leaves_no_seconds_for_the_next_statement() {
        let library = openui_lang::Library::from_json_schema(&JsonValue::Null);
        let program = openui_lang::parse(
            "a=Query(\"a\",{},null,2)\nb=Query(\"b\",{},null,false)",
            &library,
            None,
        );
        assert_eq!(program.query_statements.len(), 2);
        let manager = QueryManager::default();
        manager
            .with_statement_evaluator(&TreeInput::new(&program), &Store::new(), |scope, eval| {
                for statement in &program.query_statements {
                    scope.node(statement, eval)?;
                    assert_eq!(scope.seconds.get(), 0.0);
                }
                Ok(())
            })
            .unwrap();
    }
}
