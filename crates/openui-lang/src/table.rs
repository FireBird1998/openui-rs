//! A program's statements by name, with what incremental result building
//! needs: a slot number per name (defined or only referenced), the reverse
//! reference graph, a count of how many places in the current result reach
//! each statement, and a cache of materialized statements by slot.
//!
//! The streaming parser keeps one table and cache for the whole stream. The
//! statement still arriving is laid over the table for each result and taken
//! back afterwards. Each change invalidates the cached statements that
//! reference the changed one, directly or through others, so the rest of the
//! tree is shared with the result before; the work per result is then the
//! chain from the entry down to the change, not the whole program.

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

use crate::ast::Ast;
use crate::materialize::{ErrorPart, UnresolvedPart};
use crate::parser::{Info, Kind, Statement};
use crate::result::Value;

/// A name's number: dense from 0, fixed for the life of the table.
pub(crate) type Slot = u32;

#[derive(Default)]
pub(crate) struct Table {
    slots: HashMap<String, Slot>,
    names: Vec<String>,
    statements: Vec<Option<Statement>>,
    info: Vec<Option<Info>>,
    /// Defined slots in first-definition order: the reference's `Map` order.
    order: Vec<Slot>,
    /// Each slot's index in `order`, once it has one.
    position: Vec<u32>,
    /// The defined statements that every result has to look at: state,
    /// queries, mutations, and anything reading a `$name`; in order.
    specials: Vec<Slot>,
    /// The statements whose expression references each slot.
    referrers: Vec<Vec<Slot>>,
    /// How many places in the last result's tree reached each slot.
    reached: Vec<u32>,
    /// The last result's reached tree, to diff the next one against.
    root_reach: Option<Arc<Reach>>,
    // A staged parser shares immutable reach graphs with its predecessor. Keep
    // changed incoming counts private until the enclosing draw commits.
    staged_counts: Option<HashMap<usize, (Arc<Reach>, usize)>>,
    /// The entry statement the orphan count is relative to.
    entry: Option<Slot>,
    /// Value statements (other than the entry) reached by nothing.
    orphans: usize,
    /// Bumped whenever the orphan set may have changed.
    orphans_version: u64,
    orphans_memo: Option<(u64, Vec<String>)>,
    /// While a staged update is open: what it changed, to take back if the
    /// update is refused. See [`Table::mark`].
    journal: Option<Box<TableJournal>>,
}

/// What a [`Table`] looked like when a staged update began, and the reach
/// counts it has changed since, oldest first. A refused update puts every
/// one back ([`Table::rollback`]), so the table is as it was, without a copy
/// of it being made for each update.
struct TableJournal {
    names: usize,
    reached: Vec<(Slot, u32)>,
    entry: Option<Slot>,
    orphans: usize,
    orphans_version: u64,
    orphans_memo: Option<(u64, Vec<String>)>,
    root_reach: Option<Arc<Reach>>,
    rolling_back: bool,
}

/// What a definition replaced, so a temporary one can be taken back.
pub(crate) struct Undo {
    pub slot: Slot,
    previous: Option<(Statement, Info)>,
}

/// The statements a materialization reached: its own lookups, and the trees
/// of the cached statements it took as they were. Shared between results,
/// so two results' trees can be compared by pointer where nothing changed.
pub(crate) struct Reach {
    /// Incoming links in the owning Table's current result. Only transitions
    /// between zero and one walk children. Atomic solely to keep streams
    /// Send + Sync; every update happens under the Table's mutable borrow.
    pub active: AtomicUsize,
    /// Looked up directly, sorted and deduplicated.
    pub direct: Vec<Slot>,
    pub children: Vec<Arc<Reach>>,
}

impl Table {
    fn intern(&mut self, name: &str) -> Slot {
        if let Some(&slot) = self.slots.get(name) {
            return slot;
        }
        let slot = Slot::try_from(self.names.len()).expect("fewer than 2^32 names");
        self.slots.insert(name.to_owned(), slot);
        self.names.push(name.to_owned());
        self.statements.push(None);
        self.info.push(None);
        self.position.push(u32::MAX);
        self.referrers.push(Vec::new());
        self.reached.push(0);
        slot
    }

