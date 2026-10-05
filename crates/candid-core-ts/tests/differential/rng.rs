//! A seeded, dependency-free PRNG (SplitMix64). The whole corpus is a pure
//! function of the committed seeds, so this generator's output sequence is
//! part of the golden: changing it regenerates every case.

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x6a09_e667_f3bc_c908)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`; `0` when `n` is `0`.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// True with probability `num / den`.
    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        self.next_u64() % den < num
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// An index drawn by weight.
    pub fn weighted(&mut self, weights: &[u32]) -> usize {
        let total: u64 = weights.iter().map(|w| u64::from(*w)).sum();
        let mut roll = self.next_u64() % total;
        for (index, weight) in weights.iter().enumerate() {
            let weight = u64::from(*weight);
            if roll < weight {
                return index;
            }
            roll -= weight;
        }
        weights.len() - 1
    }

    pub fn byte(&mut self) -> u8 {
        (self.next_u64() & 0xff) as u8
    }
}
