//! Agent storage: Structure-of-Arrays with a hot/warm/cold split.
//!
//! # Why Structure-of-Arrays
//!
//! The step loop's dominant cost is the force pass, which reads position,
//! velocity and radius for every agent and its neighbours. Stored as an array
//! of structs, each agent would occupy ~200 bytes and the loop would drag all
//! of it through cache to reach the ~20 bytes it actually wants. Split into
//! parallel arrays, a tick streams only the fields it touches.
//!
//! # The hot/warm/cold split
//!
//! Not every field is read at the same rate, and mixing them defeats the point:
//!
//! - **hot** — read or written every tick by the force and integration passes.
//! - **warm** — read by behaviour systems that run every tick but touch only a
//!   fraction of agents (goal advance, patience, dwell).
//! - **cold** — spawn time, full itinerary, per-agent statistics. Read on
//!   spawn, on exit, and when analytics flush. Boxed away in its own vector so
//!   it never shares a cache line with anything hot.
//!
//! # Agent ids are stable for the whole run
//!
//! [`crate::rng`] keys every draw on the agent id, so an agent's sampled
//! walking speed *is* a function of its index. Compacting the arrays — swapping
//! the last agent into a departed agent's slot, the usual trick — would
//! renumber agents mid-run and silently change their parameters.
//!
//! So departure is a **tombstone**: `active[i] = false`, and the slot is never
//! reused. Memory is bounded by total spawns rather than peak occupancy, which
//! for a 100k-agent evacuation is a few megabytes — far cheaper than losing
//! reproducibility.

use cf_geom::Vec2;

/// Index of an agent. Stable for the lifetime of a run.
pub type AgentId = u32;

/// What an agent is currently doing. `#[repr(u8)]` so the array packs tightly
/// and can be handed to the renderer as a colour index without conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AgentState {
    /// Moving toward a goal.
    Walking = 0,
    /// Waiting in a queue for a component.
    Queuing = 1,
    /// Stationary at a destination for a dwell period.
    Dwelling = 2,
    /// Wants to move but is pinned by the crowd.
    Blocked = 3,
    /// Evacuating: heightened urgency, reduced personal space.
    Evacuating = 4,
    /// Has left the venue. Retains its slot; see the module docs.
    Exited = 5,
}

impl AgentState {
    pub fn is_mobile(self) -> bool {
        matches!(
            self,
            AgentState::Walking | AgentState::Blocked | AgentState::Evacuating
        )
    }
}

/// Fields read on spawn, on exit, and by analytics — never in the force loop.
#[derive(Clone, Debug)]
pub struct AgentCold {
    pub spawned_at: f64,
    pub exited_at: Option<f64>,
    /// Cumulative distance walked, for path-efficiency analytics.
    pub distance_walked: f64,
    /// Time spent below walking pace, for congestion analytics.
    pub time_delayed: f64,
    /// Which entry this agent came in through.
    pub entry: u16,
}

/// Everything needed to create an agent. Sampling happens in the spawner, so
/// this struct carries resolved values rather than distributions.
#[derive(Clone, Copy, Debug)]
pub struct SpawnParams {
    pub position: Vec2,
    pub radius_m: f32,
    pub desired_speed: f32,
    pub goal: u16,
    pub population: u16,
    pub entry: u16,
    pub state: AgentState,
}

/// The agent population.
#[derive(Clone, Debug, Default)]
pub struct World {
    // --- hot -----------------------------------------------------------
    pub pos_x: Vec<f32>,
    pub pos_y: Vec<f32>,
    pub vel_x: Vec<f32>,
    pub vel_y: Vec<f32>,
    /// Desired velocity from the navigation field, recomputed each tick.
    pub des_x: Vec<f32>,
    pub des_y: Vec<f32>,
    pub radius: Vec<f32>,
    pub desired_speed: Vec<f32>,
    pub state: Vec<AgentState>,
    pub active: Vec<bool>,

    // --- warm ----------------------------------------------------------
    pub goal: Vec<u16>,
    pub population: Vec<u16>,
    /// Current navmesh triangle, for O(1) mesh queries.
    pub tri: Vec<u32>,
    /// Seconds of patience remaining before an agent reconsiders its route.
    pub patience_left: Vec<f32>,

