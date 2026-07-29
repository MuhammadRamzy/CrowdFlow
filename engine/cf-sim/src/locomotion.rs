//! Agent locomotion: Social Force Model plus position-based contact resolution.
//!
//! # Why a hybrid rather than pure SFM
//!
//! The Social Force Model (Helbing & Molnár 1995) is well validated at low to
//! moderate density and produces the emergent behaviours crowd science cares
//! about — lane formation, arching at exits, faster-is-slower. But it enforces
//! non-overlap only through repulsion, and at high density that fails in a way
//! that matters:
//!
//! - Keeping bodies apart needs large repulsion constants.
//! - Large constants make the system stiff, so it needs a tiny timestep.
//! - A tiny timestep is unaffordable at 25,000 agents.
//! - With an affordable timestep, agents visibly interpenetrate and the
//!   integrator can blow up at choke points.
//!
//! Choke points are exactly the regime this product exists to analyse, so
//! "unstable when crowded" is not an acceptable limitation.
//!
//! # The split
//!
//! **SFM decides where an agent wants to go.** Driving force toward the goal,
//! anisotropic repulsion from neighbours (weighted by whether they are in
//! front), repulsion from walls. This is the part validated against measured
//! pedestrian data.
//!
//! **Position-based dynamics enforces that bodies do not overlap.** After
//! integrating, a few Jacobi iterations project overlapping pairs apart along
//! their centre line, and velocity is corrected from the resulting position
//! change. Jacobi rather than Gauss-Seidel because the latter's result depends
//! on the order pairs are visited, which would break reproducibility.
//!
//! PBD is unconditionally stable: a projection cannot add energy, so there is
//! no stiffness and no timestep constraint. The result is stable at 20 Hz,
//! guarantees zero overlap, and leaves SFM's emergent behaviours intact.
//!
//! # Parameters
//!
//! Defaults are the commonly cited Helbing values. They are not final: they get
//! calibrated against the fundamental diagram and the RiMEA suite in phase B5
//! (`docs/06-validation.md` §8), and that calibration is what makes the model
//! defensible rather than merely plausible.

use crate::fmath;
use crate::spatial::SpatialGrid;
use crate::world::{AgentState, World};
use cf_geom::Vec2;

/// Tunable constants for the locomotion model.
#[derive(Clone, Copy, Debug)]
pub struct LocomotionParams {
    /// Relaxation time for the driving force, seconds. How quickly an agent
    /// accelerates toward its desired velocity.
    pub tau: f32,
    /// Repulsion magnitude between agents, newtons.
    pub a_agent: f32,
    /// Repulsion falloff distance between agents, metres.
    pub b_agent: f32,
    /// Repulsion magnitude from walls, newtons.
    pub a_wall: f32,
    /// Repulsion falloff distance from walls, metres.
    pub b_wall: f32,
    /// Anisotropy: how much repulsion is discounted for neighbours behind.
    /// 0 means isotropic; 1 means fully ignore what is behind.
    pub lambda: f32,
    /// Beyond this range neighbours are ignored entirely, metres.
    pub interaction_range: f32,
    /// Iterations of the contact solve. Jacobi-style, so more iterations
    /// converge rather than reorder — see `resolve_contacts`.
    pub contact_iterations: u32,
    /// Fraction of overlap removed per iteration. Below 1 the solve is softer
    /// and more stable when many contacts interact.
    pub contact_stiffness: f32,
    /// Speed ceiling as a multiple of desired speed. Bounds the worst case if a
    /// force ever misbehaves.
    pub max_speed_factor: f32,
}

impl Default for LocomotionParams {
    fn default() -> Self {
        Self {
            tau: 0.5,
            a_agent: 2000.0,
            b_agent: 0.08,
            a_wall: 2000.0,
            b_wall: 0.08,
            lambda: 0.75,
            interaction_range: 2.0,
            contact_iterations: 3,
            contact_stiffness: 0.9,
            max_speed_factor: 1.6,
        }
    }
}

