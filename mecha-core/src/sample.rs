//! Reproducible random draws.
//!
//! Two places already needed one and each wrote its own: the mail corpus
//! grader, and the graph's queue sampler one repository over. This is the
//! third caller — prioritised replay's holdout — and a third copy of a
//! shuffle is a third place for the same bias to hide.
//!
//! **The seed is the caller's and gets printed.** A sample nobody can redraw
//! is a sample nobody can check, and these exist to produce numbers somebody
//! will quote.
//!
//! **Sort before you shuffle, or the seed is a lie.** The mail grader learned
//! this the expensive way: it shuffled a `HashMap`'s iteration order, which is
//! randomised per process, so two runs with the same seed graded different
//! samples while the flag documented itself as making a scorecard
//! reproducible. A deterministic shuffle of a nondeterministic order is
//! nondeterministic. Callers pass a slice whose order they control.
//!
//! The PRNG is a four-line LCG rather than a dependency: nothing here needs
//! cryptographic randomness, and `rand` would be a new dependency in a crate
//! that reads the owner's transcripts.

/// Fisher–Yates, seeded.
///
/// Backward, and over the whole vector — [`take_uniform`] then takes a prefix,
/// which is uniform because the shuffle was complete. Truncating an *unshuffled*
/// list is the bias this exists to escape.
pub fn shuffled<T>(mut v: Vec<T>, seed: u64) -> Vec<T> {
    let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as usize
    };
    for i in (1..v.len()).rev() {
        v.swap(i, next() % (i + 1));
    }
    v
}

/// A uniform draw of at most `k`, reproducible from `seed`.
pub fn take_uniform<T>(v: Vec<T>, seed: u64, k: usize) -> Vec<T> {
    shuffled(v, seed).into_iter().take(k).collect()
}

/// Seeds drawn in order from one seed: the same seed, the same stream.
///
/// A stream, never one value, because its callers redraw until they miss a
/// value (a redraw must not land on the picture's own seed), and a fixed
/// value would spin there forever. splitmix64: successive draws differ.
/// Harness-only: a replay stamps one so its arms draw paired seeds
/// (`ToolCtx::image_seeds`); nothing a model sends reaches it.
#[derive(Debug)]
pub struct SeedStream(std::sync::Mutex<u64>);

impl SeedStream {
    pub fn new(seed: u64) -> Self {
        SeedStream(std::sync::Mutex::new(seed))
    }

    /// The next draw.
    pub fn next(&self) -> u64 {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_stream_repeats_from_its_seed_and_never_repeats_a_draw_in_a_row() {
        let (a, b) = (SeedStream::new(7), SeedStream::new(7));
        let first: Vec<u64> = (0..5).map(|_| a.next()).collect();
        let second: Vec<u64> = (0..5).map(|_| b.next()).collect();
        assert_eq!(first, second);
        assert!(first.windows(2).all(|w| w[0] != w[1]));
        assert_ne!(SeedStream::new(8).next(), first[0]);
    }

    #[test]
    fn the_same_seed_draws_the_same_sample_and_a_different_one_does_not() {
        let items: Vec<u32> = (0..40).collect();
        assert_eq!(
            take_uniform(items.clone(), 42, 8),
            take_uniform(items.clone(), 42, 8)
        );
        assert_ne!(
            take_uniform(items.clone(), 42, 8),
            take_uniform(items, 43, 8)
        );
    }

    /// The property the draw exists for. Every item must appear in the drawn
    /// prefix at close to the same rate — this is what fails when a caller
    /// reaches for `truncate(k)` on an unshuffled list, which is the shape the
    /// graph's sampler had to escape and the reason this is not open-coded a
    /// third time.
    #[test]
    fn every_item_is_drawn_at_close_to_the_same_rate() {
        const N: usize = 10;
        const K: usize = 3;
        const DRAWS: u64 = 4_000;
        let mut seen = [0usize; N];
        for seed in 0..DRAWS {
            for i in take_uniform((0..N).collect::<Vec<_>>(), seed, K) {
                seen[i] += 1;
            }
        }
        let expected = (DRAWS as f64) * (K as f64) / (N as f64);
        for (i, count) in seen.iter().enumerate() {
            let drift = (*count as f64 - expected).abs() / expected;
            assert!(
                drift < 0.15,
                "item {i} drawn {count} times against an expected {expected:.0} ({:.1}% off)",
                drift * 100.0
            );
        }
    }

    #[test]
    fn a_draw_larger_than_the_pool_returns_the_pool() {
        assert_eq!(take_uniform(vec![1, 2, 3], 7, 99).len(), 3);
        assert!(take_uniform(Vec::<u8>::new(), 7, 5).is_empty());
    }
}