    // --- cold ----------------------------------------------------------
    pub cold: Vec<AgentCold>,

    // --- bookkeeping ----------------------------------------------------
    /// Simulation time in seconds.
    pub time: f64,
    pub tick: u64,
    exited: u32,
}

/// Sentinel for "not located in any triangle".
pub const NO_TRIANGLE: u32 = u32::MAX;

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(n: usize) -> Self {
        let mut w = Self::default();
        w.reserve(n);
        w
    }

    /// Pre-size every array. Spawning is then allocation-free, which keeps a
    /// large arrival surge from stalling a tick.
    pub fn reserve(&mut self, n: usize) {
        self.pos_x.reserve(n);
        self.pos_y.reserve(n);
        self.vel_x.reserve(n);
        self.vel_y.reserve(n);
        self.des_x.reserve(n);
        self.des_y.reserve(n);
        self.radius.reserve(n);
        self.desired_speed.reserve(n);
        self.state.reserve(n);
        self.active.reserve(n);
        self.goal.reserve(n);
        self.population.reserve(n);
        self.tri.reserve(n);
        self.patience_left.reserve(n);
        self.cold.reserve(n);
    }

    /// Total slots ever allocated, including departed agents.
    pub fn len(&self) -> usize {
        self.pos_x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pos_x.is_empty()
    }

    /// Agents currently in the venue.
    pub fn active_count(&self) -> u32 {
        self.len() as u32 - self.exited
    }

    /// Agents that have left.
    pub fn exited_count(&self) -> u32 {
        self.exited
    }

    /// Agents ever created.
    pub fn spawned_count(&self) -> u32 {
        self.len() as u32
    }

    pub fn is_active(&self, id: AgentId) -> bool {
        self.active.get(id as usize).copied().unwrap_or(false)
    }

    pub fn position(&self, id: AgentId) -> Vec2 {
        Vec2::new(
            self.pos_x[id as usize] as f64,
            self.pos_y[id as usize] as f64,
        )
    }

    pub fn velocity(&self, id: AgentId) -> Vec2 {
        Vec2::new(
            self.vel_x[id as usize] as f64,
            self.vel_y[id as usize] as f64,
        )
    }

    pub fn speed(&self, id: AgentId) -> f64 {
        self.velocity(id).length()
    }

    /// Create an agent. The returned id is its index, stable for the run.
    pub fn spawn(&mut self, p: SpawnParams) -> AgentId {
        let id = self.len() as AgentId;

        self.pos_x.push(p.position.x as f32);
        self.pos_y.push(p.position.y as f32);
        self.vel_x.push(0.0);
        self.vel_y.push(0.0);
        self.des_x.push(0.0);
        self.des_y.push(0.0);
        self.radius.push(p.radius_m);
        self.desired_speed.push(p.desired_speed);
        self.state.push(p.state);
        self.active.push(true);

        self.goal.push(p.goal);
        self.population.push(p.population);
        self.tri.push(NO_TRIANGLE);
        self.patience_left.push(f32::INFINITY);

        self.cold.push(AgentCold {
            spawned_at: self.time,
            exited_at: None,
            distance_walked: 0.0,
            time_delayed: 0.0,
            entry: p.entry,
        });

        id
    }

    /// Mark an agent as departed. Idempotent.
    ///
    /// The slot is retained — see the module docs on stable ids.
    pub fn despawn(&mut self, id: AgentId) {
        let i = id as usize;
        if i >= self.len() || !self.active[i] {
            return;
        }
        self.active[i] = false;
        self.state[i] = AgentState::Exited;
        self.vel_x[i] = 0.0;
        self.vel_y[i] = 0.0;
        self.cold[i].exited_at = Some(self.time);
        self.exited += 1;
    }

    /// Iterate active agent ids in ascending order.
    ///
    /// Ascending order is a determinism requirement, not a convenience: any
    /// system that accumulates across agents must do so in a fixed order.
    pub fn active_ids(&self) -> impl Iterator<Item = AgentId> + '_ {
        (0..self.len() as AgentId).filter(move |i| self.active[*i as usize])
    }

    /// Structural invariants. Empty means consistent.
    ///
    /// The population-conservation check is the one that matters most:
    /// silently losing agents would flatter every evacuation time we report.
    pub fn check_invariants(&self) -> Vec<String> {
        let mut errs = Vec::new();
        let n = self.len();

        let lengths = [
            ("pos_x", self.pos_x.len()),
            ("pos_y", self.pos_y.len()),
            ("vel_x", self.vel_x.len()),
            ("vel_y", self.vel_y.len()),
            ("des_x", self.des_x.len()),
            ("des_y", self.des_y.len()),
            ("radius", self.radius.len()),
            ("desired_speed", self.desired_speed.len()),
            ("state", self.state.len()),
            ("active", self.active.len()),
            ("goal", self.goal.len()),
            ("population", self.population.len()),
            ("tri", self.tri.len()),
            ("patience_left", self.patience_left.len()),
            ("cold", self.cold.len()),
        ];
        for (name, len) in lengths {
            if len != n {
                errs.push(format!("array '{name}' has {len} entries, expected {n}"));
            }
        }
        if !errs.is_empty() {
            return errs;
        }

        // Population conservation.
        let counted_active = self.active.iter().filter(|a| **a).count() as u32;
        if counted_active + self.exited != n as u32 {
            errs.push(format!(
                "population not conserved: {counted_active} active + {} exited != {n} spawned",
                self.exited
            ));
        }

        for i in 0..n {
            if self.active[i] && self.state[i] == AgentState::Exited {
                errs.push(format!("agent {i} is active but marked Exited"));
            }
            if !self.active[i] && self.state[i] != AgentState::Exited {
                errs.push(format!(
                    "agent {i} is inactive but state is {:?}",
                    self.state[i]
                ));
            }
            if !self.active[i] && self.cold[i].exited_at.is_none() {
                errs.push(format!("agent {i} departed without an exit time"));
            }
            if self.active[i] {
                if !self.pos_x[i].is_finite() || !self.pos_y[i].is_finite() {
                    errs.push(format!("agent {i} has a non-finite position"));
                }
                if !self.vel_x[i].is_finite() || !self.vel_y[i].is_finite() {
                    errs.push(format!("agent {i} has a non-finite velocity"));
                }
                if self.radius[i] <= 0.0 {
                    errs.push(format!("agent {i} has radius {}", self.radius[i]));
                }
            }
            if errs.len() > 32 {
                return errs;
            }
        }

        errs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(x: f64, y: f64) -> SpawnParams {
        SpawnParams {
            position: Vec2::new(x, y),
            radius_m: 0.23,
            desired_speed: 1.34,
            goal: 0,
            population: 0,
            entry: 0,
            state: AgentState::Walking,
        }
    }

    #[test]
    fn a_new_world_is_empty_and_consistent() {
        let w = World::new();
        assert_eq!(w.len(), 0);
        assert_eq!(w.active_count(), 0);
        assert_eq!(w.exited_count(), 0);
        assert!(w.check_invariants().is_empty());
    }

    #[test]
    fn spawning_assigns_sequential_ids() {
        let mut w = World::new();
        assert_eq!(w.spawn(params(1.0, 2.0)), 0);
        assert_eq!(w.spawn(params(3.0, 4.0)), 1);
        assert_eq!(w.spawn(params(5.0, 6.0)), 2);

        assert_eq!(w.len(), 3);
        assert_eq!(w.active_count(), 3);
        assert_eq!(w.position(1), Vec2::new(3.0, 4.0));
        assert!(w.check_invariants().is_empty());
    }

    #[test]
    fn despawning_retains_the_slot_and_conserves_population() {
        let mut w = World::new();
        for i in 0..10 {
            w.spawn(params(i as f64, 0.0));
        }
        w.time = 42.0;
        w.despawn(3);
        w.despawn(7);

        assert_eq!(w.len(), 10, "slots are retained, not removed");
        assert_eq!(w.active_count(), 8);
        assert_eq!(w.exited_count(), 2);
        assert_eq!(w.spawned_count(), 10);
        assert!(!w.is_active(3));
        assert!(w.is_active(4));
        assert_eq!(w.state[3], AgentState::Exited);
        assert_eq!(w.cold[3].exited_at, Some(42.0));
        assert!(w.check_invariants().is_empty());
    }

    /// Ids must not shift when an agent leaves. The RNG keys on agent id, so
    /// renumbering would silently change every surviving agent's parameters.
    #[test]
    fn ids_are_stable_across_departures() {
        let mut w = World::new();
        for i in 0..5 {
            w.spawn(params(i as f64 * 10.0, 0.0));
        }
        let before: Vec<Vec2> = (0..5).map(|i| w.position(i)).collect();

        w.despawn(0);
        w.despawn(2);

        for (i, p) in before.iter().enumerate() {
            assert_eq!(
                w.position(i as AgentId),
                *p,
                "agent {i} moved when another departed"
            );
        }
        // A later spawn takes a fresh slot rather than reusing a tombstone.
        assert_eq!(w.spawn(params(99.0, 99.0)), 5);
    }

    #[test]
    fn despawning_is_idempotent() {
        let mut w = World::new();
        w.spawn(params(0.0, 0.0));
        w.despawn(0);
        w.despawn(0);
        w.despawn(0);
        assert_eq!(w.exited_count(), 1, "double-despawn must not double-count");
        assert!(w.check_invariants().is_empty());
    }

    #[test]
    fn despawning_an_unknown_id_is_ignored() {
        let mut w = World::new();
        w.spawn(params(0.0, 0.0));
        w.despawn(999);
        assert_eq!(w.exited_count(), 0);
        assert!(w.check_invariants().is_empty());
    }

    #[test]
    fn active_ids_are_ascending_and_exclude_departures() {
        let mut w = World::new();
        for i in 0..8 {
            w.spawn(params(i as f64, 0.0));
        }
        w.despawn(1);
        w.despawn(5);

        let ids: Vec<AgentId> = w.active_ids().collect();
        assert_eq!(ids, vec![0, 2, 3, 4, 6, 7]);
        assert!(ids.windows(2).all(|p| p[0] < p[1]), "must be ascending");
    }

    #[test]
    fn invariants_catch_a_desynchronised_array() {
        let mut w = World::new();
        w.spawn(params(0.0, 0.0));
        w.radius.push(0.5); // one array now longer than the rest
        let errs = w.check_invariants();
        assert!(
            errs.iter().any(|e| e.contains("radius")),
            "expected a length mismatch: {errs:?}"
        );
    }

    #[test]
    fn invariants_catch_a_broken_population_count() {
        let mut w = World::new();
        for _ in 0..4 {
            w.spawn(params(0.0, 0.0));
        }
        // Deactivate without going through despawn — the bug this guards.
        w.active[2] = false;
        let errs = w.check_invariants();
        assert!(errs.iter().any(|e| e.contains("not conserved")), "{errs:?}");
    }

    #[test]
    fn invariants_catch_non_finite_state() {
        let mut w = World::new();
        w.spawn(params(0.0, 0.0));
        w.pos_x[0] = f32::NAN;
        let errs = w.check_invariants();
        assert!(
            errs.iter().any(|e| e.contains("non-finite position")),
            "{errs:?}"
        );

        let mut w = World::new();
        w.spawn(params(0.0, 0.0));
        w.vel_y[0] = f32::INFINITY;
        let errs = w.check_invariants();
        assert!(
            errs.iter().any(|e| e.contains("non-finite velocity")),
            "{errs:?}"
        );
    }

    #[test]
    fn spawn_records_the_current_time() {
        let mut w = World::new();
        w.time = 12.5;
        let a = w.spawn(params(0.0, 0.0));
        w.time = 30.0;
        let b = w.spawn(params(0.0, 0.0));

        assert_eq!(w.cold[a as usize].spawned_at, 12.5);
        assert_eq!(w.cold[b as usize].spawned_at, 30.0);
    }

    #[test]
    fn reserve_does_not_change_observable_state() {
        let mut w = World::with_capacity(1000);
        assert_eq!(w.len(), 0);
        w.spawn(params(1.0, 1.0));
        w.reserve(5000);
        assert_eq!(w.len(), 1);
        assert!(w.check_invariants().is_empty());
    }

    #[test]
    fn agent_state_classifies_mobility() {
        assert!(AgentState::Walking.is_mobile());
        assert!(AgentState::Blocked.is_mobile());
        assert!(AgentState::Evacuating.is_mobile());
        assert!(!AgentState::Queuing.is_mobile());
        assert!(!AgentState::Dwelling.is_mobile());
        assert!(!AgentState::Exited.is_mobile());
    }
}