    /// Define or re-define a statement. A re-definition keeps its position.
    /// Every name the statement references gets a slot too, so lookups never
    /// need one later.
    pub(crate) fn define(&mut self, name: &str, stmt: Statement) -> Undo {
        let slot = self.intern(name);
        let mut refs = Vec::new();
        stmt.expr().walk(&mut |node| {
            if let Ast::Ref(target) = node {
                refs.push(self.intern(target));
            }
        });
        refs.sort_unstable();
        refs.dedup();
        let mut info = Info::of(&stmt);
        info.refs = refs;
        let (kind, special) = (info.kind, info.special());
        for &r in &info.refs {
            if !self.referrers[r as usize].contains(&slot) {
                self.referrers[r as usize].push(slot);
            }
        }
        let previous = self.statements[slot as usize]
            .replace(stmt)
            .zip(self.info[slot as usize].replace(info));
        match &previous {
            None => {
                self.position[slot as usize] =
                    u32::try_from(self.order.len()).expect("fewer than 2^32 statements");
                self.order.push(slot);
                self.set_special(slot, special, false);
                self.orphan_change(slot, None, Some(kind));
            }
            Some((_, before)) => {
                for &r in &before.refs {
                    if !self.info(slot).refs.contains(&r) {
                        self.referrers[r as usize].retain(|&s| s != slot);
                    }
                }
                self.set_special(slot, special, before.special());
                self.orphan_change(slot, Some(before.kind), Some(kind));
            }
        }
        Undo { slot, previous }
    }

    /// Take a definition back. Definitions are undone in reverse order.
    pub(crate) fn undo(&mut self, undo: Undo) {
        let Undo { slot, previous } = undo;
        let i = slot as usize;
        let current = self.info[i].take().expect("defined statement");
        for &r in &current.refs {
            self.referrers[r as usize].retain(|&s| s != slot);
        }
        match previous {
            Some((stmt, info)) => {
                for &r in &info.refs {
                    // Always so: s is in referrers[r] once if r is in
                    // info[s].refs, and not at all otherwise. `define` sorts
                    // and deduplicates refs to keep it so, and the loop above
                    // took slot out of its current refs' lists, so each of
                    // previous.refs takes it back once.
                    self.referrers[r as usize].push(slot);
                }
                self.set_special(slot, info.special(), current.special());
                self.orphan_change(slot, Some(current.kind), Some(info.kind));
                self.statements[i] = Some(stmt);
                self.info[i] = Some(info);
            }
            None => {
                self.set_special(slot, false, current.special());
                self.orphan_change(slot, Some(current.kind), None);
                self.statements[i] = None;
                self.position[i] = u32::MAX;
                let last = self.order.pop();
                debug_assert_eq!(last, Some(slot), "undone out of order");
            }
        }
    }

    /// Keep `specials` in statement order as a slot's status changes.
    fn set_special(&mut self, slot: Slot, now: bool, before: bool) {
        if now == before {
            return;
        }
        let position = self.position[slot as usize];
        let at = self
            .specials
            .binary_search_by_key(&position, |&s| self.position[s as usize]);
        match (now, at) {
            (true, Err(at)) => self.specials.insert(at, slot),
            (false, Ok(at)) => {
                self.specials.remove(at);
            }
            _ => unreachable!("specials out of sync"),
        }
    }

    pub(crate) fn slot(&self, name: &str) -> Option<Slot> {
        self.slots.get(name).copied()
    }

    pub(crate) fn get(&self, slot: Slot) -> Option<&Statement> {
        self.statements[slot as usize].as_ref()
    }

    pub(crate) fn by_name(&self, name: &str) -> Option<&Statement> {
        self.get(self.slot(name)?)
    }

    /// Facts about a defined statement.
    pub(crate) fn info(&self, slot: Slot) -> &Info {
        self.info[slot as usize]
            .as_ref()
            .expect("defined statement")
    }

    pub(crate) fn name(&self, slot: Slot) -> &str {
        &self.names[slot as usize]
    }

    /// Defined statements, in order.
    pub(crate) fn order(&self) -> &[Slot] {
        &self.order
    }

    /// The defined statements that aren't plain values without `$name`s, in order.
    pub(crate) fn specials(&self) -> &[Slot] {
        &self.specials
    }

    /// Defined and referenced names, including missing terminals.
    pub(crate) fn slot_count(&self) -> usize {
        self.names.len()
    }

    /// How many statements are defined.
    pub(crate) fn len(&self) -> usize {
        self.order.len()
    }

    /// The statements whose expression references `slot`.
    pub(crate) fn referrers(&self, slot: Slot) -> &[Slot] {
        &self.referrers[slot as usize]
    }

    // ── Orphans ────────────────────────────────────────────────────────────
    //
    // A value statement is orphaned when nothing in the result's tree reaches
    // it (the entry excepted). `reached` counts the places that do, kept up to
    // date by diffing each result's reached tree against the last one's, and
    // `orphans` counts the statements at zero, kept up to date as statements
    // are defined, re-defined and undone. Listing them costs a scan, but only
    // when there are some and the set may have changed.

