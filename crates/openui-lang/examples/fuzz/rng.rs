//! SplitMix64: small, fast, and the same everywhere, so a seed replays exactly.

pub struct Rng(u64);

impl Rng {
    /// The generator for program `k` of a run, so any one program can be replayed alone.
    pub fn for_program(seed: u64, k: u64) -> Rng {
        let mut mixer = Rng(seed ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        Rng(mixer.next_u64())
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n` (`n` > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// True `percent` percent of the time.
    pub fn chance(&mut self, percent: u64) -> bool {
        self.next_u64() % 100 < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}
