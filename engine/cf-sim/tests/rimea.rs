//! The RiMEA verification suite.
//!
//! RiMEA (*Richtlinie für Mikroskopische Entfluchtungsanalysen*) is the
//! de-facto acceptance suite for microscopic evacuation models, and
//! `docs/06-validation.md` §2 makes passing it a V1 gate. Each test below
//! isolates one behaviour and checks it against a number that came from
//! somewhere other than this codebase.
//!
//! # Numbering
//!
//! Case numbers follow the grouping used in `docs/06-validation.md`. That
//! document warns — correctly — that numbering and tolerances must be taken
//! from the current published guideline rather than a secondary summary, and
//! that reconciliation has **not** yet been done. Treat the numbers as labels
//! for the behaviour described in each test's doc comment, not as citations.
//!
//! # Tests that fail
//!
//! The locomotion model is not yet calibrated (`docs/06-validation.md` §8), so
//! some of these do not pass. They are marked `#[ignore]` with the measured
//! value and the reference value recorded in the doc comment, exactly as
//! `cf_sim::calibration` does. **Do not weaken a threshold to make one pass** —
//! the measured numbers are the output that drives calibration, and a suite
//! tuned until it is green measures nothing.
//!
//! ```text
//! cargo test -p cf-sim --test rimea -- --ignored --nocapture
//! ```
//!
//! # Structural checks are kept separate from calibration checks
//!
//! "Everybody got out, nobody walked through a wall, nobody was lost" holds
//! regardless of how the force constants are tuned, so those assertions live in
//! their own tests and run on every PR. Only the checks that compare against a
//! measured human number are ignored.

use cf_geom::Vec2;
use cf_navmesh::{classify, edge_key, triangulate_constrained, NavMesh};
use cf_schema::dist::Distribution;
use cf_sim::calibration::{measure_speed_at_density, weidmann_speed};
use cf_sim::world::{AgentState, SpawnParams};
use cf_sim::{ExitSpan, LocomotionParams, Rng, Sim, SimParams, Stream};

/// RiMEA's nominal free walking speed on the level, m/s.
const RIMEA_SPEED: f32 = 1.33;

/// Body radius used throughout, matching the rest of the engine's fixtures.
const RADIUS: f32 = 0.23;

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

/// Build a navmesh from a point list, a set of wall edges and a set of doorways.
///
/// Doorways are triangulated as walls, classified, then reopened. That is the
/// same dance `cf-compile` performs: [`classify`] needs a closed outline to
/// decide what is inside, but the physics must not see a wall where a door is.
fn mesh(pts: &[Vec2], walls: &[(usize, usize)], doors: &[(usize, usize)]) -> NavMesh {
    let mut sealed: Vec<(usize, usize)> = walls.to_vec();
    sealed.extend_from_slice(doors);

    let mut tri = triangulate_constrained(pts, &sealed).expect("geometry triangulates");
    tri.compact();
    let regions = classify(&tri);
    for (a, b) in doors {
        tri.constraints.remove(&edge_key(*a, *b));
    }
    tri.rebuild_adjacency();
    NavMesh::with_regions(tri, regions)
}

/// The closed ring of edges `0-1-2-…-(n-1)-0`.
fn ring(n: usize) -> Vec<(usize, usize)> {
    (0..n).map(|i| (i, (i + 1) % n)).collect()
}

/// Every edge of `ring(n)` except the ones listed as doorways.
fn ring_walls(n: usize, doors: &[(usize, usize)]) -> Vec<(usize, usize)> {
    ring(n)
        .into_iter()
        .filter(|e| {
            !doors
                .iter()
                .any(|d| edge_key(d.0, d.1) == edge_key(e.0, e.1))
        })
        .collect()
}

/// A doorway span between two boundary points.
fn door(pts: &[Vec2], a: usize, b: usize) -> ExitSpan {
    ExitSpan {
        a: pts[a],
        b: pts[b],
    }
}

fn person(x: f64, y: f64, speed: f32) -> SpawnParams {
    SpawnParams {
        position: Vec2::new(x, y),
        radius_m: RADIUS,
        desired_speed: speed,
        goal: 0,
        population: 0,
        entry: 0,
        state: AgentState::Walking,
    }
}

/// Health of a whole run, accumulated tick by tick.
///
/// Collected together because the three signals are only meaningful as a set:
/// an evacuation that finishes quickly by leaking agents through a wall is not
/// a fast evacuation.
struct RunHealth {
    ticks: u64,
    /// Simulation time at which the last agent left, or `None` if some remain.
    cleared_at: Option<f64>,
    /// Agents recovered from outside the navmesh, summed over the run.
    escaped: u32,
    /// Worst residual body overlap seen, metres.
    max_overlap: f32,
    /// True if `active + exited == spawned` held at every single tick.
    population_conserved: bool,
}

/// Run to completion, watching everything that must hold whatever the
/// calibration turns out to be.
fn run_watched(sim: &mut Sim, max_ticks: u64) -> RunHealth {
    let mut h = RunHealth {
        ticks: 0,
        cleared_at: None,
        escaped: 0,
        max_overlap: 0.0,
        population_conserved: true,
    };
    for _ in 0..max_ticks {
        let st = sim.step();
        h.ticks = st.tick;
        h.escaped += st.escaped;
        h.max_overlap = h.max_overlap.max(st.max_overlap);
        if st.active + st.exited != st.spawned {
            h.population_conserved = false;
        }
        if st.active == 0 {
            h.cleared_at = Some(st.time);
            break;
        }
    }
    h
}

