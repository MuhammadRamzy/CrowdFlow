//! Counter-based deterministic random numbers.
//!
//! # Why counter-based rather than a stateful generator
//!
//! A conventional PRNG holds mutable state, so the value an agent receives
//! depends on *how many draws happened before it*. That makes the result
//! depend on iteration order, which in turn makes it depend on whether a
//! system ran in parallel, how many threads it used, and whether an unrelated
//! population happened to sample one extra value. All of that breaks the
//! guarantee in `docs/04-track-b-simulation-engine.md` §5: same seed and same
//! inputs must give bit-identical results on x86-64, aarch64 and wasm32.
//!
//! This generator has no mutable state. Every draw is a pure function of
//! `(seed, stream, entity, index)`:
//!
//! ```text
//!     value = mix(seed, stream, entity, index)
//! ```
//!
//! So agent 4,912's walking speed is the same number whether it was sampled
//! first or last, on one thread or eight, and regardless of what any other
//! agent did. A parallel force pass and a serial one draw identical values by
//! construction rather than by careful ordering.
//!
//! # The mixing function
//!
//! SplitMix64's finalizer. It is a bijection on `u64` with good avalanche —
//! flipping one input bit flips about half the output bits — which is what
//! makes independent-looking streams out of adjacent counter values. It is
//! also four multiplies and some shifts, so it costs nothing in the hot loop.

/// Which quantity is being drawn.
///
/// Separating streams means adding a new sampled parameter cannot shift the
/// values every existing one receives. Never renumber these: doing so changes
/// every previously recorded run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Stream {
    /// Which entry an agent spawns at.
    SpawnPoint = 1,
    /// Spawn time within the arrival curve.
    SpawnTime = 2,
    DesiredSpeed = 3,
    Radius = 4,
    Mass = 5,
    GroupSize = 6,
    Patience = 7,
    ReactionTime = 8,
    Familiarity = 9,
    MobilityImpaired = 10,
    /// Which itinerary branch a probabilistic step takes.
    ItineraryBranch = 11,
    DwellTime = 12,
    ServiceTime = 13,
    /// Whether an agent obeys a one-way constraint or a sign.
    Compliance = 14,
    /// Jitter applied when several agents would otherwise spawn on one point.
    SpawnJitter = 15,
    /// Whether an agent re-routes when its route congests.
    RerouteChoice = 16,
}

/// SplitMix64's finalizer: a bijection on u64 with good avalanche.
#[inline]
fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A stateless source of reproducible randomness.
///
/// Cloning is free and clones are interchangeable — there is no state to
/// diverge, which is the entire point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng {
    seed: u64,
}