    /// Whether `slot` counts as orphaned right now.
    fn orphaned(&self, slot: Slot) -> bool {
        Some(slot) != self.entry
            && self.reached[slot as usize] == 0
            && self.info[slot as usize]
                .as_ref()
                .is_some_and(|i| i.kind == Kind::Value)
    }

    /// `slot`'s kind went from `before` to `after` (`None` for undefined).
    fn orphan_change(&mut self, slot: Slot, before: Option<Kind>, after: Option<Kind>) {
        // A rollback puts the orphan count back from its journal afterwards.
        if self
            .journal
            .as_ref()
            .is_some_and(|journal| journal.rolling_back)
        {
            return;
        }
        if Some(slot) == self.entry || self.reached[slot as usize] != 0 {
            return;
        }
        let was = before == Some(Kind::Value);
        let is = after == Some(Kind::Value);
        if was != is {
            if is {
                self.orphans += 1;
            } else {
                self.orphans -= 1;
            }
            self.orphans_version += 1;
        }
    }

    fn note_reached(&mut self, slot: Slot) {
        if let Some(journal) = &mut self.journal {
            journal.reached.push((slot, self.reached[slot as usize]));
        }
    }

    fn reach_inc(&mut self, slot: Slot) {
        self.note_reached(slot);
        let count = &mut self.reached[slot as usize];
        *count += 1;
        if *count == 1 && Some(slot) != self.entry {
            let value = self.info[slot as usize]
                .as_ref()
                .is_some_and(|i| i.kind == Kind::Value);
            if value {
                self.orphans -= 1;
                self.orphans_version += 1;
            }
        }
    }

    fn reach_dec(&mut self, slot: Slot) {
        self.note_reached(slot);
        let count = &mut self.reached[slot as usize];
        *count -= 1;
        if *count == 0 && Some(slot) != self.entry {
            let value = self.info[slot as usize]
                .as_ref()
                .is_some_and(|i| i.kind == Kind::Value);
            if value {
                self.orphans += 1;
                self.orphans_version += 1;
            }
        }
    }

    /// Begin a staged update: reach counts stay private until
    /// [`Table::commit`], and what the update changes is noted so that
    /// [`Table::rollback`] can take it back. Definitions the update makes are
    /// taken back by the caller, with the [`Undo`]s they return, before the
    /// rollback.
    pub(crate) fn mark(&mut self) {
        self.staged_counts = Some(HashMap::new());
        self.journal = Some(Box::new(TableJournal {
            names: self.names.len(),
            reached: Vec::new(),
            entry: self.entry,
            orphans: self.orphans,
            orphans_version: self.orphans_version,
            orphans_memo: self.orphans_memo.clone(),
            root_reach: self.root_reach.clone(),
            rolling_back: false,
        }));
    }

    /// Keep what the staged update changed.
    pub(crate) fn commit(&mut self) {
        self.journal = None;
        if let Some(counts) = self.staged_counts.take() {
            for (_, (reach, count)) in counts {
                reach.active.store(count, Relaxed);
            }
        }
    }

    /// Put back the reach counts the staged update changed. Call
    /// [`Table::rollback_rest`] once its definitions have been undone.
    pub(crate) fn rollback_counts(&mut self) {
        self.staged_counts = None;
        if let Some(journal) = &mut self.journal {
            journal.rolling_back = true;
            for (slot, count) in journal.reached.drain(..).rev() {
                self.reached[slot as usize] = count;
            }
        }
    }

    /// Finish taking back a staged update after its definitions: the result
    /// it was compared with, and the names it brought in.
    pub(crate) fn rollback_rest(&mut self) {
        let Some(journal) = self.journal.take() else {
            return;
        };
        let TableJournal {
            names,
            entry,
            orphans,
            orphans_version,
            orphans_memo,
            root_reach,
            ..
        } = *journal;
        self.entry = entry;
        self.orphans = orphans;
        self.orphans_version = orphans_version;
        self.orphans_memo = orphans_memo;
        self.root_reach = root_reach;
        // Names the update brought in are defined by nothing now and
        // referenced by nothing older, so dropping their slots leaves the
        // table as it was.
        for name in self.names.drain(names..) {
            self.slots.remove(&name);
        }
        self.statements.truncate(names);
        self.info.truncate(names);
        self.position.truncate(names);
        self.referrers.truncate(names);
        self.reached.truncate(names);
    }

