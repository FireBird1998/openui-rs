//! Supplied identities carry measurements in the padding of the identity key.
use std::{
    collections::HashSet,
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
#[derive(Default)]
pub(super) struct InputIndex {
    entries: HashSet<Identity>,
}
impl InputIndex {
    pub(super) fn insert(&mut self, key: (u8, usize)) -> bool {
        self.entries.insert(Identity::new(key))
    }
    pub(super) fn contains(&self, key: &(u8, usize)) -> bool {
        self.entries.contains(&Identity::new(*key))
    }
    pub(super) fn measurement(&self, key: &(u8, usize)) -> Option<(Option<usize>, usize)> {
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
                entry.height = u16::try_from(height).unwrap_or(u16::MAX);
                if let Some(bytes) = bytes {
                    entry.bytes = u32::try_from(bytes).unwrap_or(0);
                }
            }
            self.entries.insert(entry);
        }
    }
}
