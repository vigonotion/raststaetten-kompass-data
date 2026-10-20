//! Own seeded PRNG (SplitMix64) for report samples and GPX noise. No external crate.

pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)` with 53 random bits.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform integer in `[0, n)` (modulo bias is irrelevant for samples).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Two independent standard normal values (Box-Muller).
    pub fn normal_pair(&mut self) -> (f64, f64) {
        let u1 = 1.0 - self.next_f64(); // (0, 1]
        let u2 = self.next_f64();
        let r = (-2.0 * u1.ln()).sqrt();
        let a = std::f64::consts::TAU * u2;
        (r * a.cos(), r * a.sin())
    }

    /// Chooses `k` distinct indices out of `0..n` (partial Fisher-Yates), in draw order.
    pub fn sample_indices(&mut self, n: usize, k: usize) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..n).collect();
        let k = k.min(n);
        for i in 0..k {
            let j = i + self.below(n - i);
            idx.swap(i, j);
        }
        idx.truncate(k);
        idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_splitmix_values() {
        // Reference values of SplitMix64 with seed 0.
        let mut r = SplitMix64::new(0);
        assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(r.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(r.next_u64(), 0x06C4_5D18_8009_454F);
    }

    #[test]
    fn deterministic_and_in_range() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        for _ in 0..100 {
            let x = a.next_f64();
            assert_eq!(x, b.next_f64());
            assert!((0.0..1.0).contains(&x));
        }
        let mut c = SplitMix64::new(43);
        assert_ne!(SplitMix64::new(42).next_u64(), c.next_u64());
    }

    #[test]
    fn normal_moments() {
        let mut r = SplitMix64::new(7);
        let n = 20_000;
        let mut sum = 0.0;
        let mut sq = 0.0;
        for _ in 0..n / 2 {
            let (a, b) = r.normal_pair();
            sum += a + b;
            sq += a * a + b * b;
        }
        let mean = sum / n as f64;
        let var = sq / n as f64 - mean * mean;
        assert!(mean.abs() < 0.03, "{mean}");
        assert!((var - 1.0).abs() < 0.05, "{var}");
    }

    #[test]
    fn sample_is_distinct() {
        let mut r = SplitMix64::new(20_261_019);
        let s = r.sample_indices(50, 20);
        assert_eq!(s.len(), 20);
        let mut t = s.clone();
        t.sort_unstable();
        t.dedup();
        assert_eq!(t.len(), 20);
        assert!(s.iter().all(|&i| i < 50));
        assert_eq!(r.sample_indices(3, 20).len(), 3);
    }
}
