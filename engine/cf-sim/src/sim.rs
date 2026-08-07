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

use crate::density::DensityGrid;
use crate::locomotion::{self, LocomotionParams, LocomotionScratch};
use crate::rng::{Rng, Stream};
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
    /// Distance at which a waypoint counts as reached.
    pub waypoint_radius: f32,
    /// Speed below which a mobile agent is considered blocked, m/s.
    pub blocked_speed: f32,
    /// Mean seconds between an agent reconsidering which exit it is heading
    /// for. Zero disables congestion-aware rerouting entirely.
    ///
    /// Reconsidering costs a path query per exit, so this is a direct
    /// performance lever as well as a behavioural one. Eight seconds is long
    /// enough to be affordable and short enough that a crowd redistributes
    /// while it still matters.
    pub reroute_interval_s: f32,
}

impl Default for SimParams {
    fn default() -> Self {
        Self {
            dt: 0.05,
            locomotion: LocomotionParams::default(),
            waypoint_radius: 0.5,
            blocked_speed: 0.1,
            reroute_interval_s: 8.0,
        }
    }
}

/// A cached route for one agent.
#[derive(Clone, Debug, Default)]
struct Route {
    points: Vec<Vec2>,
    next: usize,
    /// Where this route was planned to, so it can be planned again.
    goal: Vec2,
    /// Whether that goal is a way out.
    ///
    /// Congestion-aware rerouting may only touch agents that are trying to
    /// leave. An agent walking to a zone — a stand, a bar, a seat — has a goal
    /// of its own, and re-routing it to whichever door is least busy is not
    /// re-routing, it is overriding. That is exactly what happened: a crowd
    /// authored to dwell in a hall was hijacked to the exits within one
    /// reconsideration interval and the venue emptied itself with no alarm.
    to_exit: bool,
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
    /// Consecutive ticks each agent has wanted to move and failed to.
    stuck_ticks: Vec<u16>,
    /// Last known triangle per agent, so terrain lookup is a local walk rather
    /// than a scan of the whole mesh. Only maintained when the mesh has
    /// non-uniform walking speeds.
    tri_hint: Vec<usize>,
    /// Diagnostics from the most recent tick only.
    last_solve: SolveDiagnostics,
    density: DensityGrid,
    /// Ticks between density recomputations. The field changes far slower than
    /// agents move, and recomputing it every tick would dominate the loop for a
    /// picture nobody can read at 20 Hz.
    density_interval: u64,
}