impl Rng {
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// A raw 64-bit draw.
    ///
    /// `entity` is normally an agent id; `index` distinguishes repeated draws
    /// of the same quantity for the same entity (e.g. successive service times).
    #[inline]
    pub fn u64(&self, stream: Stream, entity: u64, index: u64) -> u64 {
        // Combine the coordinates with odd constants before mixing so that
        // adjacent entities and indices do not land on adjacent inputs.
        let a = self
            .seed
            .wrapping_add((stream as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let b = a.wrapping_add(entity.wrapping_mul(0xD1B5_4A32_D192_ED03));
        let c = b.wrapping_add(index.wrapping_mul(0xA0761D6478BD642F));
        mix64(c)
    }

    /// Uniform in `[0, 1)`.
    ///
    /// Uses the top 53 bits, which is exactly the mantissa width of `f64`, so
    /// every representable value in the range is reachable and the result is
    /// bit-identical on every target.
    #[inline]
    pub fn uniform01(&self, stream: Stream, entity: u64, index: u64) -> f64 {
        let bits = self.u64(stream, entity, index) >> 11;
        bits as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in `[lo, hi)`.
    #[inline]
    pub fn uniform_range(&self, stream: Stream, entity: u64, index: u64, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.uniform01(stream, entity, index)
    }

    /// Uniform integer in `[0, n)`. Returns 0 when `n == 0`.
    ///
    /// Uses Lemire's multiply-shift rather than modulo. Modulo introduces a
    /// small bias toward low values; over 100,000 agents choosing between
    /// exits that bias is measurable, and it would show up as an unexplained
    /// asymmetry in an egress report.
    #[inline]
    pub fn below(&self, stream: Stream, entity: u64, index: u64, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        let x = self.u64(stream, entity, index) as u128;
        ((x * n as u128) >> 64) as u64
    }

    /// True with probability `p`.
    #[inline]
    pub fn chance(&self, stream: Stream, entity: u64, index: u64, p: f64) -> bool {
        self.uniform01(stream, entity, index) < p
    }

    /// Pick an index from `weights`, proportionally.
    ///
    /// Iterates in slice order, so the result depends only on the weights and
    /// the draw — never on hash iteration order.
    pub fn weighted(&self, stream: Stream, entity: u64, index: u64, weights: &[f64]) -> usize {
        let total: f64 = weights.iter().filter(|w| **w > 0.0).sum();
        if total <= 0.0 {
            return 0;
        }
        let target = self.uniform01(stream, entity, index) * total;
        let mut acc = 0.0;
        for (i, w) in weights.iter().enumerate() {
            if *w <= 0.0 {
                continue;
            }
            acc += w;
            if target < acc {
                return i;
            }
        }
        // Only reachable through floating-point round-off at the very top.
        weights.iter().rposition(|w| *w > 0.0).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_are_pure_functions_of_their_coordinates() {
        let r = Rng::new(20260803);
        let a = r.uniform01(Stream::DesiredSpeed, 4912, 0);
        let b = r.uniform01(Stream::DesiredSpeed, 4912, 0);
        assert_eq!(a, b, "the same coordinates must give the same value");

        // Order of access is irrelevant — that is the whole point.
        let forward: Vec<f64> = (0..100)
            .map(|i| r.uniform01(Stream::Radius, i, 0))
            .collect();
        let backward: Vec<f64> = (0..100)
            .rev()
            .map(|i| r.uniform01(Stream::Radius, i, 0))
            .collect();
        assert_eq!(
            forward,
            backward.into_iter().rev().collect::<Vec<_>>(),
            "iteration order changed the values"
        );
    }

    #[test]
    fn streams_are_independent() {
        let r = Rng::new(1);
        // The same entity and index in different streams must not collide.
        let speed = r.u64(Stream::DesiredSpeed, 7, 0);
        let radius = r.u64(Stream::Radius, 7, 0);
        let mass = r.u64(Stream::Mass, 7, 0);
        assert_ne!(speed, radius);
        assert_ne!(radius, mass);
        assert_ne!(speed, mass);
    }

    /// Adding a parameter must not disturb the ones already sampled — that is
    /// what separate streams buy, and it means a scenario edit does not
    /// silently change every other agent.
    #[test]
    fn adding_a_stream_does_not_disturb_existing_ones() {
        let r = Rng::new(42);
        let before: Vec<f64> = (0..50)
            .map(|i| r.uniform01(Stream::DesiredSpeed, i, 0))
            .collect();
        // Simulate "some other quantity now gets sampled too".
        let _noise: Vec<f64> = (0..50)
            .map(|i| r.uniform01(Stream::Patience, i, 0))
            .collect();
        let after: Vec<f64> = (0..50)
            .map(|i| r.uniform01(Stream::DesiredSpeed, i, 0))
            .collect();
        assert_eq!(before, after);
    }

    #[test]
    fn different_seeds_give_different_sequences() {
        let a = Rng::new(1);
        let b = Rng::new(2);
        let xs: Vec<f64> = (0..64)
            .map(|i| a.uniform01(Stream::SpawnTime, i, 0))
            .collect();
        let ys: Vec<f64> = (0..64)
            .map(|i| b.uniform01(Stream::SpawnTime, i, 0))
            .collect();
        assert_ne!(xs, ys);
    }

    #[test]
    fn uniform01_stays_in_range() {
        let r = Rng::new(7);
        for i in 0..20_000u64 {
            let v = r.uniform01(Stream::Compliance, i, 0);
            assert!((0.0..1.0).contains(&v), "out of range at {i}: {v}");
        }
    }

    #[test]
    fn uniform01_is_roughly_uniform() {
        let r = Rng::new(99);
        let n = 100_000u64;
        let mut buckets = [0u32; 10];
        let mut sum = 0.0;
        for i in 0..n {
            let v = r.uniform01(Stream::DwellTime, i, 0);
            sum += v;
            buckets[(v * 10.0) as usize] += 1;
        }
        let mean = sum / n as f64;
        assert!((mean - 0.5).abs() < 0.01, "mean {mean} is not near 0.5");

        let expected = n as f64 / 10.0;
        for (i, count) in buckets.iter().enumerate() {
            let dev = (*count as f64 - expected).abs() / expected;
            assert!(dev < 0.05, "bucket {i} deviates {:.1}%", dev * 100.0);
        }
    }

    #[test]
    fn below_stays_in_range_and_is_unbiased() {
        let r = Rng::new(5);
        assert_eq!(r.below(Stream::SpawnPoint, 0, 0, 0), 0, "n = 0 is safe");

        let n = 6u64;
        let mut counts = vec![0u32; n as usize];
        let draws = 120_000u64;
        for i in 0..draws {
            let v = r.below(Stream::SpawnPoint, i, 0, n);
            assert!(v < n);
            counts[v as usize] += 1;
        }
        let expected = draws as f64 / n as f64;
        for (i, c) in counts.iter().enumerate() {
            let dev = (*c as f64 - expected).abs() / expected;
            assert!(dev < 0.03, "outcome {i} deviates {:.1}%", dev * 100.0);
        }
    }

    #[test]
    fn chance_matches_its_probability() {
        let r = Rng::new(11);
        let n = 100_000u64;
        for p in [0.0, 0.05, 0.42, 0.5, 0.95, 1.0] {
            let hits = (0..n)
                .filter(|i| r.chance(Stream::Compliance, *i, 0, p))
                .count();
            let rate = hits as f64 / n as f64;
            assert!((rate - p).abs() < 0.01, "p = {p} produced {rate}");
        }
    }

    #[test]
    fn weighted_respects_its_weights() {
        let r = Rng::new(13);
        let weights = [0.6, 0.3, 0.1];
        let n = 100_000u64;
        let mut counts = [0u32; 3];
        for i in 0..n {
            counts[r.weighted(Stream::SpawnPoint, i, 0, &weights)] += 1;
        }
        for (i, w) in weights.iter().enumerate() {
            let rate = counts[i] as f64 / n as f64;
            assert!((rate - w).abs() < 0.01, "outcome {i}: {rate} vs {w}");
        }
    }

    #[test]
    fn weighted_ignores_zero_and_negative_weights() {
        let r = Rng::new(17);
        let weights = [0.0, 1.0, 0.0];
        for i in 0..1000 {
            assert_eq!(r.weighted(Stream::SpawnPoint, i, 0, &weights), 1);
        }
        // All-zero weights must not panic or loop.
        assert_eq!(r.weighted(Stream::SpawnPoint, 0, 0, &[0.0, 0.0]), 0);
        assert_eq!(r.weighted(Stream::SpawnPoint, 0, 0, &[]), 0);
    }

    /// Locks the exact output. If this fails, the generator changed and every
    /// previously recorded run is invalidated — which is sometimes correct, but
    /// must always be deliberate.
    ///
    /// These values were reproduced by an independent Python implementation of
    /// the same arithmetic, so they pin the *algorithm* rather than merely
    /// whatever this build happens to emit. That matters because the editor
    /// will need a TypeScript port for its histogram previews, and the two must
    /// agree exactly.
    #[test]
    fn output_is_pinned() {
        let r = Rng::new(20260803);
        assert_eq!(r.u64(Stream::DesiredSpeed, 0, 0), 12648797988929102362);
        assert_eq!(r.u64(Stream::DesiredSpeed, 1, 0), 4031595161593691641);
        assert_eq!(r.u64(Stream::Radius, 0, 0), 14576326153878759837);
    }

    /// The first 53-bit reduction is also pinned: a change to `uniform01`'s
    /// bit extraction would otherwise slip through the `u64` pins above.
    #[test]
    fn uniform01_is_pinned() {
        let r = Rng::new(20260803);
        let v = r.uniform01(Stream::DesiredSpeed, 0, 0);
        let expected = (12648797988929102362u64 >> 11) as f64 * (1.0 / (1u64 << 53) as f64);
        assert_eq!(v.to_bits(), expected.to_bits());
    }
}