/// Scratch buffers reused across ticks so a step allocates nothing.
#[derive(Clone, Debug, Default)]
pub struct LocomotionScratch {
    force_x: Vec<f32>,
    force_y: Vec<f32>,
    corr_x: Vec<f32>,
    corr_y: Vec<f32>,
    corr_n: Vec<u32>,
}

impl LocomotionScratch {
    fn resize(&mut self, n: usize) {
        self.force_x.clear();
        self.force_y.clear();
        self.force_x.resize(n, 0.0);
        self.force_y.resize(n, 0.0);
        self.corr_x.clear();
        self.corr_y.clear();
        self.corr_n.clear();
        self.corr_x.resize(n, 0.0);
        self.corr_y.resize(n, 0.0);
        self.corr_n.resize(n, 0);
    }
}

/// Accumulate social forces into the scratch buffers.
///
/// Iterates agents in ascending id and neighbours in the grid's stable order,
/// so the sum is identical on every run and on every target.
pub fn compute_forces(
    w: &World,
    grid: &SpatialGrid,
    params: &LocomotionParams,
    scratch: &mut LocomotionScratch,
) {
    let n = w.len();
    scratch.resize(n);
    let range_sq = params.interaction_range * params.interaction_range;

    for i in 0..n {
        if !w.active[i] || !w.state[i].is_mobile() {
            continue;
        }

        // Driving force: relax current velocity toward the desired velocity.
        let drive_x = (w.des_x[i] - w.vel_x[i]) / params.tau;
        let drive_y = (w.des_y[i] - w.vel_y[i]) / params.tau;

        let px = w.pos_x[i];
        let py = w.pos_y[i];
        let ri = w.radius[i];

        // Heading, used for the anisotropy weight. An agent reacts far more to
        // what is in front of it than behind — without this, crowds behave as
        // if everyone had eyes in the back of their head, and lane formation
        // does not emerge properly.
        let speed = fmath::hypot2(w.vel_x[i], w.vel_y[i]);
        let (hx, hy) = if speed > 1e-4 {
            (w.vel_x[i] / speed, w.vel_y[i] / speed)
        } else {
            (0.0, 0.0)
        };

        grid.for_each_near(Vec2::new(px as f64, py as f64), |j| {
            let j = j as usize;
            if j == i || !w.active[j] {
                return;
            }
            let dx = px - w.pos_x[j];
            let dy = py - w.pos_y[j];
            let dist_sq = dx * dx + dy * dy;
            if dist_sq > range_sq || dist_sq <= 1e-12 {
                return;
            }
            let dist = fmath::sqrt(dist_sq);
            let nx = dx / dist;
            let ny = dy / dist;

            // Helbing's exponential repulsion on surface separation.
            let overlap = (ri + w.radius[j]) - dist;
            let mag = params.a_agent * fmath::exp(overlap / params.b_agent);

            // Anisotropy: full weight ahead, discounted behind.
            let cos_phi = -(nx * hx + ny * hy);
            let weight = if speed > 1e-4 {
                params.lambda + (1.0 - params.lambda) * (1.0 + cos_phi) * 0.5
            } else {
                1.0
            };

            scratch.force_x[i] += mag * weight * nx;
            scratch.force_y[i] += mag * weight * ny;
        });

        scratch.force_x[i] += drive_x;
        scratch.force_y[i] += drive_y;
    }
}

/// Add repulsion from a wall segment to every nearby agent.
///
/// Called per wall by the step loop. Kept separate from [`compute_forces`] so
/// wall geometry does not have to be threaded through the neighbour loop.
pub fn add_wall_force(
    w: &World,
    a: Vec2,
    b: Vec2,
    params: &LocomotionParams,
    scratch: &mut LocomotionScratch,
) {
    let seg = cf_geom::Segment::new(a, b);
    for i in 0..w.len() {
        if !w.active[i] || !w.state[i].is_mobile() {
            continue;
        }
        let p = Vec2::new(w.pos_x[i] as f64, w.pos_y[i] as f64);
        let closest = seg.closest_point(p);
        let d = p - closest;
        let dist = d.length() as f32;
        if dist > params.interaction_range || dist <= 1e-9 {
            continue;
        }
        let overlap = w.radius[i] - dist;
        let mag = params.a_wall * fmath::exp(overlap / params.b_wall);
        scratch.force_x[i] += mag * (d.x as f32 / dist);
        scratch.force_y[i] += mag * (d.y as f32 / dist);
    }
}

