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
use std::sync::Arc;

use crate::ast::Ast;
use crate::materialize::ErrorPart;
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
    /// The entry statement the orphan count is relative to.
    entry: Option<Slot>,
    /// Value statements (other than the entry) reached by nothing.
    orphans: usize,
    /// Bumped whenever the orphan set may have changed.
    orphans_version: u64,
    orphans_memo: Option<(u64, Vec<String>)>,
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
                    // The current definition's edges were removed above;
                    // refs are deduplicated, so this edge is absent.
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

    fn reach_inc(&mut self, slot: Slot) {
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

    fn reach_add(&mut self, reach: &Reach) {
        crate::stack::grow_stack!(self.reach_add(reach));
        for &slot in &reach.direct {
            self.reach_inc(slot);
        }
        for child in &reach.children {
            self.reach_add(child);
        }
    }

    fn reach_remove(&mut self, reach: &Reach) {
        crate::stack::grow_stack!(self.reach_remove(reach));
        for &slot in &reach.direct {
            self.reach_dec(slot);
        }
        for child in &reach.children {
            self.reach_remove(child);
        }
    }

    /// Move the counts from `old`'s tree to `new`'s, skipping shared nodes.
    fn reach_diff(&mut self, old: Option<&Arc<Reach>>, new: &Arc<Reach>) {
        crate::stack::grow_stack!(self.reach_diff(old, new));
        let Some(old) = old else {
            return self.reach_add(new);
        };
        if Arc::ptr_eq(old, new) {
            return;
        }
        if old.direct != new.direct {
            for &slot in &new.direct {
                self.reach_inc(slot);
            }
            for &slot in &old.direct {
                self.reach_dec(slot);
            }
        }
        let common = old.children.len().min(new.children.len());
        for (o, n) in old.children.iter().zip(&new.children) {
            self.reach_diff(Some(o), n);
        }
        for child in &new.children[common..] {
            self.reach_add(child);
        }
        for child in &old.children[common..] {
            self.reach_remove(child);
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
pub(crate) struct Entry {
    pub output: Output,
    pub reach: Arc<Reach>,
    pub unresolved: Vec<String>,
    /// Its errors, those of the statements it reached kept apart: the same
    /// whatever was reported before, so any later pass can take them.
    pub errors: Arc<Vec<ErrorPart>>,
    /// How much deeper than its starting depth the materialization went.
    pub r_max: usize,
    /// How much it grew the result (`Ctx::size`), counted again on each reuse.
    pub size: usize,
    /// For a state default's value: the statement its errors were attributed
    /// to (the entry's), which a reuse must share. `None` for the rest.
    pub context: Option<String>,
}

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
}

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
        self.entries[i][Self::index(partial, expr)] = Some(Box::new(entry));
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

    /// `slot`'s statement changed: drop its entries and those of everything
    /// that references it, however indirectly.
    pub(crate) fn invalidate(&mut self, slot: Slot, table: &Table) {
        let mut pending = vec![slot];
        let mut seen = Vec::new();
        while let Some(s) = pending.pop() {
            if self.marked(s) {
                continue;
            }
            self.mark(&[s]);
            seen.push(s);
            if let Some(entries) = self.entries.get_mut(s as usize) {
                *entries = Default::default();
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
