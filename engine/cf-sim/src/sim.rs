//! The simulation: a fixed-timestep loop over a navmesh.
//!
//! # Fixed timestep
//!
//! The loop always advances by exactly `dt`, regardless of how long a tick took
//! to compute. Variable timesteps make results depend on machine speed, which
//! would break reproducibility outright — and would also change the physics,
//! since the contact solve's convergence depends on step size.
//!
//! Rendering interpolates between ticks; it does not drive them.
//!
//! # System order
//!
//! The order below is not arbitrary. Forces are computed from the *same*
//! spatial index every agent sees, so no agent reacts to a neighbour's
//! half-updated position. Contacts resolve after integration because they
//! correct where bodies actually ended up. Exits are checked last so an agent
//! that reached a door this tick leaves cleanly rather than being pushed back
//! in by a contact correction.
//!
//! # Routing
//!
//! Each agent caches a waypoint path computed once, and follows it. This is a
//! deliberate stepping stone: phase B4 replaces it with flow fields, which
//! compute one potential per *goal* rather than one path per *agent* — the
//! difference between O(goals × mesh) and O(agents × search), and the reason
//! 100k agents is feasible at all. Per-agent paths are fine at M1 scale and
//! let the locomotion model be exercised against real geometry now.

use crate::locomotion::{self, LocomotionParams, LocomotionScratch};
use crate::rng::Rng;
use crate::spatial::SpatialGrid;
use crate::world::{AgentId, AgentState, World};
use cf_geom::{Aabb, Segment, Vec2};
use cf_navmesh::NavMesh;

/// A doorway agents can leave through.
#[derive(Clone, Copy, Debug)]
pub struct ExitSpan {
    pub a: Vec2,
    pub b: Vec2,
}

impl ExitSpan {
    pub fn midpoint(&self) -> Vec2 {
        self.a.lerp(self.b, 0.5)
    }

    pub fn segment(&self) -> Segment {
        Segment::new(self.a, self.b)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SimParams {
    /// Physics timestep in seconds. 0.05 (20 Hz) is the design point; the
    /// contact solve is stable there and it keeps the force pass affordable.
    ///
    /// Held as `f64` even though the physics runs in `f32`: simulation time is
    /// derived from it and must not drift over a 90-minute run.
    pub dt: f64,
    pub locomotion: LocomotionParams,
    /// How close an agent must get to a doorway to be counted as having left.
    pub exit_radius: f32,
    /// Distance at which a waypoint counts as reached.
    pub waypoint_radius: f32,
    /// Speed below which a mobile agent is considered blocked, m/s.
    pub blocked_speed: f32,
}

impl Default for SimParams {
    fn default() -> Self {
        Self {
            dt: 0.05,
            locomotion: LocomotionParams::default(),
            exit_radius: 0.6,
            waypoint_radius: 0.5,
            blocked_speed: 0.1,
        }
    }
}

/// A cached route for one agent.
#[derive(Clone, Debug, Default)]
struct Route {
    points: Vec<Vec2>,
    next: usize,
}

impl Route {
    fn target(&self) -> Option<Vec2> {
        self.points.get(self.next).copied()
    }
}

/// Aggregate counters for one run.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SimStats {
    pub tick: u64,
    pub time: f64,
    pub active: u32,
    pub exited: u32,
    pub spawned: u32,
    /// Agents that wanted to move but could not, this tick.
    pub blocked: u32,
    /// Largest agent overlap remaining after the contact solve, metres. A
    /// persistently non-zero value means the solve is not converging.
    pub max_overlap: f32,
    /// Agents found outside the walkable mesh this tick and put back. Should be
    /// zero; anything else means the physics is leaking.
    pub escaped: u32,
}

/// A running simulation.
pub struct Sim {
    pub world: World,
    pub params: SimParams,
    pub rng: Rng,
    mesh: Option<NavMesh>,
    grid: SpatialGrid,
    scratch: LocomotionScratch,
    walls: Vec<Segment>,
    exits: Vec<ExitSpan>,
    routes: Vec<Route>,
    stats: SimStats,
}

impl Sim {
    /// A simulation over open space, with no geometry. Useful for locomotion
    /// experiments and for the fundamental-diagram harness.
    pub fn open(bounds: Aabb, params: SimParams, seed: u64) -> Self {
        let cell = params.locomotion.interaction_range as f64;
        Self {
            world: World::new(),
            params,
            rng: Rng::new(seed),
            mesh: None,
            grid: SpatialGrid::new(bounds, cell),
            scratch: LocomotionScratch::default(),
            walls: Vec::new(),
            exits: Vec::new(),
            routes: Vec::new(),
            stats: SimStats::default(),
        }
    }

