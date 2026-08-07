//! A multi-floor venue: one simulation per floor, joined by stairs.
//!
//! # Why one `Sim` per floor rather than one `Sim` that knows about floors
//!
//! Agents on different floors must not see each other — not in the contact
//! solve, not in the density sensor, not in the neighbour queries that dominate
//! the step. Threading a floor index through every one of those is a branch in
//! the hottest loop in the engine for a comparison that is constant across a
//! whole floor, and it would put a correctness burden on code that currently
//! has none: a single forgotten check and people on the second floor start
//! shoving people in the lobby.
//!
//! Giving each floor its own `Sim` makes that impossible by construction. The
//! spatial grid, the walls, the mesh and the exits are all already per-floor
//! quantities; this simply stops pretending there is only ever one of them.
//! It also means the single-floor path — which is every venue in the fixture
//! set and most venues anywhere — is completely unchanged.
//!
//! The cost is that an agent crossing a floor is *destroyed and recreated*
//! rather than moved. Ids are therefore per-floor and are not stable across a
//! transfer, which matters for trajectory export and is recorded in
//! `docs/06-validation.md`.
//!
//! # What a stair is here
//!
//! A [`cf_compile::LinkNode`] resolved to a landing point on each floor. An
//! agent that reaches one landing reappears at the other, having spent the
//! time the link's speed multiplier implies. Walking *on* the stair is not
//! simulated as motion in a third space — the transfer is instantaneous in
//! position and costed in time, which is what a hydraulic model does and what
//! the Green Guide's rate of passage describes.

use crate::sim::{Sim, SimStats};
use crate::world::{AgentState, SpawnParams};
use cf_geom::Vec2;

/// A stair or ramp between two floors, ready to walk.
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub floor_a: usize,
    pub point_a: Vec2,
    pub floor_b: usize,
    pub point_b: Vec2,
    /// Clear width, metres. Caps how many people per second may cross.
    pub clear_width_m: f64,
    /// Seconds to traverse, derived from the link's speed multiplier.
    pub traverse_s: f64,
}

impl Link {
    /// The far end, given the floor an agent is leaving.
    fn other(&self, from: usize) -> Option<(usize, Vec2)> {
        if from == self.floor_a {
            Some((self.floor_b, self.point_b))
        } else if from == self.floor_b {
            Some((self.floor_a, self.point_a))
        } else {
            None
        }
    }
}

/// Someone on the stairs: off one floor, not yet on the next.
#[derive(Clone, Copy, Debug)]
struct InTransit {
    arrives_at: f64,
    to_floor: usize,
    to_point: Vec2,
    radius_m: f32,
    desired_speed: f32,
    population: u16,
    entry: u16,
}

/// A whole venue.
pub struct Building {
    floors: Vec<Sim>,
    links: Vec<Link>,
    transit: Vec<InTransit>,
    /// Seconds elapsed. Held here rather than read from a floor because floors
    /// step in lockstep and one of them being authoritative is an invitation
    /// for them to disagree.
    time: f64,
    dt: f64,
    /// How many have crossed each link, so a report can say which stair carried
    /// the building.
    crossings: Vec<u32>,
}

/// How close to a landing an agent has to get to be on the stairs.
///
/// A body radius, matching the doorway rule: you are on the stair when your
/// body reaches it, not when your centre point does.
const LANDING_REACH: f64 = 0.6;

impl Building {
    /// Assemble from per-floor simulations and the links between them.
    ///
    /// Links naming a floor that does not exist are dropped — the compiler has
    /// already reported those, and carrying an unusable one here would only
    /// give the transfer loop something to trip over.
    pub fn new(floors: Vec<Sim>, links: Vec<Link>) -> Self {
        let dt = floors.first().map(|f| f.params.dt).unwrap_or(0.05);
        let n = floors.len();
        let links: Vec<Link> = links
            .into_iter()
            .filter(|l| l.floor_a < n && l.floor_b < n && l.floor_a != l.floor_b)
            .collect();
        let crossings = vec![0; links.len()];
        Self {
            floors,
            links,
            transit: Vec::new(),
            time: 0.0,
            dt,
            crossings,
        }
    }

    pub fn floor(&self, i: usize) -> Option<&Sim> {
        self.floors.get(i)
    }

    pub fn floor_mut(&mut self, i: usize) -> Option<&mut Sim> {
        self.floors.get_mut(i)
    }

    pub fn floor_count(&self) -> usize {
        self.floors.len()
    }

    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// How many agents have crossed each link, in `links()` order.
    pub fn crossings(&self) -> &[u32] {
        &self.crossings
    }

