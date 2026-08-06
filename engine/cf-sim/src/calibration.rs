//! Measuring the model against published pedestrian data.
//!
//! A crowd simulator that produces plausible animations is a toy. One that can
//! be shown to reproduce measured human behaviour is an engineering tool, and
//! the difference is this module (`docs/06-validation.md` §4).
//!
//! Two measurements matter most, because everything downstream inherits them:
//!
//! - **Specific flow** — how many people per metre per minute pass a doorway
//!   under saturated demand. This sets evacuation time directly. The UK Green
//!   Guide plans at 82 persons/m/min on the level; measured doorway
//!   experiments cluster around 1.2–1.4 persons/m/s, i.e. 72–84 per minute.
//! - **The fundamental diagram** — walking speed as a function of density.
//!   Weidmann's 1993 relation is the reference curve, and a model outside its
//!   envelope will get congestion wrong even if free-flow speed is right.
//!
//! # Which direction the error matters
//!
//! A model that flows people *faster* than measurement reports shorter
//! evacuation times than reality. That is the dangerous direction: an
//! optimistic figure is one a venue could be approved on and then fail to
//! achieve. So the harness reports signed error, not absolute, and the tests
//! treat "too fast" more harshly than "too slow".

use crate::locomotion::LocomotionParams;
use crate::sim::{ExitSpan, Sim, SimParams};
use crate::world::{AgentState, SpawnParams};
use cf_geom::Vec2;
use cf_navmesh::{classify, edge_key, triangulate_constrained, NavMesh};

/// Green Guide rate of passage on the level, persons per metre per minute.
pub const GREEN_GUIDE_LEVEL_PPMM: f64 = 82.0;

/// Weidmann's free walking speed, m/s.
pub const WEIDMANN_FREE_SPEED: f64 = 1.34;

/// Weidmann (1993) speed–density relation.
///
/// `v(ρ) = v₀ · (1 − exp(−γ · (1/ρ − 1/ρ_max)))` with γ = 1.913 and
/// ρ_max = 5.4 persons/m². The reference curve every pedestrian model is
/// checked against.
pub fn weidmann_speed(density: f64) -> f64 {
    if density <= 0.0 {
        return WEIDMANN_FREE_SPEED;
    }
    if density >= 5.4 {
        return 0.0;
    }
    WEIDMANN_FREE_SPEED * (1.0 - (-1.913 * (1.0 / density - 1.0 / 5.4)).exp())
}

/// The result of a doorway flow measurement.
#[derive(Clone, Copy, Debug)]
pub struct FlowMeasurement {
    /// Doorway clear width, metres.
    pub width_m: f64,
    /// Persons who passed.
    pub count: u32,
    /// Seconds over which they passed.
    pub duration_s: f64,
    /// Persons per metre per minute.
    pub specific_flow: f64,
    /// Worst body interpenetration seen during the discharge, metres.
    ///
    /// A doorway is where the contact solve is under the most pressure. If this
    /// is a large fraction of a body radius the crowd is squeezing through, and
    /// the flow figure above is fiction regardless of how plausible it looks.
    pub max_overlap: f64,
}

impl FlowMeasurement {
    /// Signed error against the Green Guide, as a fraction.
    ///
    /// Positive means the model is faster than the standard — the direction
    /// that produces optimistic evacuation times.
    pub fn error_vs_green_guide(&self) -> f64 {
        self.specific_flow / GREEN_GUIDE_LEVEL_PPMM - 1.0
    }
}

/// A rectangular room with a single doorway of the given width in its south
/// wall, sized so a crowd can build up behind the opening.
fn room_with_door(width_m: f64, room_w: f64, room_h: f64) -> NavMesh {
    let half = width_m / 2.0;
    let mid = room_w / 2.0;

    let pts: Vec<Vec2> = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(mid - half, 0.0),
        Vec2::new(mid + half, 0.0),
        Vec2::new(room_w, 0.0),
        Vec2::new(room_w, room_h),
        Vec2::new(0.0, room_h),
    ];
    // Sealed for classification, then the doorway is reopened — the same dance
    // cf-compile performs, for the same reason.
    let sealed = [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 0)];

    let mut tri = triangulate_constrained(&pts, &sealed).expect("room triangulates");
    tri.compact();
    let regions = classify(&tri);
    tri.constraints.remove(&edge_key(1, 2));
    tri.rebuild_adjacency();
    NavMesh::with_regions(tri, regions)
}