    /// A simulation over a compiled navmesh.
    ///
    /// Wall segments for repulsion are taken from the mesh's constraint edges,
    /// so the physics and the pathfinding cannot disagree about where the walls
    /// are — they read the same data.
    pub fn new(mesh: NavMesh, exits: Vec<ExitSpan>, params: SimParams, seed: u64) -> Self {
        let mut walls = Vec::new();
        let mut keys: Vec<_> = mesh.tri.constraints.iter().copied().collect();
        // Sorted so wall force accumulates in a fixed order across runs.
        keys.sort_unstable();
        for (a, b) in keys {
            if a < mesh.tri.points.len() && b < mesh.tri.points.len() {
                walls.push(Segment::new(mesh.tri.points[a], mesh.tri.points[b]));
            }
        }

        let bounds = Aabb::of(mesh.tri.points.iter().copied()).unwrap_or(Aabb {
            min: Vec2::ZERO,
            max: Vec2::new(1.0, 1.0),
        });
        let cell = params.locomotion.interaction_range as f64;

        Self {
            world: World::new(),
            params,
            rng: Rng::new(seed),
            mesh: Some(mesh),
            grid: SpatialGrid::new(bounds, cell),
            scratch: LocomotionScratch::default(),
            walls,
            exits,
            routes: Vec::new(),
            stats: SimStats::default(),
        }
    }

    pub fn mesh(&self) -> Option<&NavMesh> {
        self.mesh.as_ref()
    }

    pub fn exits(&self) -> &[ExitSpan] {
        &self.exits
    }

    pub fn walls(&self) -> &[Segment] {
        &self.walls
    }

    pub fn stats(&self) -> SimStats {
        self.stats
    }

    /// Spawn an agent and immediately plan its route to `goal`.
    pub fn spawn(&mut self, params: crate::world::SpawnParams, goal: Vec2) -> AgentId {
        let id = self.world.spawn(params);
        let route = self.plan(params.position, goal);
        // Slots are never reused, so routes grow in lockstep with agents.
        debug_assert_eq!(self.routes.len(), id as usize);
        self.routes.push(route);
        id
    }

    /// Spawn an agent routed to whichever exit is nearest by walkable distance.
    ///
    /// "Nearest" means shortest *path*, not shortest straight line — a door on
    /// the far side of a wall is not near, and treating it as such is a classic
    /// way to under-report evacuation times.
    pub fn spawn_to_nearest_exit(&mut self, params: crate::world::SpawnParams) -> AgentId {
        let best = self.nearest_exit(params.position);
        match best {
            Some(g) => self.spawn(params, g),
            None => {
                let id = self.world.spawn(params);
                self.routes.push(Route::default());
                id
            }
        }
    }

    /// The exit reachable in the shortest walkable distance from `from`.
    pub fn nearest_exit(&self, from: Vec2) -> Option<Vec2> {
        let mut best: Option<(f64, Vec2)> = None;
        for e in &self.exits {
            let target = e.midpoint();
            let cost = match &self.mesh {
                Some(m) => match m.find_path(from, self.approach_point(*e)) {
                    Some(p) => cf_navmesh::path_length(&p),
                    None => continue,
                },
                None => from.distance(target),
            };
            if best.map(|(c, _)| cost < c).unwrap_or(true) {
                best = Some((cost, target));
            }
        }
        best.map(|(_, g)| g)
    }

    /// A point just inside a doorway.
    ///
    /// The doorway span itself lies on the mesh boundary, so `locate` may or
    /// may not place it in a walkable triangle. Aiming a little inside the
    /// threshold sidesteps that entirely.
    fn approach_point(&self, e: ExitSpan) -> Vec2 {
        let mid = e.midpoint();
        let along = (e.b - e.a).normalized().unwrap_or(Vec2::new(1.0, 0.0));
        let inward = along.perp() * 0.15;
        match &self.mesh {
            Some(m) if m.locate(mid + inward).is_some() => mid + inward,
            Some(m) if m.locate(mid - inward).is_some() => mid - inward,
            _ => mid,
        }
    }