    /// Agents currently on a stair: off one floor and not yet on the next.
    ///
    /// A run is over when every floor is empty **and** this is zero. Without
    /// it a building looks evacuated while people are still on the stairs.
    pub fn in_transit(&self) -> usize {
        self.transit.len()
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    /// Send everyone on `floor` toward the nearest link landing on it.
    ///
    /// For an upper floor with no doors of its own, this is what "evacuate"
    /// means: reach the stairs. Returns how many were routed.
    pub fn route_to_stairs(&mut self, floor: usize) -> u32 {
        let landings: Vec<Vec2> = self
            .links
            .iter()
            .filter_map(|l| {
                if l.floor_a == floor {
                    Some(l.point_a)
                } else if l.floor_b == floor {
                    Some(l.point_b)
                } else {
                    None
                }
            })
            .collect();
        if landings.is_empty() {
            return 0;
        }

        let Some(sim) = self.floors.get_mut(floor) else {
            return 0;
        };
        let mut routed = 0;
        for i in 0..sim.world.len() {
            if !sim.world.active[i] {
                continue;
            }
            let p = Vec2::new(sim.world.pos_x[i] as f64, sim.world.pos_y[i] as f64);
            let Some(&best) = landings
                .iter()
                .min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)))
            else {
                continue;
            };
            // Not an exit: congestion-aware rerouting must not override a goal
            // that is a staircase with whichever door happens to be quiet.
            sim.retarget(i, best, false);
            routed += 1;
        }
        routed
    }

    /// Advance every floor one tick, then move anyone who reached a stair.
    pub fn step(&mut self) {
        for sim in &mut self.floors {
            sim.step();
        }
        self.time += self.dt;
        self.collect_onto_stairs();
        self.deliver_from_stairs();
    }

    /// Take agents standing on a landing off their floor and onto the stairs.
    fn collect_onto_stairs(&mut self) {
        for (li, link) in self.links.iter().enumerate() {
            for from in [link.floor_a, link.floor_b] {
                let Some((to_floor, to_point)) = link.other(from) else {
                    continue;
                };
                let here = if from == link.floor_a {
                    link.point_a
                } else {
                    link.point_b
                };

                let Some(sim) = self.floors.get_mut(from) else {
                    continue;
                };

                // Ascending id order, so the transfer log is reproducible.
                let mut taken = Vec::new();
                for i in 0..sim.world.len() {
                    if !sim.world.active[i] || !sim.world.state[i].is_mobile() {
                        continue;
                    }
                    let p = Vec2::new(sim.world.pos_x[i] as f64, sim.world.pos_y[i] as f64);
                    if p.distance(here) > LANDING_REACH {
                        continue;
                    }
                    // You take the stairs because you are trying to, not
                    // because you walked over them.
                    //
                    // Without this an agent delivered onto the ground floor
                    // arrives standing *on* the landing, is collected again the
                    // same tick, and rides back up — 120 people made 5,868
                    // crossings and the building never emptied. Someone
                    // crossing the lobby past a staircase they are not using
                    // must be left alone.
                    if sim
                        .goal_of(i)
                        .is_none_or(|g| g.distance(here) > LANDING_REACH)
                    {
                        continue;
                    }
                    taken.push(InTransit {
                        arrives_at: self.time + link.traverse_s,
                        to_floor,
                        to_point,
                        radius_m: sim.world.radius[i],
                        desired_speed: sim.world.desired_speed[i],
                        population: sim.world.population[i],
                        entry: sim.world.cold[i].entry,
                    });
                    sim.world.despawn(i as u32);
                }

                self.crossings[li] += taken.len() as u32;
                self.transit.extend(taken);
            }
        }
    }

    /// Put anyone whose traverse time has elapsed onto their destination floor.
    fn deliver_from_stairs(&mut self) {
        let now = self.time;
        let mut still = Vec::with_capacity(self.transit.len());

        for t in std::mem::take(&mut self.transit) {
            if t.arrives_at > now {
                still.push(t);
                continue;
            }
            let Some(sim) = self.floors.get_mut(t.to_floor) else {
                continue;
            };
            sim.spawn_to_nearest_exit(SpawnParams {
                position: t.to_point,
                radius_m: t.radius_m,
                desired_speed: t.desired_speed,
                goal: 0,
                population: t.population,
                entry: t.entry,
                state: AgentState::Walking,
            });
        }
        self.transit = still;
    }

    /// Counters across the whole building.
    ///
    /// `active` includes people on the stairs, who are in the building and are
    /// not out of it — reporting them as neither would make a run look finished
    /// while a staircase was still full.
    pub fn stats(&self) -> SimStats {
        let mut out = SimStats {
            time: self.time,
            ..SimStats::default()
        };
        for sim in &self.floors {
            let s = sim.stats();
            out.tick = out.tick.max(s.tick);
            out.active += s.active;
            out.exited += s.exited;
            out.spawned += s.spawned;
            out.blocked += s.blocked;
            out.escaped += s.escaped;
            out.max_overlap = out.max_overlap.max(s.max_overlap);
        }
        out.active += self.transit.len() as u32;
        // Someone who crossed a stair was spawned twice and has left once too
        // often for the total to mean anything without this.
        let crossed: u32 = self.crossings.iter().sum();
        out.spawned = out.spawned.saturating_sub(crossed);
        out.exited = out.exited.saturating_sub(crossed);
        out
    }
}
