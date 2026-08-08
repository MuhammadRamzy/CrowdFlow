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
//! - [`world`] — Structure-of-Arrays agent storage. **Done.**
//! - [`locomotion`] — Social Force Model + position-based contacts. **Done.**
//! - [`calibration`] — measurement against published pedestrian data.
//! - [`density`] — smoothed crowd-density field. **Done.**
//! - [`sim`] — fixed-timestep step loop, routing and exits. **Done.**
//! - Flow fields, queues, groups, evacuation mode — next.

pub mod building;
pub mod calibration;
pub mod congestion;
pub mod density;
pub mod events;
pub mod fmath;
pub mod locomotion;
pub mod rng;
pub mod sim;
pub mod spatial;
pub mod world;

pub use density::{DensityGrid, BAND_COMFORTABLE, BAND_CRITICAL, BAND_RESTRICTED};
pub use locomotion::{LocomotionParams, LocomotionScratch};
pub use rng::{Rng, Stream};
pub use sim::{ExitSpan, Sim, SimParams, SimStats};
pub use spatial::SpatialGrid;
pub use world::{AgentId, AgentState, SpawnParams, World, NO_TRIANGLE};