    fn plan(&self, from: Vec2, goal: Vec2) -> Route {
        let points = match &self.mesh {
            Some(m) => m.find_path(from, goal).unwrap_or_else(|| vec![goal]),
            None => vec![goal],
        };
        // A pathfound route starts at the agent's own position, so skip it.
        // The single-point fallback *is* the goal, so do not.
        let next = usize::from(points.len() > 1);
        Route { points, next }
    }

    /// Advance one tick.
    pub fn step(&mut self) -> SimStats {
        let dt = self.params.dt as f32;

        // 1. One spatial index, shared by every agent this tick.
        self.grid
            .rebuild(&self.world.pos_x, &self.world.pos_y, &self.world.active);

        // 2. Follow routes: pick each agent's desired velocity.
        self.steer();

        // 3. Social forces, then walls.
        locomotion::compute_forces(
            &self.world,
            &self.grid,
            &self.params.locomotion,
            &mut self.scratch,
        );
        for wall in &self.walls {
            locomotion::add_wall_force(
                &self.world,
                wall.a,
                wall.b,
                &self.params.locomotion,
                &mut self.scratch,
            );
        }

        // 4. Integrate, clamp to walls, then project bodies apart and clamp
        //    again. Enforcing walls on both sides of the agent solve means a
        //    body never starts an iteration inside a wall, and the solve cannot
        //    leave it in one.
        locomotion::integrate(&mut self.world, &self.params.locomotion, &self.scratch, dt);
        locomotion::resolve_wall_contacts(&mut self.world, &self.walls, 1);
        let max_overlap = locomotion::resolve_contacts(
            &mut self.world,
            &self.grid,
            &self.params.locomotion,
            &mut self.scratch,
            dt,
        );

        // 5. Walls are hard constraints, applied after agent contacts so the
        //    crowd cannot push anyone through one. Soft repulsion alone loses
        //    agents through corners under load — see `resolve_wall_contacts`.
        let wall_pen = locomotion::resolve_wall_contacts(&mut self.world, &self.walls, 2);

        // 6. Safety net: the navmesh is the authority on where an agent may be.
        //    Wall projection alone cannot recover an agent that already escaped,
        //    because it pushes toward whichever side the agent is on. Anyone off
        //    the mesh is put back and counted — a non-zero count here means the
        //    physics is leaking and should be investigated, not tuned around.
        let escaped = self.recover_escaped();

        // 7. Classify motion, then let anyone at a door leave.
        locomotion::update_blocked_state(&mut self.world, self.params.blocked_speed);
        self.process_exits();

        // Derive time from the tick count rather than accumulating. Repeated
        // `time += dt` drifts — `0.05f32` is not exactly 0.05, and a 90-minute
        // run is over 100,000 additions of that error. Multiplying instead
        // keeps every timestamp exact, which matters because egress times are
        // read off these values.
        self.world.tick += 1;
        self.world.time = self.world.tick as f64 * self.params.dt;

        self.stats = SimStats {
            tick: self.world.tick,
            time: self.world.time,
            active: self.world.active_count(),
            exited: self.world.exited_count(),
            spawned: self.world.spawned_count(),
            blocked: self
                .world
                .state
                .iter()
                .zip(&self.world.active)
                .filter(|(s, a)| **a && **s == AgentState::Blocked)
                .count() as u32,
            max_overlap: max_overlap.max(wall_pen),
            escaped,
        };
        self.stats
    }

    /// Run until everyone has left or `max_ticks` elapse.
    ///
    /// Returns the tick count actually run. A caller that gets `max_ticks` back
    /// should treat the result as a non-completion, not a slow evacuation.
    pub fn run_until_empty(&mut self, max_ticks: u64) -> u64 {
        let mut n = 0;
        while n < max_ticks && self.world.active_count() > 0 {
            self.step();
            n += 1;
        }
        n
    }

