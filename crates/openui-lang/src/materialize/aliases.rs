//! Compact depth-cut traversal of acyclic aliases ending in a missing name.
//! Complete aliases keep the ordinary cache; cycles and non-alias values keep
//! the ordinary walker. Prices and the slot whose price fails remain unchanged.
use std::collections::HashMap;

use super::{Ctx, Mode, REFERENCE_SIZE, Resolved, cut_reference};
use crate::{
    ast::Ast,
    table::{Slot, Table},
};

#[derive(Clone, Copy)]
struct Tail {
    len: usize,
    next: Slot,
}
#[derive(Clone, Copy, Default)]
enum State {
    #[default]
    New,
    Open,
    Other,
    Tail(Tail),
}

pub(crate) struct Aliases {
    states: Vec<State>,
}
#[derive(Default)]
pub(crate) struct Marked {
    links: HashMap<Slot, Slot>,
}

#[inline]
fn visit() {
    #[cfg(test)]
    super::cost_tests::visit();
}
impl Aliases {
    pub(crate) fn new(slots: usize) -> Self {
        Self {
            states: vec![State::New; slots],
        }
    }
    fn tail(&mut self, slot: Slot, table: &Table) -> Option<Tail> {
        match self.states.get(slot as usize)? {
            State::Tail(t) => return Some(*t),
            State::Other | State::Open => return None,
            State::New => {}
        }
        self.build(slot, table)
    }
    #[inline(never)]
    fn build(&mut self, mut slot: Slot, table: &Table) -> Option<Tail> {
        let mut path = Vec::new();
        let mut tail = loop {
            visit();
            match self.states.get(slot as usize).copied() {
                Some(State::Tail(t)) => break Some(t),
                Some(State::New) => {}
                _ => break None,
            }
            let next = match table.get(slot).map(|s| s.expr()) {
                None => {
                    let t = Tail { len: 0, next: slot };
                    if let Some(state) = self.states.get_mut(slot as usize) {
                        *state = State::Tail(t);
                    }
                    break Some(t);
                }
                Some(Ast::Ref(name)) => table.slot(name),
                _ => None,
            };
            let Some(next) = next else {
                if let Some(state) = self.states.get_mut(slot as usize) {
                    *state = State::Other;
                }
                break None;
            };
            if let Some(state) = self.states.get_mut(slot as usize) {
                *state = State::Open;
            }
            path.push((slot, next));
            slot = next;
        };
        for (slot, next) in path.into_iter().rev() {
            visit();
            let state = match tail {
                Some(t) => {
                    let t = Tail {
                        len: t.len.saturating_add(1),
                        next,
                    };
                    tail = Some(t);
                    State::Tail(t)
                }
                None => State::Other,
            };
            if let Some(entry) = self.states.get_mut(slot as usize) {
                *entry = state;
            }
        }
        tail
    }
    fn mark(&self, marked: &mut Marked, reached: &mut Vec<Slot>, start: Slot, count: usize) {
        let Some(tail) = self.states.get(start as usize) else {
            return;
        };
        let State::Tail(tail) = tail else { return };
        let end = tail.len.saturating_sub(count);
        let mut slot = start;
        loop {
            slot = marked.find(slot);
            let Some(State::Tail(tail)) = self.states.get(slot as usize) else {
                break;
            };
            if tail.len <= end {
                break;
            }
            visit();
            // resolve_ref already recorded this call's initial lookup.
            if slot != start {
                reached.push(slot);
            }
            marked.links.insert(slot, tail.next);
            slot = tail.next;
        }
    }
}
impl Marked {
    fn find(&mut self, mut slot: Slot) -> Slot {
        let mut path = Vec::new();
        loop {
            visit();
            let Some(next) = self.links.get(&slot).copied() else {
                break;
            };
            path.push(slot);
            slot = next;
        }
        for old in path {
            self.links.insert(old, slot);
        }
        slot
    }
}

/// Jump only a depth-cut pure alias tail. A known missing terminal proves that
/// its path is acyclic and cannot contain an open non-alias ancestor. Per-frame
/// marks retain every reached slot once, including the reference whose price
/// fails. Complete tails use the ordinary shared cache and its identities.
#[inline(never)]
pub(super) fn resolve(slot: Slot, mode: Mode, ctx: &mut Ctx<'_>) -> Option<Resolved> {
    let table = ctx.table;
    let aliases = ctx
        .aliases
        .get_or_insert_with(|| Box::new(Aliases::new(table.slot_count())));
    let tail = aliases.tail(slot, table)?;
    let remaining_depth = ctx.limits.depth().saturating_sub(ctx.depth);
    if tail.len <= remaining_depth {
        return None;
    }
    let wanted = remaining_depth.saturating_add(1).min(tail.len);
    let room = ctx.limits.result_size().saturating_sub(ctx.size);
    let successful = (room / REFERENCE_SIZE).min(wanted);
    let size_cut = successful < wanted;
    let reached = successful + usize::from(size_cut);
    // The current Ref already entered at depth - 1. Each successful price
    // enters one target Ref, except the last one on a depth cut.
    ctx.frame.max_depth = ctx
        .frame
        .max_depth
        .max(ctx.depth.saturating_sub(1).saturating_add(successful));
    ctx.size += successful * REFERENCE_SIZE;
    ctx.frame.side_size += successful * REFERENCE_SIZE;
    if let Some(aliases) = ctx.aliases.as_ref() {
        let marks = ctx
            .frame
            .alias_marks
            .get_or_insert_with(|| Box::new(Marked::default()));
        aliases.mark(marks, &mut ctx.frame.reached, slot, reached);
    }
    if size_cut {
        // room / REFERENCE_SIZE proved this price cannot fit. Record the
        // same failure without overflowing at a usize::MAX ceiling.
        ctx.frame.size_limited = true;
        ctx.cuts += 1;
        return Some(cut_reference(ctx, mode));
    }
    ctx.frame.limited = true;
    ctx.cuts += 1;
    // enter() depth-cuts before reading the target or its unresolved terminal.
    Some(match mode {
        Mode::Value => {
            ctx.cut(super::NULL_VALUE);
            Resolved::Value(crate::Value::Null)
        }
        Mode::Expr => {
            ctx.cut(super::NULL_EXPR);
            Resolved::Ast(Ast::Null)
        }
    })
}
