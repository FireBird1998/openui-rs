//! Live, read-only state for a query manager whose Store borrow ends at registration.
use super::*;
use std::sync::{Mutex, Weak};

pub(crate) struct QueryState {
    snapshot: Option<StoreSnapshot>,
    undefined_fields: Arc<crate::tree::actions::Provenance>,
    entry_bytes: usize,
    limits: Limits,
}

#[derive(Default)]
pub(super) struct QueryWatch {
    // Managers own the view. A removed reader or refused registration releases
    // its snapshot even if the Store is never written again.
    state: Mutex<Weak<Mutex<QueryState>>>,
    staged: bool,
}

impl QueryWatch {
    pub(super) fn staged(&self) -> Self {
        Self {
            state: Mutex::new(self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()),
            staged: true,
        }
    }

    pub(super) fn commit(&mut self) {
        self.staged = false;
    }

    // A staged Store publishes only after its transaction commits.
    pub(super) fn writing(&mut self) -> Option<Arc<Mutex<QueryState>>> {
        if self.staged {
            return None;
        }
        self.state
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .upgrade()
    }

    pub(super) fn publish(
        &mut self,
        snapshot: &StoreSnapshot,
        bytes: usize,
        fields: &Arc<crate::tree::actions::Provenance>,
    ) {
        if let Some(state) = self.writing() {
            state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .publish(snapshot, bytes, fields);
        }
    }
}

#[derive(Clone)]
pub(crate) struct QueryStateReader(Arc<Mutex<QueryState>>);

impl QueryStateReader {
    pub(crate) fn read(&self) -> Store {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Store {
            snapshot: state.snapshot.clone().unwrap_or_default(),
            entry_bytes: state.entry_bytes,
            undefined_fields: state.undefined_fields.clone(),
            ..Store::with_limits(state.limits)
        }
    }
}

impl Store {
    pub(crate) fn query_state(&self) -> QueryStateReader {
        let mut watched = self
            .query_watch
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(state) = watched.upgrade() {
            return QueryStateReader(state);
        }
        let state = Arc::new(Mutex::new(QueryState {
            snapshot: Some(self.snapshot.clone()),
            entry_bytes: self.entry_bytes,
            undefined_fields: self.undefined_fields.clone(),
            limits: self.limits,
        }));
        *watched = Arc::downgrade(&state);
        QueryStateReader(state)
    }
}

impl QueryState {
    pub(super) fn begin_write(&mut self) {
        // Release the view's owner before Arc::make_mut so the Store can still
        // update its snapshot in place. Readers take this same lock.
        self.snapshot.take();
    }

    pub(super) fn publish(
        &mut self,
        snapshot: &StoreSnapshot,
        bytes: usize,
        fields: &Arc<crate::tree::actions::Provenance>,
    ) {
        self.snapshot = Some(snapshot.clone());
        self.entry_bytes = bytes;
        self.undefined_fields = fields.clone();
    }
}