    fn change_count(&mut self, reach: &Arc<Reach>, add: bool) -> usize {
        if let Some(counts) = &mut self.staged_counts {
            let (_, count) = counts
                .entry(Arc::as_ptr(reach) as usize)
                .or_insert_with(|| (reach.clone(), reach.active.load(Relaxed)));
            let before = *count;
            if add {
                *count += 1;
            } else {
                *count -= 1;
            }
            before
        } else if add {
            reach.active.fetch_add(1, Relaxed)
        } else {
            reach.active.fetch_sub(1, Relaxed)
        }
    }

    fn reach_add(&mut self, reach: &Arc<Reach>) {
        crate::stack::grow_stack!(self.reach_add(reach));
        #[cfg(test)]
        crate::materialize::cost_tests::visit();
        if self.change_count(reach, true) != 0 {
            return;
        }
        for &slot in &reach.direct {
            self.reach_inc(slot);
        }
        for child in &reach.children {
            self.reach_add(child);
        }
    }

    fn reach_remove(&mut self, reach: &Arc<Reach>) {
        crate::stack::grow_stack!(self.reach_remove(reach));
        #[cfg(test)]
        crate::materialize::cost_tests::visit();
        if self.change_count(reach, false) != 1 {
            return;
        }
        for &slot in &reach.direct {
            self.reach_dec(slot);
        }
        for child in &reach.children {
            self.reach_remove(child);
        }
    }

    /// Add before removing: unchanged shared tails stay live throughout.
    fn reach_diff(&mut self, old: Option<&Arc<Reach>>, new: &Arc<Reach>) {
        if old.is_some_and(|old| Arc::ptr_eq(old, new)) {
            return;
        }
        self.reach_add(new);
        if let Some(old) = old {
            self.reach_remove(old);
        }
    }

    /// A result was built from `entry` and reached `reach`: bring the counts
    /// up to date and return the orphaned statements, in order.
    pub(crate) fn finish_result(&mut self, entry: Slot, reach: Arc<Reach>) -> Vec<String> {
        if self.entry != Some(entry) {
            self.entry = Some(entry);
            self.orphans = self.order.iter().filter(|&&s| self.orphaned(s)).count();
            self.orphans_version += 1;
        }
        let old = self.root_reach.take();
        self.reach_diff(old.as_ref(), &reach);
        self.root_reach = Some(reach);
        if self.orphans == 0 {
            return Vec::new();
        }
        if let Some((version, list)) = &self.orphans_memo
            && *version == self.orphans_version
        {
            return list.clone();
        }
        let list: Vec<String> = self
            .order
            .iter()
            .filter(|&&s| self.orphaned(s))
            .map(|&s| self.names[s as usize].clone())
            .collect();
        self.orphans_memo = Some((self.orphans_version, list.clone()));
        list
    }
}

/// What a statement materialized to, with everything its materialization did
/// on the side, so a later result can take it as is.
#[derive(Clone)]
pub(crate) struct Entry {
    pub output: Output,
    pub reach: Arc<Reach>,
    pub unresolved: Arc<Vec<UnresolvedPart>>,
    /// Its errors, those of the statements it reached kept apart: the same
    /// whatever was reported before, so any later pass can take them.
    pub errors: Arc<Vec<ErrorPart>>,
    /// How much deeper than its starting depth the materialization went.
    pub r_max: usize,
    /// How much it grew the result (`Ctx::size`), counted again on each reuse.
    pub size: usize,
    /// Of that size, the charges outside the retained AST.
    pub side_size: usize,
    /// Room needed while building it, including temporary call arguments.
    pub peak: usize,
    /// For a state default's value: the statement its errors were attributed
    /// to (the entry's), which a reuse must share. `None` for the rest.
    pub context: Option<String>,
    /// A depth-cut result is reusable only at this starting depth with no
    /// open reference path; replay retains its exact number of cuts.
    pub depth_cut: Option<(usize, usize)>,
}

#[derive(Clone)]
pub(crate) enum Output {
    Value(Value),
    Ast(Ast),
}

/// Materialized statements by slot, one per (partial, mode) combination:
/// `partial` changes every element, and expression mode produces AST.
///
/// An entry is valid for as long as it's here: a change to a statement drops
/// the entries of every statement that references it, directly or not.
#[derive(Default)]
pub(crate) struct Cache {
    entries: Vec<[Option<Box<Entry>>; 4]>,
    /// One bit per slot, for set operations on slot lists; clear between uses.
    marks: Vec<u64>,
    /// While a staged update is open: how many slots there were, and each
    /// entry it replaced or dropped, oldest first, moved here rather than
    /// copied.
    journal: Option<(usize, Vec<Replaced>)>,
}