/// Per-tick solver health, which cannot be derived from world state.
#[derive(Clone, Copy, Debug, Default)]
struct SolveDiagnostics {
    max_overlap: f32,
    escaped: u32,
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
            stuck_ticks: Vec::new(),
            tri_hint: Vec::new(),
            last_solve: SolveDiagnostics::default(),
            density: DensityGrid::new(bounds, 0.5),
            density_interval: 4,
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
            stuck_ticks: Vec::new(),
            tri_hint: Vec::new(),
            last_solve: SolveDiagnostics::default(),
            density: DensityGrid::new(bounds, 0.5),
            density_interval: 4,
        }
    }

    /// The crowd-density field. Recomputed every few ticks; see
    /// `density_interval`.
    pub fn density(&self) -> &DensityGrid {
        &self.density
    }

    pub fn mesh(&self) -> Option<&NavMesh> {
        self.mesh.as_ref()
    }

    pub fn exits(&self) -> &[ExitSpan] {
        &self.exits
    }

    /// Shut a doorway mid-run.
    ///
    /// *What if this exit is blocked* is the question a fire-safety engineer
    /// actually asks, and it is the one thing a static analysis cannot answer.
    ///
    /// Dropping the span from `exits` is not enough. The doorway edge is
    /// *unconstrained* in the triangulation — that is what makes it a doorway —
    /// so agents would keep walking through the gap and straight off the mesh,
    /// where the escape recovery would drag them back in a loop. The edge has
    /// to become a wall: put it back in the constraint set, rebuild adjacency
    /// and portals so pathfinding stops routing through it, and add the segment
    /// to the wall list so the physics pushes bodies off it.
    ///
    /// Every route is then re-planned, because any of them may have been
    /// threaded through the opening that just closed. That is O(agents × search)
    /// in one tick, which is affordable for an event that happens a handful of
    /// times in a run and would not be if it happened every tick.
    ///
    /// Returns false if the index is out of range or the edge is not in the
    /// mesh — a caller asking to close something that does not exist should
    /// find out, not be quietly ignored.
    pub fn close_exit(&mut self, index: usize) -> bool {
        if index >= self.exits.len() {
            return false;
        }
        let span = self.exits[index];
        let Some(mesh) = self.mesh.as_mut() else {
            return false;
        };

        // Find the mesh vertices at the ends of the span.
        let find = |p: Vec2| mesh.tri.points.iter().position(|q| q.distance(p) < 1e-6);
        let (Some(a), Some(b)) = (find(span.a), find(span.b)) else {
            return false;
        };

        mesh.tri.constraints.insert(cf_navmesh::edge_key(a, b));
        mesh.tri.rebuild_adjacency();
        // Portals are derived from adjacency, so the mesh has to be rebuilt for
        // pathfinding to see the new wall.
        let rebuilt = NavMesh::with_regions(mesh.tri.clone(), mesh.regions.clone());
        self.mesh = Some(rebuilt);

        self.walls.push(span.segment());
        self.exits.remove(index);

        // Anyone routed through the closed door needs a new plan. Agents whose
        // route did not touch it get the same answer back.
        let goals: Vec<(usize, Vec2, f64)> = (0..self.world.len())
            .filter(|i| self.world.active[*i])
            .map(|i| {
                (
                    i,
                    Vec2::new(self.world.pos_x[i] as f64, self.world.pos_y[i] as f64),
                    self.world.radius[i] as f64,
                )
            })
            .collect();
        for (i, from, r) in goals {
            // With no exit left reachable the old route is kept rather than
            // cleared: a stale target at least keeps the agent walking toward
            // where a door used to be, which is what a person who has not yet
            // noticed would do. Clearing it would freeze them on the spot.
            if let Some(g) = self.nearest_exit(from) {
                self.routes[i] = self.plan(from, g, r, true);
            }
            self.stuck_ticks[i] = 0;
        }
        true
    }

    /// Send everyone to the nearest exit, now.
    ///
    /// What an alarm does. Until one sounds, a population with a zone goal
    /// walks to that zone and stays there — which is the point, because a venue
    /// full of people who are not yet trying to leave is the state an
    /// evacuation starts from, and an analysis that begins with everyone
    /// already heading for a door skips the part where they have to notice.
    ///
    /// Agents already routed to an exit get the same answer back, so sounding
    /// an alarm in a scenario that was always an evacuation costs a re-plan and
    /// changes nothing.
    ///
    /// Returns how many were re-routed.
    pub fn evacuate_all(&mut self) -> u32 {
        let targets: Vec<(usize, Vec2, f64)> = (0..self.world.len())
            .filter(|i| self.world.active[*i])
            .map(|i| {
                (
                    i,
                    Vec2::new(self.world.pos_x[i] as f64, self.world.pos_y[i] as f64),
                    self.world.radius[i] as f64,
                )
            })
            .collect();

        let mut moved = 0;
        for (i, from, r) in targets {
            if let Some(g) = self.nearest_exit(from) {
                self.routes[i] = self.plan(from, g, r, true);
                moved += 1;
            }
            // Dwelling agents are not mobile, so nothing would steer them.
            if self.world.state[i] == AgentState::Dwelling {
                self.world.state[i] = AgentState::Evacuating;
            }
            self.stuck_ticks[i] = 0;
            // Reconsider promptly rather than up to a full interval later: a
            // crowd that has just been told to leave does not wait eight
            // seconds before looking at which door is busiest.
            self.world.patience_left[i] = 0.0;
        }
        moved
    }

    /// Where agent `i` is trying to get to, if it has a route.
    pub fn goal_of(&self, i: usize) -> Option<Vec2> {
        self.routes.get(i).map(|r| r.goal)
    }

    /// Point an existing agent at a new goal.
    ///
    /// `to_exit` says whether that goal is a way out; only agents heading for
    /// one are subject to congestion-aware rerouting, so a goal that is a
    /// staircase must say so or it will be overridden by whichever door
    /// happens to be quiet.
    pub fn retarget(&mut self, i: usize, goal: Vec2, to_exit: bool) {
        if i >= self.routes.len() || !self.world.active[i] {
            return;
        }
        let from = Vec2::new(self.world.pos_x[i] as f64, self.world.pos_y[i] as f64);
        let r = self.world.radius[i] as f64;
        self.routes[i] = self.plan(from, goal, r, to_exit);
        self.stuck_ticks[i] = 0;
        if self.world.state[i] == AgentState::Dwelling {
            self.world.state[i] = AgentState::Evacuating;
        }
    }

    /// The spatial index as of the last step.
    ///
    /// Exposed so a harness can ask the locomotion model what an agent senses
    /// without rebuilding an index that would not match the one the forces
    /// actually used.
    pub fn spatial_grid(&self) -> &SpatialGrid {
        &self.grid
    }

    pub fn walls(&self) -> &[Segment] {
        &self.walls
    }

    /// Current counters.
    ///
    /// Computed from the world rather than returned from a cache. A cached
    /// value is wrong in exactly the case that matters most: immediately after
    /// spawning, before the first tick, when a UI asks "what did I just
    /// create?" and a stale struct answers "nothing".
    pub fn stats(&self) -> SimStats {
        SimStats {
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
            // These two describe the most recent tick, so they do come from the
            // last step; before any step they are legitimately zero.
            max_overlap: self.last_solve.max_overlap,
            escaped: self.last_solve.escaped,
        }
    }

    /// Spawn an agent and immediately plan its route to `goal`.
    /// Spawn an agent and plan its route to `goal`.
    ///
    /// `to_exit` says whether that goal is a way out. Only agents heading for
    /// one are subject to congestion-aware rerouting — see `Route::to_exit`.
    pub fn spawn_toward(
        &mut self,
        params: crate::world::SpawnParams,
        goal: Vec2,
        to_exit: bool,
    ) -> AgentId {
        let id = self.world.spawn(params);
        let route = self.plan(params.position, goal, params.radius_m as f64, to_exit);
        // Slots are never reused, so routes grow in lockstep with agents.
        debug_assert_eq!(self.routes.len(), id as usize);
        self.routes.push(route);
        self.stuck_ticks.push(0);
        self.tri_hint.push(0);
        // `World` starts patience at infinity, meaning "never reconsiders".
        // Give it a finite, staggered value so the crowd does not all
        // re-evaluate on the same tick and oscillate between doors.
        let u = self.rng.uniform01(Stream::RerouteChoice, id as u64, 0) as f32;
        self.world.patience_left[id as usize] = self.params.reroute_interval_s * (0.5 + u);
        id
    }

    /// An agent's planned route, and which waypoint it is steering toward.
    ///
    /// Exposed for diagnostics and for drawing paths in the editor: when an
    /// agent misbehaves, the first question is always "where does it think it
    /// is going", and inferring that from positions alone is guesswork.
    pub fn route_of(&self, id: AgentId) -> (&[Vec2], usize) {
        let r = &self.routes[id as usize];
        (&r.points, r.next)
    }

    /// Recompute the density field now, without stepping.
    ///
    /// Called after spawning so a placed crowd is visible before the first
    /// tick — otherwise the heatmap is blank until playback starts, which
    /// reads as a broken feature rather than an empty one.
    pub fn refresh_density(&mut self) {
        self.density.accumulate(&self.world);
    }

    /// Spawn an agent routed to whichever exit is nearest by walkable distance.
    ///
    /// "Nearest" means shortest *path*, not shortest straight line — a door on
    /// the far side of a wall is not near, and treating it as such is a classic
    /// way to under-report evacuation times.
    pub fn spawn_to_nearest_exit(&mut self, params: crate::world::SpawnParams) -> AgentId {
        let best = self.nearest_exit(params.position);
        match best {
            Some(g) => self.spawn_toward(params, g, true),
            None => {
                let id = self.world.spawn(params);
                self.routes.push(Route::default());
                self.stuck_ticks.push(0);
                self.tri_hint.push(0);
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

    /// Nudge interior waypoints off the corners they sit on.
    ///
    /// The funnel returns the tautest path, which means its waypoints are the
    /// obstacle vertices themselves. A body of radius r cannot stand on a
    /// vertex, so an agent steering at one drives into the corner until contact
    /// resolution stops it, and if the wall it is pressing into runs the way it
    /// wants to go, it never gets round: RiMEA TC3 and TC6 each wedged an agent
    /// at the same apex on every run.
    ///
    /// Each interior waypoint moves along the bisector of its two legs, which
    /// points into the free space away from the corner. `√2 · r` is what a right
    /// angle needs to clear both walls, and right angles are what buildings are
    /// made of; anything sharper is under-corrected rather than over. A nudge
    /// that would leave the mesh is dropped, so a corridor too narrow to ease
    /// through keeps the taut path instead of being handed an unreachable goal.
    fn ease_corners(&self, points: &mut [Vec2], radius: f64) {
        let Some(mesh) = &self.mesh else {
            return;
        };
        if points.len() < 3 || radius <= 0.0 {
            return;
        }
        let offset = radius * std::f64::consts::SQRT_2;

        for k in 1..points.len() - 1 {
            let wp = points[k];
            let u = points[k - 1] - wp;
            let v = points[k + 1] - wp;
            let (ul, vl) = (u.length(), v.length());
            if ul <= 1e-9 || vl <= 1e-9 {
                continue;
            }
            // `u + v` bisects the narrow angle between the two legs. The funnel
            // pulls a path taut *around* an obstacle, so that narrow side is the
            // obstacle — stepping along it walks into the wall. Free space is
            // the other way.
            let inward = u * (1.0 / ul) + v * (1.0 / vl);
            let bl = inward.length();
            // Collinear: no corner to ease.
            if bl <= 1e-6 {
                continue;
            }
            let eased = wp - inward * (offset / bl);
            if mesh.locate(eased).is_some() {
                points[k] = eased;
            }
        }
    }

    fn plan(&self, from: Vec2, goal: Vec2, radius: f64, to_exit: bool) -> Route {
        let mut points = match &self.mesh {
            Some(m) => m.find_path(from, goal).unwrap_or_else(|| vec![goal]),
            None => vec![goal],
        };
        // The funnel can emit the same corner twice. A duplicate is not merely
        // redundant: `steer` advances past every waypoint already reached, so
        // two coincident points are consumed in a single tick, and an agent
        // 0.6 m short of a corner skipped the whole turn and aimed at the exit
        // through a wall.
        points.dedup_by(|a, b| a.distance(*b) < 1e-6);
        self.ease_corners(&mut points, radius);
        // A pathfound route starts at the agent's own position, so skip it.
        // The single-point fallback *is* the goal, so do not.
        let next = usize::from(points.len() > 1);
        Route {
            points,
            next,
            goal,
            to_exit,
        }
    }

    /// Advance one tick.
    pub fn step(&mut self) -> SimStats {
        let dt = self.params.dt as f32;

        // 1. One spatial index, shared by every agent this tick.
        self.grid
            .rebuild(&self.world.pos_x, &self.world.pos_y, &self.world.active);

        // 2. Follow routes: pick each agent's desired velocity.
        self.steer();
        self.apply_terrain_speed();

        // 3. Local density suppresses desired speed. Applied after steering so
        //    it scales a direction that already exists, and before forces so the
        //    driving term targets the reduced velocity.
        locomotion::apply_density_speed_limit(
            &mut self.world,
            &self.grid,
            &self.walls,
            &self.params.locomotion,
        );

        // 4. Social forces, then walls.
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

        // 5. Integrate, clamp to walls, then project bodies apart and clamp
        //    again. Enforcing walls on both sides of the agent solve means a
        //    body never starts an iteration inside a wall, and the solve cannot
        //    leave it in one.
        self.scratch.snapshot_positions(&self.world);
        locomotion::integrate(&mut self.world, &self.params.locomotion, &self.scratch, dt);
        locomotion::resolve_wall_contacts(&mut self.world, &self.walls, 1);
        let max_overlap = locomotion::resolve_contacts(
            &mut self.world,
            &self.grid,
            &self.params.locomotion,
            &mut self.scratch,
            dt,
        );

        // 6. Walls are hard constraints, applied after agent contacts so the
        //    crowd cannot push anyone through one. Soft repulsion alone loses
        //    agents through corners under load — see `resolve_wall_contacts`.
        let wall_pen = locomotion::resolve_wall_contacts(&mut self.world, &self.walls, 2);

        // Velocity follows from where bodies ended up, once every constraint has
        // been applied. Folding corrections into velocity as they happen injects
        // energy — see `derive_velocity_from_positions`.
        locomotion::derive_velocity_from_positions(&mut self.world, &self.scratch, dt);

        // 7. Let anyone who crossed a doorway this tick leave.
        //
        //    This must run *before* the escape recovery below. Stepping through
        //    a door means stepping off the mesh, and recovery cannot tell that
        //    from a leak — it would drag the agent back inside, so nobody would
        //    ever get out through the one route that works.
        self.process_exits();

        // 8. Safety net: the navmesh is the authority on where an agent may be.
        //    Wall projection alone cannot recover an agent that already escaped,
        //    because it pushes toward whichever side the agent is on. Anyone off
        //    the mesh is put back and counted — a non-zero count here means the
        //    physics is leaking and should be investigated, not tuned around.
        let escaped = self.recover_escaped();

        locomotion::update_blocked_state(&mut self.world, self.params.blocked_speed);
        self.replan_the_stuck();
        self.reconsider_exits(dt);

        // Derive time from the tick count rather than accumulating. Repeated
        // `time += dt` drifts — `0.05f32` is not exactly 0.05, and a 90-minute
        // run is over 100,000 additions of that error. Multiplying instead
        // keeps every timestamp exact, which matters because egress times are
        // read off these values.
        self.world.tick += 1;
        self.world.time = self.world.tick as f64 * self.params.dt;

        if self.world.tick % self.density_interval == 0 {
            self.density.accumulate(&self.world);
        }

        self.last_solve = SolveDiagnostics {
            max_overlap: max_overlap.max(wall_pen),
            escaped,
        };
        self.stats()
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

    /// Give up on a route that is not working and plan a fresh one.
    ///
    /// Waypoint advance is monotonic, which is fine until a crowd shoves an
    /// agent *backwards* past a corner it had already turned. It is then aiming
    /// at a waypoint it can no longer reach in a straight line, walks into the
    /// wall between them, and stays there: the route says go north, the wall
    /// says no, and nothing in the loop ever revisits the decision. RiMEA TC3
    /// left one agent wedged in exactly this way, in the same spot every run.
    ///
    /// Two seconds of wanting to move and not moving is the trigger. That is
    /// long enough not to fire on ordinary queueing — where agents are blocked
    /// constantly and correctly — and short enough that a genuinely stuck agent
    /// recovers rather than standing there for the rest of the run.
    fn replan_the_stuck(&mut self) {
        const GIVE_UP_TICKS: u16 = 40;

        for i in 0..self.world.len() {
            if !self.world.active[i] {
                continue;
            }
            if self.world.state[i] != AgentState::Blocked {
                self.stuck_ticks[i] = 0;
                continue;
            }
            self.stuck_ticks[i] += 1;
            if self.stuck_ticks[i] < GIVE_UP_TICKS {
                continue;
            }
            self.stuck_ticks[i] = 0;

            let from = Vec2::new(self.world.pos_x[i] as f64, self.world.pos_y[i] as f64);
            let goal = self.routes[i].goal;
            if goal == Vec2::ZERO && self.routes[i].points.is_empty() {
                continue;
            }
            let to_exit = self.routes[i].to_exit;
            self.routes[i] = self.plan(from, goal, self.world.radius[i] as f64, to_exit);
        }
    }

    /// Let agents change their minds about which exit to use.
    ///
    /// Routing everyone to their nearest door is right until that door
    /// saturates. A real crowd redistributes: people can see a queue, and some
    /// of them walk further to avoid it. Without this a hall with one popular
    /// and one ignored exit reports the egress time of the popular one alone,
    /// which is optimistic — the direction that matters.
    ///
    /// The cost of an exit is the time to walk to it plus the time to get
    /// through the queue already there, `queue / (width × specific flow)`, using
    /// the Green Guide's 82 persons/m/min. That is the hydraulic model the
    /// dossier already quotes, so the agent's decision and the compliance
    /// arithmetic rest on the same figure rather than on two different ones.
    ///
    /// Agents reconsider on a stagger drawn from `Stream::RerouteChoice`, not
    /// all on the same tick: a synchronised crowd oscillates between two doors,
    /// which looks dramatic and is wrong.
    fn reconsider_exits(&mut self, dt: f32) {
        if self.params.reroute_interval_s <= 0.0 || self.exits.len() < 2 {
            return;
        }
        let Some(mesh) = &self.mesh else {
            return;
        };

        // How many are already waiting at each door. One pass, shared by every
        // agent that reconsiders this tick.
        const QUEUE_RADIUS: f64 = 4.0;
        let mut queue = vec![0u32; self.exits.len()];
        for i in 0..self.world.len() {
            if !self.world.active[i] {
                continue;
            }
            let p = Vec2::new(self.world.pos_x[i] as f64, self.world.pos_y[i] as f64);
            for (e, span) in self.exits.iter().enumerate() {
                if span.segment().distance_to_point(p) <= QUEUE_RADIUS {
                    queue[e] += 1;
                    break;
                }
            }
        }

        // Persons per second each door can pass, from its clear width.
        let capacity: Vec<f64> = self
            .exits
            .iter()
            .map(|e| (e.segment().length() * 82.0 / 60.0).max(1e-3))
            .collect();

        let mut changes: Vec<(usize, Route)> = Vec::new();
        for i in 0..self.world.len() {
            if !self.world.active[i] || !self.world.state[i].is_mobile() {
                continue;
            }
            // Only agents that are trying to leave. See `Route::to_exit`.
            if !self.routes[i].to_exit {
                continue;
            }
            self.world.patience_left[i] -= dt;
            if self.world.patience_left[i] > 0.0 {
                continue;
            }
            // Stagger the next one so the crowd never decides in lockstep.
            let u = self
                .rng
                .uniform01(Stream::RerouteChoice, i as u64, self.world.tick)
                as f32;
            self.world.patience_left[i] = self.params.reroute_interval_s * (0.5 + u);

            let p = Vec2::new(self.world.pos_x[i] as f64, self.world.pos_y[i] as f64);
            let speed = (self.world.desired_speed[i] as f64).max(0.1);

            let mut best: Option<(f64, Vec2)> = None;
            for (e, span) in self.exits.iter().enumerate() {
                let target = self.approach_point(*span);
                let Some(path) = mesh.find_path(p, target) else {
                    continue;
                };
                let walk = cf_navmesh::path_length(&path) / speed;
                let wait = queue[e] as f64 / capacity[e];
                let cost = walk + wait;
                if best.is_none_or(|(b, _)| cost < b) {
                    best = Some((cost, span.midpoint()));
                }
            }

            if let Some((_, goal)) = best {
                if goal != self.routes[i].goal {
                    changes.push((i, self.plan(p, goal, self.world.radius[i] as f64, true)));
                }
            }
        }

        for (i, r) in changes {
            self.routes[i] = r;
        }
    }

    /// Scale desired speed by the terrain an agent is standing on.
    ///
    /// This is what makes a stair a stair. `cf_schema` has carried
    /// `Zone::speed_multiplier` and `VerticalLink::speed_multiplier_up/_down`
    /// since the data model was written, and until now nothing read them:
    /// `desired_speed` was a constant for an agent's whole run, so a flight of
    /// stairs was walked at the same pace as a foyer and RiMEA TC2 could not be
    /// written at all.
    ///
    /// Costs nothing on a venue without stairs. `uniform_speed` is settled at
    /// build time, and when it holds this returns before touching an agent —
    /// which matters, because the alternative is a point-location query per
    /// agent per tick.
    fn apply_terrain_speed(&mut self) {
        let Some(mesh) = &self.mesh else {
            return;
        };
        if mesh.uniform_speed {
            return;
        }
        for i in 0..self.world.len() {
            if !self.world.active[i] || !self.world.state[i].is_mobile() {
                continue;
            }
            let p = Vec2::new(self.world.pos_x[i] as f64, self.world.pos_y[i] as f64);
            let Some(idx) = mesh.locate_from(p, self.tri_hint[i]) else {
                continue;
            };
            self.tri_hint[i] = idx;
            let m = mesh.speed_at(idx);
            if (m - 1.0).abs() < 1e-6 {
                continue;
            }
            self.world.des_x[i] *= m;
            self.world.des_y[i] *= m;
        }
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

    /// Retire anyone whose movement this tick crossed a doorway.
    ///
    /// # Why crossing, and not proximity
    ///
    /// This used to despawn any agent within `exit_radius` (0.6 m) of the door
    /// segment. That is a capsule, not a doorway: it reached 0.6 m back into the
    /// room and 0.6 m past each post, so a 1.0 m door absorbed people over a
    /// 2.2 m front and did it *before* they reached the opening. No queue ever
    /// formed, and throughput was set by how fast a crowd could walk up to the
    /// capture region rather than by the width of the gap. Measured flow through
    /// a 1 m door was 281 persons/m/min against a Green Guide figure of 82 — and
    /// because every reported evacuation time inherited it, the error ran in the
    /// dangerous direction: too fast, on a number a venue gets approved on.
    ///
    /// A door is a line you pass through. So the test is whether the capsule
    /// swept by the agent's *body* this tick reaches the opening, which makes
    /// flow scale with clear width the way measurement says it should.
    ///
    /// The capture distance is the agent's own radius — anatomy, not a tuning
    /// knob, and a quarter of the 0.6 m it replaced. Testing the centre point
    /// alone is not enough: an agent whose goal *is* the doorway decelerates as
    /// it arrives and can stop a few centimetres short forever. A dense crowd
    /// shoves it through, so the doorway harness never saw this, but RiMEA TC3,
    /// TC6, TC8 and TC12 all deadlocked with nobody leaving at all.
    fn process_exits(&mut self) {
        if self.exits.is_empty() {
            return;
        }
        let mut leaving: Vec<AgentId> = Vec::new();

        for i in 0..self.world.len() {
            if !self.world.active[i] {
                continue;
            }
            let to = Vec2::new(self.world.pos_x[i] as f64, self.world.pos_y[i] as f64);
            let from = self.scratch.previous_position(i).unwrap_or(to);

            // The body sweeps a capsule from `from` to `to`. It has passed the
            // door once that capsule touches the opening — which is what a
            // person passing through a door physically does.
            let swept = Segment::new(from, to);
            let r = self.world.radius[i] as f64;
            if self
                .exits
                .iter()
                .any(|e| cf_geom::segment_distance(&swept, &e.segment()) <= r)
            {
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
        s.spawn_toward(agent(0.0, 0.0), Vec2::new(5.0, 0.0), false);
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
        s.spawn_toward(agent(0.0, 0.0), Vec2::new(10.0, 0.0), false);
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
        s.spawn_toward(agent(0.0, 0.0), Vec2::new(10.0, 0.0), false);

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
            s.spawn_toward(agent(x, y), Vec2::new(12.0, 0.0), false);
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
                s.spawn_toward(agent(x, y), Vec2::new(20.0, 3.0), false);
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
        s.spawn_toward(agent(0.0, 0.0), Vec2::new(5.0, 0.0), false);

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
            s.spawn_toward(
                agent((i % 6) as f64 * 0.4, (i / 6) as f64 * 0.4),
                Vec2::new(6.0, 0.6),
                false,
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