/// Integrate velocity and position, then clamp speed.
///
/// Semi-implicit Euler: velocity is updated first and the new velocity moves
/// the position. It is one line different from explicit Euler and markedly more
/// stable for this kind of system.
pub fn integrate(w: &mut World, params: &LocomotionParams, scratch: &LocomotionScratch, dt: f32) {
    for i in 0..w.len() {
        if !w.active[i] || !w.state[i].is_mobile() {
            continue;
        }
        w.vel_x[i] += scratch.force_x[i] * dt;
        w.vel_y[i] += scratch.force_y[i] * dt;

        let speed = fmath::hypot2(w.vel_x[i], w.vel_y[i]);
        let cap = w.desired_speed[i] * params.max_speed_factor;
        if speed > cap && speed > 1e-6 {
            let s = cap / speed;
            w.vel_x[i] *= s;
            w.vel_y[i] *= s;
        }

        w.pos_x[i] += w.vel_x[i] * dt;
        w.pos_y[i] += w.vel_y[i] * dt;
    }
}

/// Project overlapping agents apart.
///
/// Jacobi-style: every pair's correction is accumulated against the *same*
/// starting positions, then applied at the end of each iteration. Gauss-Seidel
/// (applying each correction immediately) converges faster but makes the result
/// depend on pair visitation order — unacceptable when the same run has to
/// reproduce on another machine.
///
/// Returns the largest overlap remaining, in metres, which the step loop can
/// use as a convergence signal.
pub fn resolve_contacts(
    w: &mut World,
    grid: &SpatialGrid,
    params: &LocomotionParams,
    scratch: &mut LocomotionScratch,
    dt: f32,
) -> f32 {
    let n = w.len();
    let mut worst = 0.0f32;

    for _ in 0..params.contact_iterations {
        for v in scratch.corr_x.iter_mut() {
            *v = 0.0;
        }
        for v in scratch.corr_y.iter_mut() {
            *v = 0.0;
        }
        for v in scratch.corr_n.iter_mut() {
            *v = 0;
        }

        worst = 0.0;

        for i in 0..n {
            if !w.active[i] || !w.state[i].is_mobile() {
                continue;
            }
            let px = w.pos_x[i];
            let py = w.pos_y[i];
            let ri = w.radius[i];

            grid.for_each_near(Vec2::new(px as f64, py as f64), |j| {
                let j = j as usize;
                // Handle each pair once, from the lower index.
                if j <= i || !w.active[j] {
                    return;
                }
                let dx = px - w.pos_x[j];
                let dy = py - w.pos_y[j];
                let dist_sq = dx * dx + dy * dy;
                let min_dist = ri + w.radius[j];
                if dist_sq >= min_dist * min_dist {
                    return;
                }

                let dist = fmath::sqrt(dist_sq);
                // Exactly coincident agents have no separation direction.
                // Nudge along +x so the next iteration has something to work
                // with; without this they stay welded together forever.
                let (nx, ny) = if dist > 1e-6 {
                    (dx / dist, dy / dist)
                } else {
                    (1.0, 0.0)
                };

                let overlap = min_dist - dist;
                if overlap > worst {
                    worst = overlap;
                }

                // Split the correction evenly. Mass weighting goes here once
                // agents carry mass.
                let push = overlap * 0.5 * params.contact_stiffness;
                scratch.corr_x[i] += nx * push;
                scratch.corr_y[i] += ny * push;
                scratch.corr_n[i] += 1;
                scratch.corr_x[j] -= nx * push;
                scratch.corr_y[j] -= ny * push;
                scratch.corr_n[j] += 1;
            });
        }

        if worst <= 0.0 {
            break;
        }

        // Apply averaged corrections, and fold the position change back into
        // velocity so momentum stays consistent with where bodies ended up.
        let inv_dt = if dt > 1e-9 { 1.0 / dt } else { 0.0 };
        for i in 0..n {
            if scratch.corr_n[i] == 0 {
                continue;
            }
            let k = 1.0 / scratch.corr_n[i] as f32;
            let cx = scratch.corr_x[i] * k;
            let cy = scratch.corr_y[i] * k;
            w.pos_x[i] += cx;
            w.pos_y[i] += cy;
            w.vel_x[i] += cx * inv_dt;
            w.vel_y[i] += cy * inv_dt;
        }
    }

    worst
}