/// Measure saturated flow through a doorway.
///
/// Agents are packed behind the opening and all head for it at once, which is
/// the condition doorway experiments reproduce and the condition an evacuation
/// creates. Flow is measured over the *steady* middle of the discharge, not the
/// whole run: the leading edge and the final stragglers are transients and
/// including them understates the sustained rate.
pub fn measure_doorway_flow(
    width_m: f64,
    agents: u32,
    params: LocomotionParams,
) -> FlowMeasurement {
    let room_w = 12.0;
    let room_h = 10.0;
    let mesh = room_with_door(width_m, room_w, room_h);
    let mid = room_w / 2.0;
    let half = width_m / 2.0;

    let exits = vec![ExitSpan {
        a: Vec2::new(mid - half, 0.0),
        b: Vec2::new(mid + half, 0.0),
    }];

    let mut sim = Sim::new(
        mesh,
        exits,
        SimParams {
            locomotion: params,
            ..SimParams::default()
        },
        20260803,
    );

    // Pack the room on a lattice tight enough to saturate the door but loose
    // enough that nobody starts overlapping.
    let spacing = 0.52;
    let cols = ((room_w - 2.0) / spacing) as u32;
    let mut placed = 0;
    'fill: for row in 0..200u32 {
        for col in 0..cols {
            if placed >= agents {
                break 'fill;
            }
            let x = 1.0 + col as f64 * spacing;
            let y = 1.2 + row as f64 * spacing;
            if y > room_h - 0.6 {
                break 'fill;
            }
            sim.spawn_to_nearest_exit(SpawnParams {
                position: Vec2::new(x, y),
                radius_m: 0.23,
                desired_speed: WEIDMANN_FREE_SPEED as f32,
                goal: 0,
                population: 0,
                entry: 0,
                state: AgentState::Walking,
            });
            placed += 1;
        }
    }

    // Record cumulative exits each tick so the steady window can be found
    // afterwards rather than guessed in advance.
    let mut exited_at_tick: Vec<u32> = Vec::new();
    let mut max_overlap = 0.0f64;
    for _ in 0..8000 {
        let st = sim.step();
        exited_at_tick.push(st.exited);
        max_overlap = max_overlap.max(st.max_overlap as f64);
        if st.active == 0 {
            break;
        }
    }

    let total = *exited_at_tick.last().unwrap_or(&0);
    if total < 10 {
        return FlowMeasurement {
            width_m,
            count: 0,
            duration_s: 0.0,
            specific_flow: 0.0,
            max_overlap,
        };
    }

    // Steady window: between 20% and 80% of the discharge.
    let lo_target = (total as f64 * 0.2) as u32;
    let hi_target = (total as f64 * 0.8) as u32;
    let lo = exited_at_tick
        .iter()
        .position(|c| *c >= lo_target)
        .unwrap_or(0);
    let hi = exited_at_tick
        .iter()
        .position(|c| *c >= hi_target)
        .unwrap_or(exited_at_tick.len() - 1);

    let dt = sim.params.dt;
    let duration_s = (hi.saturating_sub(lo)) as f64 * dt;
    let count = exited_at_tick[hi] - exited_at_tick[lo];

    let specific_flow = if duration_s > 0.0 && width_m > 0.0 {
        count as f64 / (duration_s / 60.0) / width_m
    } else {
        0.0
    };

    FlowMeasurement {
        width_m,
        count,
        duration_s,
        specific_flow,
        max_overlap,
    }
}