    fn steer(&mut self) {
        let wp_r = self.params.waypoint_radius;
        for i in 0..self.world.len() {
            if !self.world.active[i] || !self.world.state[i].is_mobile() {
                continue;
            }
            let p = Vec2::new(self.world.pos_x[i] as f64, self.world.pos_y[i] as f64);
            let route = &mut self.routes[i];

            // Advance past every waypoint already reached — a fast agent can
            // clear more than one in a tick, and stopping at the first would
            // make it double back.
            //
            // The *final* waypoint is never passed. An agent that has arrived
            // but not yet left must keep pressing toward the exit: if it simply
            // stopped, one jammed at a doorway would stand there forever,
            // just outside the exit radius, and never leave. People at a door
            // keep pushing, and so must this.
            while route.next + 1 < route.points.len() {
                if p.distance(route.points[route.next]) <= wp_r as f64 {
                    route.next += 1;
                } else {
                    break;
                }
            }

            match route.target() {
                Some(t) => {
                    let d = t - p;
                    let dist = d.length();
                    if dist > 1e-6 {
                        let s = self.world.desired_speed[i] as f64 / dist;
                        self.world.des_x[i] = (d.x * s) as f32;
                        self.world.des_y[i] = (d.y * s) as f32;
                    } else {
                        self.world.des_x[i] = 0.0;
                        self.world.des_y[i] = 0.0;
                    }
                }
                None => {
                    self.world.des_x[i] = 0.0;
                    self.world.des_y[i] = 0.0;
                }
            }
        }
    }

    /// Put any agent that ended up off the mesh back onto it.
    fn recover_escaped(&mut self) -> u32 {
        let Some(mesh) = &self.mesh else {
            return 0;
        };
        let mut fixes: Vec<(usize, Vec2)> = Vec::new();
        for i in 0..self.world.len() {
            if !self.world.active[i] {
                continue;
            }
            let p = Vec2::new(self.world.pos_x[i] as f64, self.world.pos_y[i] as f64);
            if mesh.locate(p).is_some() {
                continue;
            }
            if let Some(c) = mesh.nearest_walkable_point(p) {
                fixes.push((i, c));
            }
        }
        let n = fixes.len() as u32;
        for (i, c) in fixes {
            self.world.pos_x[i] = c.x as f32;
            self.world.pos_y[i] = c.y as f32;
            // Zero the velocity: whatever it was, it took the agent through a
            // wall, and keeping it would push straight back out.
            self.world.vel_x[i] = 0.0;
            self.world.vel_y[i] = 0.0;
        }
        n
    }