/// Set each agent's desired velocity to point at a fixed target.
///
/// A placeholder for the flow fields of phase B4 — enough to get agents walking
/// to an exit, and enough to test the locomotion model against.
pub fn steer_towards(w: &mut World, target: Vec2, arrive_radius: f32) {
    for i in 0..w.len() {
        if !w.active[i] || !w.state[i].is_mobile() {
            continue;
        }
        let dx = target.x as f32 - w.pos_x[i];
        let dy = target.y as f32 - w.pos_y[i];
        let dist = fmath::hypot2(dx, dy);
        if dist <= arrive_radius || dist < 1e-6 {
            w.des_x[i] = 0.0;
            w.des_y[i] = 0.0;
            continue;
        }
        let s = w.desired_speed[i] / dist;
        w.des_x[i] = dx * s;
        w.des_y[i] = dy * s;
    }
}

/// Update `AgentState` from motion: an agent that wants to move but cannot is
/// `Blocked`, which is what the congestion analytics count.
pub fn update_blocked_state(w: &mut World, threshold: f32) {
    for i in 0..w.len() {
        if !w.active[i] {
            continue;
        }
        if !matches!(w.state[i], AgentState::Walking | AgentState::Blocked) {
            continue;
        }
        let wants = fmath::hypot2(w.des_x[i], w.des_y[i]) > 1e-3;
        let moving = fmath::hypot2(w.vel_x[i], w.vel_y[i]) > threshold;
        w.state[i] = if wants && !moving {
            AgentState::Blocked
        } else {
            AgentState::Walking
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::SpawnParams;
    use cf_geom::Aabb;

    fn bounds() -> Aabb {
        Aabb {
            min: Vec2::new(-50.0, -50.0),
            max: Vec2::new(50.0, 50.0),
        }
    }

    fn spawn_at(w: &mut World, x: f64, y: f64) -> u32 {
        w.spawn(SpawnParams {
            position: Vec2::new(x, y),
            radius_m: 0.23,
            desired_speed: 1.34,
            goal: 0,
            population: 0,
            entry: 0,
            state: AgentState::Walking,
        })
    }

    /// Drive the loop for `steps` ticks. Mirrors what the step loop will do.
    fn run(w: &mut World, params: &LocomotionParams, target: Vec2, steps: usize, dt: f32) {
        let mut grid = SpatialGrid::new(bounds(), params.interaction_range as f64);
        let mut scratch = LocomotionScratch::default();
        for _ in 0..steps {
            grid.rebuild(&w.pos_x, &w.pos_y, &w.active);
            steer_towards(w, target, 0.1);
            compute_forces(w, &grid, params, &mut scratch);
            integrate(w, params, &scratch, dt);
            resolve_contacts(w, &grid, params, &mut scratch, dt);
            w.time += dt as f64;
            w.tick += 1;
        }
    }

    #[test]
    fn a_lone_agent_walks_to_its_target_at_its_desired_speed() {
        let mut w = World::new();
        spawn_at(&mut w, 0.0, 0.0);
        let p = LocomotionParams::default();
        let target = Vec2::new(10.0, 0.0);

        // 10 m at 1.34 m/s is ~7.5 s, plus a little to accelerate.
        run(&mut w, &p, target, 180, 0.05);

        let pos = w.position(0);
        assert!(
            pos.distance(target) < 0.5,
            "agent ended at {pos:?}, target {target:?}"
        );
        assert!(w.check_invariants().is_empty());
    }

    #[test]
    fn an_agent_does_not_exceed_its_speed_cap() {
        let mut w = World::new();
        spawn_at(&mut w, 0.0, 0.0);
        let p = LocomotionParams::default();
        let mut grid = SpatialGrid::new(bounds(), p.interaction_range as f64);
        let mut scratch = LocomotionScratch::default();

        for _ in 0..200 {
            grid.rebuild(&w.pos_x, &w.pos_y, &w.active);
            steer_towards(&mut w, Vec2::new(1000.0, 0.0), 0.1);
            compute_forces(&w, &grid, &p, &mut scratch);
            integrate(&mut w, &p, &scratch, 0.05);
            let cap = (w.desired_speed[0] * p.max_speed_factor) as f64;
            assert!(
                w.speed(0) <= cap + 1e-4,
                "speed {} exceeded cap {cap}",
                w.speed(0)
            );
        }
    }

    /// The property PBD exists to guarantee. Pure SFM cannot promise this at an
    /// affordable timestep.
    #[test]
    fn agents_never_end_a_tick_overlapping() {
        let mut w = World::new();
        // A dense clump, all converging on one point.
        for i in 0..60 {
            let a = i as f64 * 0.7;
            let r = 0.4 + (i % 5) as f64 * 0.15;
            spawn_at(&mut w, 6.0 + r * a.cos(), r * a.sin());
        }
        let p = LocomotionParams::default();
        run(&mut w, &p, Vec2::new(0.0, 0.0), 120, 0.05);

        for i in 0..w.len() {
            for j in (i + 1)..w.len() {
                let d = w.position(i as u32).distance(w.position(j as u32));
                let min = (w.radius[i] + w.radius[j]) as f64;
                assert!(
                    d >= min - 0.02,
                    "agents {i} and {j} overlap: {d:.4} m apart, need {min:.4}"
                );
            }
        }
        assert!(w.check_invariants().is_empty());
    }

    #[test]
    fn a_dense_crowd_stays_finite() {
        let mut w = World::new();
        // 400 agents in a 6x6 m box: ~11 per m², far past any real crowd.
        for i in 0..400 {
            let x = (i % 20) as f64 * 0.3;
            let y = (i / 20) as f64 * 0.3;
            spawn_at(&mut w, x, y);
        }
        let p = LocomotionParams::default();
        run(&mut w, &p, Vec2::new(30.0, 0.0), 200, 0.05);

        let errs = w.check_invariants();
        assert!(errs.is_empty(), "{errs:#?}");
        for i in 0..w.len() {
            let s = w.speed(i as u32);
            assert!(
                s.is_finite() && s < 10.0,
                "agent {i} reached speed {s} — the integrator blew up"
            );
        }
    }

    #[test]
    fn coincident_agents_separate_rather_than_welding() {
        let mut w = World::new();
        // Exactly the same position: no separation direction exists.
        spawn_at(&mut w, 3.0, 3.0);
        spawn_at(&mut w, 3.0, 3.0);
        let p = LocomotionParams::default();
        run(&mut w, &p, Vec2::new(3.0, 3.0), 40, 0.05);

        let d = w.position(0).distance(w.position(1));
        assert!(
            d > 0.1,
            "coincident agents stayed welded together ({d:.4} m)"
        );
        assert!(w.check_invariants().is_empty());
    }

    #[test]
    fn a_wall_stops_an_agent_walking_through_it() {
        let mut w = World::new();
        spawn_at(&mut w, 0.0, 0.0);
        let p = LocomotionParams::default();
        let wall_a = Vec2::new(5.0, -5.0);
        let wall_b = Vec2::new(5.0, 5.0);

        let mut grid = SpatialGrid::new(bounds(), p.interaction_range as f64);
        let mut scratch = LocomotionScratch::default();
        for _ in 0..300 {
            grid.rebuild(&w.pos_x, &w.pos_y, &w.active);
            steer_towards(&mut w, Vec2::new(10.0, 0.0), 0.1);
            compute_forces(&w, &grid, &p, &mut scratch);
            add_wall_force(&w, wall_a, wall_b, &p, &mut scratch);
            integrate(&mut w, &p, &scratch, 0.05);
        }

        let x = w.position(0).x;
        assert!(x < 5.0, "agent passed through the wall: x = {x}");
        assert!(x > 4.0, "agent stopped implausibly far short: x = {x}");
    }

    /// Repulsion must be stronger from in front than from behind, or lane
    /// formation does not emerge and crowds behave unnaturally.
    #[test]
    fn repulsion_is_anisotropic() {
        let p = LocomotionParams::default();
        let mut grid = SpatialGrid::new(bounds(), p.interaction_range as f64);
        let mut scratch = LocomotionScratch::default();

        // Subject moving +x, with a neighbour ahead.
        let mut ahead = World::new();
        spawn_at(&mut ahead, 0.0, 0.0);
        spawn_at(&mut ahead, 0.6, 0.0);
        ahead.vel_x[0] = 1.3;
        grid.rebuild(&ahead.pos_x, &ahead.pos_y, &ahead.active);
        compute_forces(&ahead, &grid, &p, &mut scratch);
        let f_ahead = scratch.force_x[0].abs();

        // Same geometry, neighbour behind.
        let mut behind = World::new();
        spawn_at(&mut behind, 0.0, 0.0);
        spawn_at(&mut behind, -0.6, 0.0);
        behind.vel_x[0] = 1.3;
        grid.rebuild(&behind.pos_x, &behind.pos_y, &behind.active);
        compute_forces(&behind, &grid, &p, &mut scratch);
        let f_behind = scratch.force_x[0].abs();

        assert!(
            f_ahead > f_behind,
            "front repulsion {f_ahead} should exceed rear {f_behind}"
        );
    }

    #[test]
    fn departed_agents_neither_move_nor_repel() {
        let mut w = World::new();
        spawn_at(&mut w, 0.0, 0.0);
        spawn_at(&mut w, 0.5, 0.0);
        w.despawn(1);
        let frozen = w.position(1);

        let p = LocomotionParams::default();
        run(&mut w, &p, Vec2::new(20.0, 0.0), 60, 0.05);

        assert_eq!(w.position(1), frozen, "a departed agent moved");
        // Agent 0 walked away unimpeded.
        assert!(w.position(0).x > 3.0, "agent 0 was blocked by a ghost");
    }

    /// Same inputs, same outputs, bit for bit. The whole determinism contract
    /// reduces to this at the locomotion layer.
    #[test]
    fn stepping_is_bit_reproducible() {
        let build = || {
            let mut w = World::new();
            for i in 0..80 {
                spawn_at(&mut w, (i % 10) as f64 * 0.5, (i / 10) as f64 * 0.5);
            }
            w
        };
        let p = LocomotionParams::default();

        let mut a = build();
        let mut b = build();
        run(&mut a, &p, Vec2::new(20.0, 5.0), 100, 0.05);
        run(&mut b, &p, Vec2::new(20.0, 5.0), 100, 0.05);

        for i in 0..a.len() {
            assert_eq!(
                a.pos_x[i].to_bits(),
                b.pos_x[i].to_bits(),
                "agent {i} x diverged"
            );
            assert_eq!(a.pos_y[i].to_bits(), b.pos_y[i].to_bits());
            assert_eq!(a.vel_x[i].to_bits(), b.vel_x[i].to_bits());
            assert_eq!(a.vel_y[i].to_bits(), b.vel_y[i].to_bits());
        }
    }

    #[test]
    fn blocked_state_tracks_whether_an_agent_can_move() {
        let mut w = World::new();
        spawn_at(&mut w, 0.0, 0.0);
        w.des_x[0] = 1.34;
        w.vel_x[0] = 0.0;
        update_blocked_state(&mut w, 0.1);
        assert_eq!(w.state[0], AgentState::Blocked, "wants to move but is not");

        w.vel_x[0] = 1.2;
        update_blocked_state(&mut w, 0.1);
        assert_eq!(w.state[0], AgentState::Walking);

        // An agent with no desired velocity is not blocked, merely arrived.
        w.des_x[0] = 0.0;
        w.vel_x[0] = 0.0;
        update_blocked_state(&mut w, 0.1);
        assert_eq!(w.state[0], AgentState::Walking);
    }
}