/// One point on the fundamental diagram.
#[derive(Clone, Copy, Debug)]
pub struct SpeedDensityPoint {
    pub density: f64,
    /// Mean speed the model produced, m/s.
    ///
    /// This is the speed *along the direction of travel*, which is what a
    /// fundamental diagram means. See `lateral` for why the distinction is not
    /// pedantic.
    pub speed: f64,
    /// What Weidmann's relation predicts at this density, m/s.
    pub weidmann: f64,
    /// Mean |v_y|, m/s — motion across the flow.
    ///
    /// A unidirectional crowd should have very little of this. A large value
    /// means the contact solver is shoving bodies sideways, and a mean *speed*
    /// (the vector magnitude) would report that jitter as if it were progress.
    /// Reported separately so the two can never be confused again.
    pub lateral: f64,
    /// Mean pairwise overlap among neighbours, metres. Should be near zero.
    pub overlap: f64,
}

impl SpeedDensityPoint {
    /// Signed error as a fraction of the reference. Positive means too fast.
    pub fn error(&self) -> f64 {
        if self.weidmann <= 1e-6 {
            return 0.0;
        }
        self.speed / self.weidmann - 1.0
    }
}

/// Measure mean walking speed at a fixed density.
///
/// # Why this runs the locomotion primitives directly
///
/// A fundamental diagram must hold density *constant* while the crowd walks,
/// which needs a periodic domain: an agent leaving the right edge re-enters at
/// the left. Two earlier harnesses failed for want of that.
///
/// The first packed a closed room and aimed everyone at a goal outside the
/// mesh; pathfinding failed, agents walked into a wall and piled up, and the
/// "speed" measured was how fast a stationary pile jitters — which made walking
/// speed appear to *rise* with density.
///
/// The second used a long corridor and let the crowd walk into the empty
/// remainder. That measures a dispersing crowd: density falls from the target
/// the moment the run starts, so every reading came back near free speed
/// regardless of how the crowd was packed.
///
/// So this drives `compute_forces`, `integrate` and `resolve_contacts` itself,
/// wraps positions each tick, and sets the desired velocity directly rather
/// than routing. Neighbours across the seam are missed by the spatial grid, so
/// the sampled band is kept away from both edges.
pub fn measure_speed_at_density(density: f64, params: LocomotionParams) -> SpeedDensityPoint {
    use crate::locomotion;
    use crate::spatial::SpatialGrid;
    use crate::world::World;
    use cf_geom::Aabb;

    let length = 30.0;
    let width = 12.0;
    let spacing = (1.0 / density).sqrt();

    let mut w = World::new();
    let mut y = spacing * 0.5;
    while y < width {
        let mut x = spacing * 0.5;
        while x < length {
            w.spawn(SpawnParams {
                position: Vec2::new(x, y),
                radius_m: 0.23,
                desired_speed: WEIDMANN_FREE_SPEED as f32,
                goal: 0,
                population: 0,
                entry: 0,
                state: AgentState::Walking,
            });
            x += spacing;
        }
        y += spacing;
    }
    if w.is_empty() {
        return SpeedDensityPoint {
            density,
            speed: 0.0,
            weidmann: weidmann_speed(density),
            lateral: 0.0,
            overlap: 0.0,
        };
    }

    let bounds = Aabb {
        min: Vec2::new(-2.0, -2.0),
        max: Vec2::new(length + 2.0, width + 2.0),
    };
    let mut grid = SpatialGrid::new(bounds, params.interaction_range as f64);
    let mut scratch = locomotion::LocomotionScratch::default();
    let dt = 0.05f32;

    let mut total = 0.0;
    let mut total_lateral = 0.0;
    let mut samples = 0u32;
    let mut overlap_sum = 0.0;
    let mut overlap_n = 0u32;

    for tick in 0..300 {
        grid.rebuild(&w.pos_x, &w.pos_y, &w.active);

        // Everyone walks +x at their free speed; the density limit and the
        // forces then decide what they actually achieve.
        for i in 0..w.len() {
            w.des_x[i] = w.desired_speed[i];
            w.des_y[i] = 0.0;
        }
        locomotion::apply_density_speed_limit(&mut w, &grid, &[], &params);
        locomotion::compute_forces(&w, &grid, &params, &mut scratch);
        scratch.snapshot_positions(&w);
        locomotion::integrate(&mut w, &params, &scratch, dt);
        locomotion::resolve_contacts(&mut w, &grid, &params, &mut scratch, dt);
        locomotion::derive_velocity_from_positions(&mut w, &scratch, dt);

        // Periodic in x, and reflect in y so the crowd cannot drift out.
        for i in 0..w.len() {
            if w.pos_x[i] >= length as f32 {
                w.pos_x[i] -= length as f32;
            } else if w.pos_x[i] < 0.0 {
                w.pos_x[i] += length as f32;
            }
            w.pos_y[i] = w.pos_y[i].clamp(0.25, width as f32 - 0.25);
        }

        // Skip the first 5 s of acceleration, then sample the interior.
        if tick >= 100 {
            for i in 0..w.len() {
                let x = w.pos_x[i] as f64;
                let y = w.pos_y[i] as f64;
                // Away from the seam, where neighbour queries are incomplete,
                // and away from the y walls.
                if x < 4.0 || x > length - 4.0 || y < 1.5 || y > width - 1.5 {
                    continue;
                }
                let v = w.velocity(i as u32);
                // Along the flow, not the vector magnitude — see `lateral`.
                total += v.x;
                total_lateral += v.y.abs();
                samples += 1;

                // How far bodies are interpenetrating. If the contact solve is
                // healthy this is ~0 and the crowd is not boiling.
                for j in (i + 1)..w.len() {
                    let dx = w.pos_x[j] - w.pos_x[i];
                    let dy = w.pos_y[j] - w.pos_y[i];
                    let d2 = dx * dx + dy * dy;
                    let rsum = w.radius[i] + w.radius[j];
                    if d2 < rsum * rsum && d2 > 1e-12 {
                        overlap_sum += (rsum - d2.sqrt()) as f64;
                        overlap_n += 1;
                    }
                }
            }
        }
    }

    SpeedDensityPoint {
        density,
        speed: if samples > 0 {
            total / samples as f64
        } else {
            0.0
        },
        weidmann: weidmann_speed(density),
        lateral: if samples > 0 {
            total_lateral / samples as f64
        } else {
            0.0
        },
        overlap: if overlap_n > 0 {
            overlap_sum / overlap_n as f64
        } else {
            0.0
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weidmann_curve_has_the_right_shape() {
        // Free flow at vanishing density.
        assert!((weidmann_speed(0.05) - WEIDMANN_FREE_SPEED).abs() < 0.1);
        // Monotonically decreasing.
        let mut prev = f64::INFINITY;
        for i in 1..54 {
            let v = weidmann_speed(i as f64 / 10.0);
            assert!(
                v <= prev + 1e-9,
                "speed rose at density {}",
                i as f64 / 10.0
            );
            prev = v;
        }
        // Movement ceases at the jam density.
        assert_eq!(weidmann_speed(5.4), 0.0);
        // Around 1 person/m² a crowd still moves at roughly walking pace.
        let v1 = weidmann_speed(1.0);
        assert!((0.9..1.3).contains(&v1), "v(1.0) = {v1}");
    }

    /// The headline calibration check. **Currently failing by design.**
    ///
    /// Ignored rather than deleted or weakened: the model is not yet calibrated,
    /// and the honest state of the project is "we measure this, we know the
    /// number, it is wrong". Weakening the band until it passes would turn the
    /// harness into decoration. Run with:
    ///
    /// ```text
    /// cargo test -p cf-sim calibration -- --ignored --nocapture
    /// ```
    #[test]
    fn doorway_flow_is_within_the_measured_band() {
        let m = measure_doorway_flow(1.0, 200, LocomotionParams::default());
        println!(
            "1.0 m doorway: {:.1} persons/m/min over {:.1} s ({} people), \
             {:+.0}% vs Green Guide {GREEN_GUIDE_LEVEL_PPMM}",
            m.specific_flow,
            m.duration_s,
            m.count,
            m.error_vs_green_guide() * 100.0
        );

        assert!(m.count > 20, "too few agents passed to measure a rate");

        // Measured doorway experiments cluster at 1.2–1.4 persons/m/s, i.e.
        // 72–84 per minute. Allow a generous band around that; the point is to
        // catch a model that is wrong by a factor, not to over-fit.
        assert!(
            (60.0..100.0).contains(&m.specific_flow),
            "specific flow {:.1} p/m/min is outside the measured band of 60–100",
            m.specific_flow
        );
    }

    /// Specific flow is per metre of clear width, so it should be roughly
    /// width-independent. It was not — 5.45x between a 0.9 m and a 1.8 m door
    /// — because a narrow opening jammed and a wide one did not. Directional
    /// density sensing fixed the jam and with it the width dependence.
    #[test]
    fn flow_scales_with_doorway_width() {
        let narrow = measure_doorway_flow(0.9, 150, LocomotionParams::default());
        let wide = measure_doorway_flow(1.8, 250, LocomotionParams::default());
        println!(
            "0.9 m: {:.1} p/m/min · 1.8 m: {:.1} p/m/min",
            narrow.specific_flow, wide.specific_flow
        );

        // Specific flow is per metre, so doubling the width should roughly
        // double throughput and leave the *specific* rate similar. A model
        // where it collapses is not resolving the doorway.
        let ratio = wide.specific_flow / narrow.specific_flow;
        assert!(
            (0.6..1.6).contains(&ratio),
            "specific flow changed by {ratio:.2}x between 0.9 m and 1.8 m doors"
        );
    }

    /// A parameter sweep, kept as a tool rather than an assertion.
    ///
    /// `a_agent` came from Helbing's paper as **2000 N** and was used directly
    /// as an acceleration, which is a factor of body mass (~80 kg) too large.
    /// This sweep is what established that; it is left in so the next person
    /// re-tuning the model has the instrument rather than the conclusion.
    ///
    /// ```text
    /// cargo test -p cf-sim sweep_agent_repulsion -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "diagnostic tool, not an assertion"]
    fn sweep_agent_repulsion() {
        println!(
            "{:>8} {:>9} {:>9} {:>10} {:>10} {:>9}",
            "a_agent", "v(1.0)", "v(2.0)", "lateral(2)", "door 1.0m", "overlap"
        );
        for a in [2000.0f32, 200.0, 50.0, 25.0, 12.0, 6.0] {
            let params = LocomotionParams {
                a_agent: a,
                a_wall: a,
                ..LocomotionParams::default()
            };
            let p1 = measure_speed_at_density(1.0, params);
            let p2 = measure_speed_at_density(2.0, params);
            let door = measure_doorway_flow(1.0, 200, params);
            println!(
                "{a:>8.0} {:>9.2} {:>9.2} {:>10.2} {:>10.1} {:>9.3}",
                p1.speed, p2.speed, p2.lateral, door.specific_flow, door.max_overlap
            );
        }
        println!("reference: v(1.0)=1.06  v(2.0)=0.61  lateral~0  door=82  overlap~0");
    }

    #[test]
    fn speed_falls_as_density_rises() {
        let mut prev = f64::INFINITY;
        for d in [0.5, 1.0, 2.0, 3.0] {
            let p = measure_speed_at_density(d, LocomotionParams::default());
            println!(
                "ρ = {:.1}: model {:.2} m/s, Weidmann {:.2} m/s ({:+.0}%) \
                 · lateral {:.2} m/s · overlap {:.3} m",
                p.density,
                p.speed,
                p.weidmann,
                p.error() * 100.0,
                p.lateral,
                p.overlap
            );
            assert!(
                p.speed <= prev + 0.05,
                "speed rose from {prev:.2} to {:.2} as density increased",
                p.speed
            );
            prev = p.speed;
        }
    }
}
