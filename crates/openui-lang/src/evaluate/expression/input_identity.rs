//! Supplied identities carry measurements in the padding of the identity key.
use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
};
#[derive(Clone, Copy)]
struct Identity {
    address: usize,
    bytes: u32,
    height: u16,
    kind: u8,
}
impl Identity {
    fn new(key: (u8, usize)) -> Self {
        Self {
            address: key.1,
            kind: key.0,
            bytes: 0,
            height: u16::MAX,
        }
    }
}
impl PartialEq for Identity {
    fn eq(&self, b: &Self) -> bool {
        self.address == b.address && self.kind == b.kind
    }
}
impl Eq for Identity {}
impl Hash for Identity {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.kind.hash(h);
        self.address.hash(h);
    }
}
type LargeMeasurements = HashMap<(u8, usize), (Option<usize>, usize)>;
#[derive(Default)]
pub(super) struct InputIndex {
    entries: HashSet<Identity>,
    // Rare large/deep measurements must remain exact too. Ordinary identities
    // retain their compact inline representation.
    large: Option<Box<LargeMeasurements>>,
}
impl InputIndex {
    pub(super) fn insert(&mut self, key: (u8, usize)) -> bool {
        self.entries.insert(Identity::new(key))
    }
    pub(super) fn contains(&self, key: &(u8, usize)) -> bool {
        self.entries.contains(&Identity::new(*key))
    }
    pub(super) fn measurement(&self, key: &(u8, usize)) -> Option<(Option<usize>, usize)> {
        if let Some(value) = self.large.as_deref().and_then(|m| m.get(key)) {
            return Some(*value);
        }
        let entry = self.entries.get(&Identity::new(*key))?;
        (entry.height != u16::MAX).then_some((
            (entry.bytes != 0).then_some(entry.bytes as usize),
            entry.height as usize,
        ))
    }
    pub(super) fn record(
        &mut self,
        key: (u8, usize),
        bytes: Option<usize>,
        height: usize,
        exact: bool,
    ) {
        if let Some(mut entry) = self.entries.take(&Identity::new(key)) {
            if exact {
                let prior = self.large.as_deref().and_then(|m| m.get(&key)).copied();
                let bytes = bytes
                    .or_else(|| prior.and_then(|v| v.0))
                    .or_else(|| (entry.bytes != 0).then_some(entry.bytes as usize));
                if height >= u16::MAX as usize || bytes.is_some_and(|b| b > u32::MAX as usize) {
                    self.large
                        .get_or_insert_with(Default::default)
                        .insert(key, (bytes, height));
                } else {
                    entry.height = height as u16;
                    entry.bytes = bytes.map_or(0, |b| b as u32);
                    if let Some(large) = &mut self.large {
                        large.remove(&key);
                    }
                }
            }
            self.entries.insert(entry);
        }
    }
}