    fn process_exits(&mut self) {
        if self.exits.is_empty() {
            return;
        }
        let r = self.params.exit_radius as f64;
        let mut leaving: Vec<AgentId> = Vec::new();

        for i in 0..self.world.len() {
            if !self.world.active[i] {
                continue;
            }
            let p = Vec2::new(self.world.pos_x[i] as f64, self.world.pos_y[i] as f64);
            let at_door = self
                .exits
                .iter()
                .any(|e| e.segment().distance_to_point(p) <= r);
            if at_door {
                leaving.push(i as AgentId);
            }
        }

        // Ascending order, so the exit log is reproducible.
        for id in leaving {
            self.world.despawn(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::SpawnParams;

    fn agent(x: f64, y: f64) -> SpawnParams {
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

    fn open_sim() -> Sim {
        Sim::open(
            Aabb {
                min: Vec2::new(-50.0, -50.0),
                max: Vec2::new(50.0, 50.0),
            },
            SimParams::default(),
            20260803,
        )
    }

    #[test]
    fn a_fresh_sim_has_no_agents() {
        let s = open_sim();
        assert_eq!(s.stats().active, 0);
        assert_eq!(s.world.tick, 0);
    }

    #[test]
    fn time_advances_by_exactly_dt() {
        let mut s = open_sim();
        s.spawn(agent(0.0, 0.0), Vec2::new(5.0, 0.0));
        for i in 1..=10u64 {
            let st = s.step();
            assert_eq!(st.tick, i);
            assert!(
                (st.time - i as f64 * 0.05).abs() < 1e-12,
                "tick {i} reached t = {}",
                st.time
            );
        }
    }

    #[test]
    fn an_agent_walks_to_its_goal() {
        let mut s = open_sim();
        s.spawn(agent(0.0, 0.0), Vec2::new(10.0, 0.0));
        for _ in 0..200 {
            s.step();
        }
        let p = s.world.position(0);
        assert!(p.distance(Vec2::new(10.0, 0.0)) < 0.6, "ended at {p:?}");
        assert!(s.world.check_invariants().is_empty());
    }

    #[test]
    fn reaching_an_exit_removes_the_agent() {
        let mut s = Sim::open(
            Aabb {
                min: Vec2::new(-20.0, -20.0),
                max: Vec2::new(20.0, 20.0),
            },
            SimParams::default(),
            1,
        );
        s.exits = vec![ExitSpan {
            a: Vec2::new(10.0, -1.0),
            b: Vec2::new(10.0, 1.0),
        }];
        s.spawn(agent(0.0, 0.0), Vec2::new(10.0, 0.0));

        let ticks = s.run_until_empty(600);
        assert!(ticks < 600, "agent never reached the exit");
        assert_eq!(s.stats().active, 0);
        assert_eq!(s.stats().exited, 1);
        assert_eq!(s.stats().spawned, 1);
        assert!(s.world.check_invariants().is_empty());
    }

    #[test]
    fn population_is_conserved_throughout() {
        let mut s = Sim::open(
            Aabb {
                min: Vec2::new(-20.0, -20.0),
                max: Vec2::new(20.0, 20.0),
            },
            SimParams::default(),
            2,
        );
        s.exits = vec![ExitSpan {
            a: Vec2::new(12.0, -1.5),
            b: Vec2::new(12.0, 1.5),
        }];
        for i in 0..40 {
            let x = -(i % 8) as f64 * 0.6;
            let y = (i / 8) as f64 * 0.6 - 1.2;
            s.spawn(agent(x, y), Vec2::new(12.0, 0.0));
        }

        for _ in 0..500 {
            let st = s.step();
            assert_eq!(
                st.active + st.exited,
                st.spawned,
                "population leaked at tick {}",
                st.tick
            );
        }
        assert!(s.world.check_invariants().is_empty());
    }

    #[test]
    fn a_run_is_bit_reproducible() {
        let build = || {
            let mut s = Sim::open(
                Aabb {
                    min: Vec2::new(-30.0, -30.0),
                    max: Vec2::new(30.0, 30.0),
                },
                SimParams::default(),
                777,
            );
            for i in 0..50 {
                let x = (i % 10) as f64 * 0.5;
                let y = (i / 10) as f64 * 0.5;
                s.spawn(agent(x, y), Vec2::new(20.0, 3.0));
            }
            s
        };

        let mut a = build();
        let mut b = build();
        for _ in 0..150 {
            a.step();
            b.step();
        }

        assert_eq!(a.stats(), b.stats());
        for i in 0..a.world.len() {
            assert_eq!(
                a.world.pos_x[i].to_bits(),
                b.world.pos_x[i].to_bits(),
                "agent {i}"
            );
            assert_eq!(a.world.pos_y[i].to_bits(), b.world.pos_y[i].to_bits());
        }
    }

    #[test]
    fn run_until_empty_stops_early_when_everyone_leaves() {
        let mut s = Sim::open(
            Aabb {
                min: Vec2::new(-20.0, -20.0),
                max: Vec2::new(20.0, 20.0),
            },
            SimParams::default(),
            3,
        );
        s.exits = vec![ExitSpan {
            a: Vec2::new(5.0, -1.0),
            b: Vec2::new(5.0, 1.0),
        }];
        s.spawn(agent(0.0, 0.0), Vec2::new(5.0, 0.0));

        let ticks = s.run_until_empty(10_000);
        assert!(ticks > 0 && ticks < 300, "took {ticks} ticks");
        assert_eq!(s.world.active_count(), 0);
    }

    #[test]
    fn an_agent_with_no_reachable_exit_is_not_lost() {
        let mut s = open_sim();
        // No exits configured at all.
        s.spawn_to_nearest_exit(agent(0.0, 0.0));
        for _ in 0..50 {
            s.step();
        }
        assert_eq!(s.stats().active, 1, "the agent must still be accounted for");
        assert!(s.world.check_invariants().is_empty());
    }

    #[test]
    fn stats_track_blocked_agents() {
        let mut s = open_sim();
        // A tight wedge: many agents converging on one point will jam.
        for i in 0..30 {
            s.spawn(
                agent((i % 6) as f64 * 0.4, (i / 6) as f64 * 0.4),
                Vec2::new(6.0, 0.6),
            );
        }
        let mut saw_blocked = false;
        for _ in 0..120 {
            if s.step().blocked > 0 {
                saw_blocked = true;
            }
        }
        assert!(saw_blocked, "a jam should register blocked agents");
    }
}
