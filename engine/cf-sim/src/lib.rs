//! # cf-sim
//!
//! The deterministic agent-based crowd simulation core.
//!
//! Consumes a compiled `NavGraph` and steps a population of agents across it.
//! Knows nothing about documents, files or hosts: `cf-wasm` and `cf-native` are
//! thin shells around this crate, which is what makes "design in the browser,
//! stress-test on the server, get the same numbers" true rather than
//! aspirational.
//!
//! ## The determinism contract
//!
//! Same seed plus same inputs must produce **bit-identical** results on
//! x86-64, aarch64 and wasm32. Everything in this crate is written to that
//! constraint; see `docs/04-track-b-simulation-engine.md` §5 for the full
//! hazard list. The three that shape the code most:
//!
//! - **No stateful PRNG.** [`rng`] is counter-based, so a draw depends on
//!   *which* agent wants it, never on how many draws came before.
//! - **No hash iteration.** Anything iterated in the step loop is a slice or a
//!   sorted vector.
//! - **No `std` transcendentals** in the hot path. They go through `fmath` so
//!   the whole crate can move to a bit-reproducible `libm` in one edit.
//!
//! ## Status
//!
//! - [`rng`] — counter-based deterministic randomness. **Done.**
//! - [`spatial`] — uniform grid for neighbour queries. **Done.**
//! - ECS world, locomotion, contact resolution — next.

pub mod rng;
pub mod spatial;

pub use rng::{Rng, Stream};
pub use spatial::SpatialGrid;