/// Assert the three properties that hold regardless of parameter values.
fn assert_sound(h: &RunHealth, what: &str) {
    assert!(
        h.population_conserved,
        "{what}: population was not conserved — agents were silently lost"
    );
    assert_eq!(
        h.escaped, 0,
        "{what}: {} agent-ticks spent outside the navmesh — bodies are passing through walls",
        h.escaped
    );
    // The bar `tests/packing.rs` already holds the contact solve to.
    assert!(
        h.max_overlap < 0.10,
        "{what}: bodies interpenetrated by {:.3} m — the contact solve is not holding",
        h.max_overlap
    );
}

/// Exit times in agent-id order. Panics if anyone is still inside.
fn exit_times(sim: &Sim, what: &str) -> Vec<f64> {
    (0..sim.world.len())
        .map(|i| {
            sim.world.cold[i]
                .exited_at
                .unwrap_or_else(|| panic!("{what}: agent {i} never left"))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// TC1 — maintaining walking speed in a corridor
// ---------------------------------------------------------------------------

/// **TC1.** A person in a 2 m wide corridor walks 40 m at their assigned speed.
///
/// `docs/06-validation.md` §2 states the criterion as "time within ±2% of
/// `length / desired_speed`". Two numbers are taken from one run:
///
/// - **Steady-state**, measured over 40 m *after* a 5 m run-up. This is the
///   criterion above, and it is the one that isolates "does an unobstructed
///   agent hold its assigned speed" from "how fast does it get up to speed".
/// - **Standing start**, 40 m from rest, which is the case RiMEA describes.
///   The only legitimate excess over `L / v₀` is the acceleration transient:
///   with a relaxation time τ the agent's position lags the ideal by `v₀·τ`
///   metres for ever after, i.e. exactly τ = 0.5 s of extra time. Anything
///   beyond about 1 s of excess is the model losing speed, not accelerating.
///
/// Note the model cannot reach exactly `v₀`: an isolated agent still senses
/// itself inside its own 1 m density disc (1/π = 0.32 persons/m²), and
/// `apply_density_speed_limit` takes 0.35% off for it. That is a real modelling
/// choice, not noise, and it is why the tolerance is not tighter than ±2%.
#[test]
fn tc1_an_agent_holds_its_walking_speed_along_a_corridor() {
    // A 5 m run-up, then the 40 m measured section, then 5 m of overrun so the
    // agent is not being removed at the doorway while still being measured.
    let pts = vec![
        Vec2::new(-5.0, 0.0),
        Vec2::new(45.0, 0.0),
        Vec2::new(45.0, 2.0),
        Vec2::new(-5.0, 2.0),
    ];
    let doors = [(1usize, 2usize)];
    let m = mesh(&pts, &ring_walls(4, &doors), &doors);
    let exits = vec![door(&pts, 1, 2)];

    let mut sim = Sim::new(m, exits, SimParams::default(), 20260803);
    sim.spawn_to_nearest_exit(person(-4.5, 1.0, RIMEA_SPEED));

    let dt = sim.params.dt;
    let mut prev_x = sim.world.pos_x[0] as f64;
    // Crossing times at x = 0 (start of the measured section), x = 35.5 (40 m
    // from the standing start at x = -4.5) and x = 40 (end of it).
    let mut at_0 = None;
    let mut at_35_5 = None;
    let mut at_40 = None;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for tick in 1..2000u64 {
        sim.step();
        if !sim.world.is_active(0) {
            break;
        }
        let x = sim.world.pos_x[0] as f64;
        let y = sim.world.pos_y[0] as f64;
        min_y = min_y.min(y);
        max_y = max_y.max(y);

        // Linear interpolation between ticks: at 1.33 m/s a tick covers 6.7 cm,
        // and quantising both ends of a 30 s measurement to 50 ms would spend a
        // third of the ±2% budget on the clock alone.
        let cross = |target: f64, slot: &mut Option<f64>| {
            if slot.is_none() && x >= target && prev_x < target {
                let frac = (target - prev_x) / (x - prev_x);
                *slot = Some((tick - 1) as f64 * dt + frac * dt);
            }
        };
        cross(0.0, &mut at_0);
        cross(35.5, &mut at_35_5);
        cross(40.0, &mut at_40);
        if at_40.is_some() {
            break;
        }
        prev_x = x;
    }

    let t0 = at_0.expect("agent never reached the start of the measured section");
    let t1 = at_40.expect("agent never reached the end of the measured section");
    let from_rest = at_35_5.expect("agent never covered 40 m from a standing start");

    let steady = 40.0 / (t1 - t0);
    let ideal = 40.0 / RIMEA_SPEED as f64;
    println!(
        "TC1: steady-state {steady:.4} m/s over 40 m ({:+.2}% vs {RIMEA_SPEED} m/s); \
         40 m from rest in {from_rest:.2} s (ideal {ideal:.2} s, {:+.2}%)",
        (steady / RIMEA_SPEED as f64 - 1.0) * 100.0,
        (from_rest / ideal - 1.0) * 100.0
    );

    // The corridor is 2 m wide; a body of radius 0.23 m must stay inside it.
    assert!(
        min_y >= RADIUS as f64 - 1e-3 && max_y <= 2.0 - RADIUS as f64 + 1e-3,
        "agent left the corridor: y ranged {min_y:.3}..{max_y:.3}"
    );

    let err = steady / RIMEA_SPEED as f64 - 1.0;
    assert!(
        err.abs() <= 0.02,
        "steady walking speed {steady:.4} m/s is {:+.2}% off the assigned \
         {RIMEA_SPEED} m/s — docs/06-validation.md §2 allows ±2%",
        err * 100.0
    );

    // From rest, only the acceleration transient may be added: one relaxation
    // time, τ = 0.5 s. Allow 1 s and no more.
    assert!(
        from_rest >= ideal - 1e-6 && from_rest <= ideal + 1.0,
        "40 m from a standing start took {from_rest:.2} s; the ideal is \
         {ideal:.2} s and only the acceleration transient (~τ = {} s) may be added",
        SimParams::default().locomotion.tau
    );
}

// ---------------------------------------------------------------------------
// TC2 — maintaining walking speed on stairs
// ---------------------------------------------------------------------------

/// **TC2.** Assigned speed reduction applied going up and down stairs.
///
/// **Cannot be written: `cf-sim` does not model stairs at all.**
///
/// The *data contract* has everything needed —
/// `cf_schema::venue::VerticalLink` carries `speed_multiplier_up`,
/// `speed_multiplier_down`, `flow_rate_ppmm`, `riser_m` and `going_m`, and
/// `cf_schema::venue::Zone` carries a `speed_multiplier`. None of it reaches
/// the simulation: [`cf_sim::Sim`] holds a single [`cf_navmesh::NavMesh`] with
/// no floor identity, no vertical links, and nothing anywhere in
/// `cf_sim::locomotion` reads a per-zone or per-triangle speed multiplier.
/// `desired_speed` is a per-agent constant for the whole run.
///
/// Two things are needed before this test can say anything:
///
/// 1. A per-triangle (or per-zone) speed multiplier that `Sim::steer` applies
///    to `desired_speed`, so a "stair" region slows agents crossing it.
/// 2. Multi-floor navigation, so an agent can traverse a `VerticalLink`
///    between two meshes.
///
/// Item 1 alone is enough for TC2 itself — a stair can be verified as a plane
/// region with a multiplier before vertical circulation exists. Item 2 is what
/// `docs/06-validation.md` §3 "Vertical circulation" additionally needs.
#[test]
#[ignore = "stairs are not modelled: cf-sim has no vertical links and no per-zone speed multiplier; see docs/06-validation.md"]
fn tc2_walking_speed_on_stairs_is_reduced() {
    panic!(
        "not implementable: cf_sim::Sim has no notion of a stair. \
         cf_schema::venue::VerticalLink::speed_multiplier_up / _down and \
         cf_schema::venue::Zone::speed_multiplier exist in the data contract \
         but nothing in cf-sim reads them, and desired_speed is constant for \
         an agent's whole run. See this test's doc comment for what is needed."
    );
}

// ---------------------------------------------------------------------------
// TC3 — movement round a corner
// ---------------------------------------------------------------------------

/// An L-shaped corridor of the given clear width: `leg` metres east, then
/// `leg` metres north. The doorway is the full width of the north end.
fn corner_corridor(leg: f64, width: f64) -> (Vec<Vec2>, NavMesh, Vec<ExitSpan>) {
    let outer = leg + width;
    let pts = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(outer, 0.0),
        Vec2::new(outer, outer),
        Vec2::new(leg, outer),
        Vec2::new(leg, width),
        Vec2::new(0.0, width),
    ];
    let doors = [(2usize, 3usize)];
    let m = mesh(&pts, &ring_walls(6, &doors), &doors);
    let exits = vec![door(&pts, 2, 3)];
    (pts, m, exits)
}

/// **TC3.** Twenty people round a 90° corner without passing through the wall
/// or deadlocking.
///
/// The criterion in `docs/06-validation.md` §2 is qualitative — "zero wall
/// penetration; no deadlock; smooth trajectory" — so it is checked as three
/// hard assertions rather than a tolerance. All three hold independently of
/// calibration, which is why this test is not ignored.
///
/// The wall-penetration check reads [`cf_sim::SimStats::escaped`], the count of
/// agents found outside the navmesh and put back. That counter exists precisely
/// because soft wall repulsion was observed to lose agents at corners under
/// load, and a corner under load is exactly this test.
#[test]
fn tc3_twenty_people_round_a_corner_without_leaving_the_corridor() {
    let (_pts, m, exits) = corner_corridor(20.0, 2.0);
    let mut sim = Sim::new(m, exits, SimParams::default(), 20260803);

    // Two ranks of ten in the horizontal leg.
    for i in 0..20 {
        let x = 1.0 + (i / 2) as f64 * 0.9;
        let y = if i % 2 == 0 { 0.6 } else { 1.4 };
        sim.spawn_to_nearest_exit(person(x, y, RIMEA_SPEED));
    }

    // The longest route is about 20 + 20 = 40 m; 150 s is ample even with
    // heavy queueing at the corner, so exhausting it means a deadlock.
    let h = run_watched(&mut sim, 3000);
    println!(
        "TC3: 20 agents cleared a 90° corner in {:?} s (escaped {}, worst overlap {:.4} m)",
        h.cleared_at, h.escaped, h.max_overlap
    );

    assert!(
        h.cleared_at.is_some(),
        "TC3: {} agents were still in the corridor after 150 s — deadlock at the corner",
        sim.stats().active
    );
    assert_sound(&h, "TC3");
}

// ---------------------------------------------------------------------------
// TC4 — fundamental diagram in a corridor
// ---------------------------------------------------------------------------

/// **TC4.** The speed–density relation in a corridor must sit near the
/// published curve.
///
/// Measured with [`measure_speed_at_density`] rather than a second harness of
/// this test's own. That function's doc comment records two earlier harnesses
/// that measured the wrong thing — a stationary pile jittering, and a
/// dispersing crowd — and re-deriving it here would be re-earning those bugs.
///
/// **Currently failing.** Measured against Weidmann (1993), default params:
///
/// | ρ (p/m²) | model | Weidmann | error |
/// |---|---|---|---|
/// | 0.5 | 1.33 m/s | 1.30 m/s | +3% |
/// | 1.0 | 1.04 m/s | 1.06 m/s | −2% |
/// | 1.5 | 0.76 m/s | 0.81 m/s | −6% |
/// | 2.0 | **2.13 m/s** | 0.61 m/s | **+252%** |
/// | 3.0 | **2.15 m/s** | 0.33 m/s | **+549%** |
///
/// The two large numbers are above free walking speed and are pinned at the
/// speed cap (`1.34 × max_speed_factor 1.6 = 2.14`), which is physically
/// impossible for a crowd at 2 persons/m² and so is a signal about the
/// measurement as much as the model. Measuring *net transport* along the
/// corridor instead of the magnitude of instantaneous velocity, on the same
/// harness, gives 0.12 m/s at ρ = 1.5 and **0.00 m/s at ρ ≥ 2.0**: the crowd is
/// not moving at all, it is vibrating in place at the speed cap, and mean |v|
/// is reading the vibration. So there are two defects stacked here, in opposite
/// directions — see `docs/06-validation.md` §4.1.
#[test]
#[ignore = "one repulsion constant cannot satisfy both this and doorway flow; \
the model is deliberately tuned to the door, which is the safe side. \
-89% at 3 p/m²; see docs/06-validation.md §4.1"]
fn tc4_fundamental_diagram_matches_weidmann() {
    let mut failures = Vec::new();
    for d in [0.5, 1.0, 1.5, 2.0, 3.0] {
        let p = measure_speed_at_density(d, LocomotionParams::default());
        println!(
            "TC4: ρ = {:.1} → model {:.3} m/s, Weidmann {:.3} m/s ({:+.0}%)",
            p.density,
            p.speed,
            p.weidmann,
            p.error() * 100.0
        );
        // A quarter of the reference value is already a loose band for a curve
        // this well measured; it is set to catch a model wrong by a factor,
        // not to over-fit.
        if p.error().abs() > 0.25 {
            failures.push(format!(
                "ρ = {:.1}: {:.3} m/s vs {:.3} m/s ({:+.0}%)",
                p.density,
                p.speed,
                p.weidmann,
                p.error() * 100.0
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "fundamental diagram outside ±25% of Weidmann at:\n  {}",
        failures.join("\n  ")
    );
}

/// The shape of the reference curve itself, independent of the model.
///
/// Cheap insurance: if [`weidmann_speed`] were ever edited, every comparison
/// above would move together and none of them would notice.
#[test]
fn tc4_the_reference_curve_is_the_published_one() {
    // Weidmann's own free walking speed and jam density.
    assert!((weidmann_speed(0.01) - 1.34).abs() < 0.01);
    assert_eq!(weidmann_speed(5.4), 0.0);
    // Fruin's Level of Service C/D boundary sits near 1 person/m², where a
    // crowd still moves at close to normal pace.
    assert!((0.9..1.3).contains(&weidmann_speed(1.0)));
    // Strictly decreasing across the whole range.
    let mut prev = f64::INFINITY;
    for i in 1..540 {
        let v = weidmann_speed(i as f64 / 100.0);
        assert!(v <= prev + 1e-12, "curve rose at ρ = {}", i as f64 / 100.0);
        prev = v;
    }
}

// ---------------------------------------------------------------------------
// TC6 — movement round a corner without overtaking
// ---------------------------------------------------------------------------

/// **TC6.** A single-file column rounds a corner and keeps its order.
///
/// The corridor is 0.8 m of clear width. Two bodies of radius 0.23 m need
/// 0.92 m to pass abreast, and the hard wall constraint keeps each body 0.23 m
/// off both walls, leaving a usable band of 0.34 m — so overtaking is
/// geometrically impossible and any reordering means bodies passed *through*
/// each other. That makes this a contact-solver test as much as a corner test.
#[test]
fn tc6_a_single_file_column_rounds_a_corner_without_overtaking() {
    let (_pts, m, exits) = corner_corridor(20.0, 0.8);
    let mut sim = Sim::new(m, exits, SimParams::default(), 20260803);

    // Agent 0 leads, at the greatest x; each subsequent agent is 0.8 m behind.
    for i in 0..10 {
        sim.spawn_to_nearest_exit(person(8.0 - i as f64 * 0.8, 0.4, RIMEA_SPEED));
    }

    let h = run_watched(&mut sim, 4000);
    assert!(
        h.cleared_at.is_some(),
        "TC6: {} agents never left the corridor — deadlock",
        sim.stats().active
    );
    assert_sound(&h, "TC6");

    let times = exit_times(&sim, "TC6");
    println!(
        "TC6: exit times {:?}",
        times
            .iter()
            .map(|t| (t * 100.0).round() / 100.0)
            .collect::<Vec<_>>()
    );

    for i in 1..times.len() {
        assert!(
            times[i] > times[i - 1],
            "TC6: agent {i} left at {:.2} s, ahead of agent {} at {:.2} s — \
             the column reordered in a corridor too narrow to overtake in",
            times[i],
            i - 1,
            times[i - 1]
        );
    }
}

// ---------------------------------------------------------------------------
// TC7 — distribution of walking speeds over a population
// ---------------------------------------------------------------------------

/// Sup-norm deviation between the sampled population and its target
/// distribution — the Kolmogorov–Smirnov statistic.
///
/// Evaluated at the target's own quantiles rather than against a normal CDF
/// approximation. Because `sample_icdf` is monotone,
/// `P(X ≤ icdf(q)) = P(U ≤ q) = q` exactly, so comparing the empirical
/// fraction below `icdf(q)` against `q` needs no error function and introduces
/// no approximation of its own.
fn ks_statistic(sorted: &[f64], dist: &Distribution) -> f64 {
    let n = sorted.len() as f64;
    let mut worst = 0.0f64;
    for k in 1..1000 {
        let q = k as f64 / 1000.0;
        let x = dist.sample_icdf(q);
        let below = sorted.partition_point(|v| *v <= x) as f64 / n;
        worst = worst.max((below - q).abs());
    }
    worst
}

/// **TC7.** A population's sampled walking speeds reproduce the distribution
/// they were specified from.
///
/// This exercises the exact pairing the product uses: `cf-wasm` spawns agents
/// with `speed_dist.sample_icdf(rng.uniform01(Stream::DesiredSpeed, i, 0))`,
/// and that composition is what is tested here rather than either half alone.
///
/// The acceptance criterion is the one in `docs/06-validation.md` §2: a KS test
/// against the target distribution at p > 0.05, i.e. `D < 1.36/√n`.
#[test]
fn tc7_a_population_reproduces_its_specified_speed_distribution() {
    // The engine default: Weidmann's 1.34 m/s mean, sd 0.26, clipped to a
    // plausible human range.
    let dist = Distribution::Normal {
        mean: 1.34,
        sd: 0.26,
        min: Some(0.60),
        max: Some(2.20),
    };
    let rng = Rng::new(20260803);
    let n = 2000u64;

    let mut speeds: Vec<f64> = (0..n)
        .map(|i| dist.sample_icdf(rng.uniform01(Stream::DesiredSpeed, i, 0)))
        .collect();

    // Every sampled speed must be a speed a person could walk at. An unclamped
    // normal's quantile function is unbounded, so this is a real hazard.
    for (i, v) in speeds.iter().enumerate() {
        assert!(
            (0.60..=2.20).contains(v),
            "agent {i} was assigned {v} m/s, outside the specified 0.60–2.20 band"
        );
    }

    let mean = speeds.iter().sum::<f64>() / n as f64;
    let var = speeds.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
    let sd = var.sqrt();

    speeds.sort_by(|a, b| a.partial_cmp(b).expect("no NaN speeds"));
    let d = ks_statistic(&speeds, &dist);
    // Two-sided KS critical value at α = 0.05.
    let crit = 1.36 / (n as f64).sqrt();

    println!(
        "TC7: n = {n}, mean {mean:.4} m/s (target 1.34), sd {sd:.4} (target 0.26), \
         KS D = {d:.4} (critical {crit:.4})"
    );

    assert!(
        d < crit,
        "TC7: KS statistic {d:.4} exceeds the α = 0.05 critical value {crit:.4} — \
         the sampled population does not follow its specified distribution"
    );
    // The clip at ±2.85σ / +3.3σ removes about 0.2% of the mass, so the moments
    // move by a little. These bands are wide enough for that and no wider.
    assert!(
        (mean - 1.34).abs() < 0.02,
        "mean walking speed {mean:.4} m/s"
    );
    assert!((sd - 0.26).abs() < 0.02, "walking speed sd {sd:.4} m/s");
}

/// **TC7 (demographic form).** Splitting a population into age/sex cohorts with
/// different assigned speeds reproduces both the cohort proportions and each
/// cohort's own speed distribution.
///
/// RiMEA states TC7 in terms of a demographic table, so the check is not just
/// "the marginal distribution is right" — it is that a named cohort gets the
/// speeds that cohort was specified with. Cohort membership and speed are drawn
/// from *different* [`Stream`]s, which is what stops adding a demographic model
/// from silently changing every agent's speed.
#[test]
fn tc7_demographic_cohorts_each_get_their_own_speed_distribution() {
    // A coarse stand-in for RiMEA's age/sex table. Real values belong in a
    // fixture once the guideline table is transcribed; the property under test
    // is that per-cohort sampling works, not these particular numbers.
    let cohorts: [(&str, f64, Distribution); 3] = [
        (
            "under 30",
            0.35,
            Distribution::Normal {
                mean: 1.52,
                sd: 0.20,
                min: Some(0.8),
                max: Some(2.4),
            },
        ),
        (
            "30 to 50",
            0.40,
            Distribution::Normal {
                mean: 1.41,
                sd: 0.19,
                min: Some(0.8),
                max: Some(2.4),
            },
        ),
        (
            "over 50",
            0.25,
            Distribution::Normal {
                mean: 1.13,
                sd: 0.22,
                min: Some(0.5),
                max: Some(2.0),
            },
        ),
    ];
    let weights: Vec<f64> = cohorts.iter().map(|c| c.1).collect();

    let rng = Rng::new(20260803);
    let n = 6000u64;
    let mut members: Vec<Vec<f64>> = vec![Vec::new(); cohorts.len()];

    for i in 0..n {
        // Cohort from one stream, speed from another.
        let c = rng.weighted(Stream::MobilityImpaired, i, 0, &weights);
        let u = rng.uniform01(Stream::DesiredSpeed, i, 0);
        members[c].push(cohorts[c].2.sample_icdf(u));
    }

    for (c, (name, share, dist)) in cohorts.iter().enumerate() {
        let observed_share = members[c].len() as f64 / n as f64;
        let mean = members[c].iter().sum::<f64>() / members[c].len() as f64;
        println!(
            "TC7: cohort '{name}' — share {observed_share:.3} (target {share:.3}), \
             mean {mean:.3} m/s (target {:.3})",
            dist.mean()
        );
        assert!(
            (observed_share - share).abs() < 0.02,
            "cohort '{name}' took {observed_share:.3} of the population, specified {share:.3}"
        );
        assert!(
            (mean - dist.mean()).abs() < 0.03,
            "cohort '{name}' walks at {mean:.3} m/s, specified {:.3} m/s",
            dist.mean()
        );
    }
}

// ---------------------------------------------------------------------------
// TC8 — evacuation of a room with a single exit
// ---------------------------------------------------------------------------

/// A square room with a single doorway of `door_w` centred in its south wall.
fn room_with_one_door(side: f64, door_w: f64) -> (Vec<Vec2>, NavMesh, Vec<ExitSpan>) {
    let mid = side / 2.0;
    let half = door_w / 2.0;
    let pts = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(mid - half, 0.0),
        Vec2::new(mid + half, 0.0),
        Vec2::new(side, 0.0),
        Vec2::new(side, side),
        Vec2::new(0.0, side),
    ];
    let doors = [(1usize, 2usize)];
    let m = mesh(&pts, &ring_walls(6, &doors), &doors);
    let exits = vec![door(&pts, 1, 2)];
    (pts, m, exits)
}

/// Fill a `side` × `side` room with `n` agents on a lattice, clear of the walls.
fn fill_room(sim: &mut Sim, n: u32, side: f64, spacing: f64) {
    let cols = ((side - 2.0) / spacing) as u32;
    let mut placed = 0;
    'fill: for row in 0.. {
        for col in 0..cols {
            if placed >= n {
                break 'fill;
            }
            let y = 1.0 + row as f64 * spacing;
            if y > side - 1.0 {
                break 'fill;
            }
            sim.spawn_to_nearest_exit(person(1.0 + col as f64 * spacing, y, RIMEA_SPEED));
            placed += 1;
        }
    }
    assert_eq!(placed, n, "the room was too small to hold {n} agents");
}

/// **TC8, structural half.** A hundred people leave a 10 × 10 m room through a
/// single 1 m door: everyone gets out, nobody is lost, nobody is squeezed
/// through a wall.
///
/// Separate from the timing check below because these properties must hold
/// whatever the force constants are, so this one gates every PR.
#[test]
fn tc8_a_room_with_one_exit_empties_completely() {
    let (_pts, m, exits) = room_with_one_door(10.0, 1.0);
    let mut sim = Sim::new(m, exits, SimParams::default(), 20260803);
    fill_room(&mut sim, 100, 10.0, 0.8);

    // 400 s. The hydraulic estimate is ~80 s; if this is exhausted the room is
    // jammed, not merely slow.
    let h = run_watched(&mut sim, 8000);
    println!(
        "TC8: 100 agents through a 1.0 m door, cleared at {:?} s (worst overlap {:.4} m)",
        h.cleared_at, h.max_overlap
    );
    assert!(
        h.cleared_at.is_some(),
        "TC8: {} agents were still in the room after 400 s — the doorway jammed",
        sim.stats().active
    );
    assert_sound(&h, "TC8");
}

/// **TC8, quantitative half.** Total egress time against the hydraulic
/// calculation an engineer would do by hand.
///
/// Reference: 100 people through a 1.0 m door at the Green Guide's 82
/// persons/m/min is 73.2 s of discharge, plus roughly one room-diagonal of
/// travel for the first person, ≈ 80 s.
///
/// **Currently failing.** Measured 178.2 s against the ≈80 s reference, i.e.
/// **+123%**. That is consistent with `cf_sim::calibration`'s doorway
/// measurement of 41.7 persons/m/min — about half the Green Guide rate — and
/// it is the *safe* direction of error: the model over-predicts egress time.
/// It is still wrong, and a factor of two on the headline number a dossier
/// reports is not a tolerable error.
#[test]
fn tc8_egress_time_matches_the_hydraulic_calculation() {
    let (_pts, m, exits) = room_with_one_door(10.0, 1.0);
    let mut sim = Sim::new(m, exits, SimParams::default(), 20260803);
    fill_room(&mut sim, 100, 10.0, 0.8);

    let h = run_watched(&mut sim, 8000);
    let measured = h.cleared_at.expect("the room never emptied");

    // Green Guide rate of passage on the level, persons per metre per minute.
    let discharge = 100.0 / (cf_sim::calibration::GREEN_GUIDE_LEVEL_PPMM / 60.0 * 1.0);
    let travel = (10.0f64 * 10.0 + 10.0 * 10.0).sqrt() / RIMEA_SPEED as f64;
    let reference = discharge + travel;

    println!(
        "TC8: measured {measured:.1} s vs hydraulic reference {reference:.1} s \
         ({discharge:.1} s discharge + {travel:.1} s travel), {:+.0}%",
        (measured / reference - 1.0) * 100.0
    );

    let err = measured / reference - 1.0;
    assert!(
        err.abs() <= 0.30,
        "egress took {measured:.1} s against a {reference:.1} s hydraulic \
         reference ({:+.0}%); ±30% is the band a hand calculation is worth",
        err * 100.0
    );
}

// ---------------------------------------------------------------------------
// TC11 — route choice
// ---------------------------------------------------------------------------

/// A hall split by a partial internal wall, with one 2 m exit in each half's
/// south wall.
///
/// The point of the shape: an agent standing just east of the internal wall is
/// 2.5 m from the *west* exit in a straight line but must walk 15 m to reach
/// it, while its own exit is 6.5 m away and unobstructed. Straight-line
/// nearest and walk-nearest give opposite answers, which is exactly the
/// mistake that under-predicts egress time.
fn split_hall() -> (Vec<Vec2>, NavMesh, Vec<ExitSpan>) {
    let pts = vec![
        Vec2::new(0.0, 0.0),  // 0
        Vec2::new(7.0, 0.0),  // 1  west exit, left jamb
        Vec2::new(9.0, 0.0),  // 2  west exit, right jamb
        Vec2::new(10.0, 0.0), // 3  foot of the internal wall
        Vec2::new(16.0, 0.0), // 4  east exit, left jamb
        Vec2::new(18.0, 0.0), // 5  east exit, right jamb
        Vec2::new(20.0, 0.0), // 6
        Vec2::new(20.0, 10.0),
        Vec2::new(0.0, 10.0),
        Vec2::new(10.0, 8.0), // 9  head of the internal wall; 2 m gap above it
    ];
    let doors = [(1usize, 2usize), (4usize, 5usize)];
    let mut walls = ring_walls(9, &doors);
    walls.push((3, 9));

    let m = mesh(&pts, &walls, &doors);
    let exits = vec![door(&pts, 1, 2), door(&pts, 4, 5)];
    (pts, m, exits)
}

/// **TC11.** Agents choose their exit by walkable distance, not by straight
/// line.
///
/// `docs/06-validation.md` §3 lists exit selection under V2 functional
/// verification. What the engine supports today is nearest-by-path selection
/// at spawn time — [`Sim::spawn_to_nearest_exit`] — which is the property
/// checked here. Familiarity weighting and signage are not implemented.
#[test]
fn tc11_agents_choose_the_nearest_exit_by_walking_distance() {
    let (pts, m, exits) = split_hall();
    let west = pts[1].lerp(pts[2], 0.5);
    let east = pts[4].lerp(pts[5], 0.5);

    let mut sim = Sim::new(m, exits, SimParams::default(), 20260803);

    // The case that discriminates: hard against the east face of the internal
    // wall, where the west exit looks close and is not.
    let probe = Vec2::new(10.5, 0.5);
    assert!(
        probe.distance(west) < probe.distance(east),
        "the fixture is wrong: the west exit must be the straight-line nearest \
         from {probe:?} for this test to discriminate"
    );
    let chosen = sim.nearest_exit(probe).expect("both exits are reachable");
    assert!(
        chosen.distance(east) < 1e-9,
        "an agent at {probe:?} chose the exit at {chosen:?}; the east exit at \
         {east:?} is 6.5 m away by foot and the west exit at {west:?} is 15 m \
         away despite looking closer"
    );

    // A column down each face of the wall. Everyone should use the exit on
    // their own side.
    let mut expect_east = Vec::new();
    for i in 0..7 {
        let y = 0.5 + i as f64 * 1.0;
        sim.spawn_to_nearest_exit(person(10.6, y, RIMEA_SPEED));
        expect_east.push(true);
    }
    for i in 0..7 {
        let y = 0.5 + i as f64 * 1.0;
        sim.spawn_to_nearest_exit(person(9.4, y, RIMEA_SPEED));
        expect_east.push(false);
    }

    let h = run_watched(&mut sim, 4000);
    assert!(h.cleared_at.is_some(), "TC11: the hall never emptied");
    assert_sound(&h, "TC11");

    // Positions are frozen at despawn, so the last known x says which doorway
    // each agent left through.
    for (i, east_expected) in expect_east.iter().enumerate() {
        let x = sim.world.pos_x[i] as f64;
        let used_east = (x - east.x).abs() < (x - west.x).abs();
        assert_eq!(
            used_east,
            *east_expected,
            "agent {i} left at x = {x:.2}, i.e. through the {} exit",
            if used_east { "east" } else { "west" }
        );
    }
    println!("TC11: 14 agents all used the exit nearest by walking distance");
}

/// **TC11 (congestion form).** When the chosen exit saturates, some agents
/// should divert to an underused one.
///
/// **Currently failing, and it is a missing feature rather than a tuning
/// error.** [`Sim`] plans a route once in `Sim::spawn` and never re-plans:
/// `World::patience_left` is initialised to infinity and read nowhere, and
/// `Stream::RerouteChoice` is declared but never drawn. So the diverted
/// fraction is **0%** against the ≥10% asserted here, and it will stay 0%
/// until route re-evaluation lands (`docs/06-validation.md` §3, "Route choice
/// under congestion").
///
/// Left in rather than deleted because it is the acceptance test for that
/// feature, and because a suite that quietly omits what the engine cannot do
/// reads as a suite the engine passes.
#[test]
fn tc11_agents_divert_from_a_saturating_exit() {
    // A plain 20 x 10 hall with a 2 m exit at each end of the south wall.
    let pts = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(2.0, 0.0),
        Vec2::new(4.0, 0.0),
        Vec2::new(16.0, 0.0),
        Vec2::new(18.0, 0.0),
        Vec2::new(20.0, 0.0),
        Vec2::new(20.0, 10.0),
        Vec2::new(0.0, 10.0),
    ];
    let doors = [(1usize, 2usize), (3usize, 4usize)];
    let m = mesh(&pts, &ring_walls(8, &doors), &doors);
    let west = pts[1].lerp(pts[2], 0.5);
    let east = pts[3].lerp(pts[4], 0.5);
    let mut sim = Sim::new(m, exits_of(&pts, &doors), SimParams::default(), 20260803);

    // Everyone packed into the western third, so the west exit is the nearest
    // for all of them and will saturate badly.
    let mut n = 0;
    for row in 0..12 {
        for col in 0..12 {
            sim.spawn_to_nearest_exit(person(
                1.0 + col as f64 * 0.55,
                1.0 + row as f64 * 0.55,
                RIMEA_SPEED,
            ));
            n += 1;
        }
    }

    let h = run_watched(&mut sim, 12000);
    assert!(h.cleared_at.is_some(), "the hall never emptied");

    let via_east = (0..sim.world.len())
        .filter(|i| {
            let x = sim.world.pos_x[*i] as f64;
            (x - east.x).abs() < (x - west.x).abs()
        })
        .count();
    let frac = via_east as f64 / n as f64;
    println!(
        "TC11: {via_east}/{n} agents ({:.0}%) diverted to the far exit",
        frac * 100.0
    );

    assert!(
        frac >= 0.10,
        "only {:.0}% of {n} agents diverted to the underused exit; with the \
         near exit saturated a real crowd redistributes",
        frac * 100.0
    );
}

/// The exit spans for a set of doorway edges.
fn exits_of(pts: &[Vec2], doors: &[(usize, usize)]) -> Vec<ExitSpan> {
    doors.iter().map(|(a, b)| door(pts, *a, *b)).collect()
}

// ---------------------------------------------------------------------------
// TC12 — merging flows
// ---------------------------------------------------------------------------

/// **TC12.** Two streams merging into one corridor share it fairly and neither
/// branch is starved.
///
/// The geometry is a symmetric T: a 20 m cross-bar feeding a 2 m stem with the
/// only exit at its foot. Twenty people wait in each arm, mirrored, so any
/// persistent asymmetry in the outcome comes from the merge and not from the
/// setup.
///
/// The fairness criterion is deliberately loose — each branch must supply
/// between 25% and 75% of the first half of the exits. A merge that lets one
/// arm drain completely before the other starts is the failure mode worth
/// catching; a 55/45 split is not.
#[test]
fn tc12_two_streams_merge_without_starving_either_branch() {
    let pts = vec![
        Vec2::new(0.0, 6.0),
        Vec2::new(9.0, 6.0),
        Vec2::new(9.0, 0.0),
        Vec2::new(11.0, 0.0),
        Vec2::new(11.0, 6.0),
        Vec2::new(20.0, 6.0),
        Vec2::new(20.0, 8.0),
        Vec2::new(0.0, 8.0),
    ];
    let doors = [(2usize, 3usize)];
    let m = mesh(&pts, &ring_walls(8, &doors), &doors);
    let mut sim = Sim::new(m, exits_of(&pts, &doors), SimParams::default(), 20260803);

    // Twenty per arm, mirrored about the stem at x = 10.
    let per_arm = 20;
    for i in 0..per_arm {
        let back = (i / 2) as f64 * 0.75;
        let y = if i % 2 == 0 { 6.5 } else { 7.5 };
        sim.spawn_to_nearest_exit(person(8.5 - back, y, RIMEA_SPEED));
    }
    for i in 0..per_arm {
        let back = (i / 2) as f64 * 0.75;
        let y = if i % 2 == 0 { 6.5 } else { 7.5 };
        sim.spawn_to_nearest_exit(person(11.5 + back, y, RIMEA_SPEED));
    }

    let h = run_watched(&mut sim, 6000);
    assert!(
        h.cleared_at.is_some(),
        "TC12: {} agents never reached the exit — the merge deadlocked",
        sim.stats().active
    );
    assert_sound(&h, "TC12");

    // Order the exits by time and see which arm each came from. Agents
    // 0..per_arm are the west arm; the rest are the east arm.
    let times = exit_times(&sim, "TC12");
    let mut order: Vec<usize> = (0..times.len()).collect();
    order.sort_by(|a, b| {
        times[*a]
            .partial_cmp(&times[*b])
            .expect("no NaN exit times")
    });

    let half = order.len() / 2;
    let west_first_half = order[..half]
        .iter()
        .filter(|i| **i < per_arm as usize)
        .count();
    let share = west_first_half as f64 / half as f64;
    println!(
        "TC12: of the first {half} exits, {west_first_half} came from the west arm ({:.0}%); \
         cleared at {:?} s",
        share * 100.0,
        h.cleared_at
    );

    assert!(
        (0.25..=0.75).contains(&share),
        "the west arm supplied {:.0}% of the first half of the exits; one \
         branch is being starved at the merge",
        share * 100.0
    );
}