/// An entry a staged update replaced: its slot, its variant and what was there.
type Replaced = (Slot, usize, Option<Box<Entry>>);

impl Cache {
    fn index(partial: bool, expr: bool) -> usize {
        usize::from(partial) | (usize::from(expr) << 1)
    }

    pub(crate) fn get(&self, slot: Slot, partial: bool, expr: bool) -> Option<&Entry> {
        self.entries.get(slot as usize)?[Self::index(partial, expr)].as_deref()
    }

    pub(crate) fn put(&mut self, slot: Slot, partial: bool, expr: bool, entry: Entry) {
        let i = slot as usize;
        if self.entries.len() <= i {
            self.entries.resize_with(i + 1, Default::default);
        }
        let index = Self::index(partial, expr);
        let old = self.entries[i][index].replace(Box::new(entry));
        if let Some((_, journal)) = &mut self.journal {
            journal.push((slot, index, old));
        }
    }

    /// Begin a staged update; see [`Table::mark`].
    pub(crate) fn mark_journal(&mut self) {
        self.journal = Some((self.entries.len(), Vec::new()));
    }

    /// Keep the entries the staged update made.
    pub(crate) fn commit(&mut self) {
        self.journal = None;
    }

    /// Put back every entry the staged update replaced or dropped.
    pub(crate) fn rollback(&mut self) {
        let Some((len, journal)) = self.journal.take() else {
            return;
        };
        for (slot, index, old) in journal.into_iter().rev() {
            if let Some(entries) = self.entries.get_mut(slot as usize) {
                entries[index] = old;
            }
        }
        self.entries.truncate(len);
    }

    /// The reached tree another variant of `slot` already holds, when it's
    /// the same tree; otherwise `reach` itself. `partial` and the mode change
    /// what a statement materializes to, not what it reaches, so the variants
    /// end up sharing one tree and results built under different `partial`
    /// flags compare equal by pointer.
    pub(crate) fn share_reach(&self, slot: Slot, reach: Arc<Reach>) -> Arc<Reach> {
        let Some(entries) = self.entries.get(slot as usize) else {
            return reach;
        };
        let same = |other: &Arc<Reach>| {
            other.direct == reach.direct
                && other.children.len() == reach.children.len()
                && other
                    .children
                    .iter()
                    .zip(&reach.children)
                    .all(|(a, b)| Arc::ptr_eq(a, b))
        };
        match entries.iter().flatten().find(|e| same(&e.reach)) {
            Some(entry) => entry.reach.clone(),
            None => reach,
        }
    }

    /// The statements in `slots` changed: drop their entries and those of
    /// everything that references them, however indirectly. One walk for
    /// them all, so a statement above many of them is visited once. Nothing
    /// is cached between the changes of one push, so walking the references
    /// as they stand after the last of them drops the same entries as a walk
    /// after each: a reference a later change took away belonged to a
    /// statement that is itself in `slots`.
    pub(crate) fn invalidate(&mut self, slots: &[Slot], table: &Table) {
        let mut pending = slots.to_vec();
        let mut seen = Vec::new();
        while let Some(s) = pending.pop() {
            if self.marked(s) {
                continue;
            }
            self.mark(&[s]);
            seen.push(s);
            if let Some(entries) = self.entries.get_mut(s as usize) {
                let dropped = std::mem::take(entries);
                if let Some((_, journal)) = &mut self.journal {
                    for (index, old) in dropped.into_iter().enumerate() {
                        if old.is_some() {
                            journal.push((s, index, old));
                        }
                    }
                }
            }
            pending.extend_from_slice(table.referrers(s));
        }
        self.unmark(&seen);
    }

    fn mark(&mut self, slots: &[Slot]) {
        for &slot in slots {
            let word = slot as usize / 64;
            if self.marks.len() <= word {
                self.marks.resize(word + 1, 0);
            }
            self.marks[word] |= 1 << (slot % 64);
        }
    }

    fn marked(&self, slot: Slot) -> bool {
        self.marks
            .get(slot as usize / 64)
            .is_some_and(|word| word & (1 << (slot % 64)) != 0)
    }

    fn unmark(&mut self, slots: &[Slot]) {
        for &slot in slots {
            if let Some(word) = self.marks.get_mut(slot as usize / 64) {
                *word &= !(1 << (slot % 64));
            }
        }
    }
}
