//! How many agents this engine actually steps in a frame.
//!
//! `docs/00-overview.md` §hard-numbers commits to **25k agents at 60 fps in the
//! browser**. Nothing had ever measured it, so the number was an intention
//! rather than a claim. This is the instrument that turns it into one.
//!
//! Timing is machine-dependent, so these are `#[ignore]`d and print rather than
//! assert — the same treatment as `calibration::sweep_agent_repulsion`. A test
//! that fails on a slow laptop teaches people to ignore the suite.
//!
//! ```text
//! cargo test -p cf-sim --release --test scale -- --ignored --nocapture
//! ```
//!
//! **Run it in `--release`.** A debug build is roughly an order of magnitude
//! slower and would send anyone reading the output off optimising noise.

use cf_geom::Vec2;
use cf_navmesh::{classify, edge_key, triangulate_constrained, NavMesh};
use cf_sim::world::{AgentState, SpawnParams};
use cf_sim::{ExitSpan, Sim, SimParams};
use std::time::Instant;

/// A hall `w` x `h` with a doorway at each end of the south wall.
fn hall(w: f64, h: f64) -> (NavMesh, Vec<ExitSpan>) {
    let pts = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(2.0, 0.0),
        Vec2::new(4.0, 0.0),
        Vec2::new(w - 4.0, 0.0),
        Vec2::new(w - 2.0, 0.0),
        Vec2::new(w, 0.0),
        Vec2::new(w, h),
        Vec2::new(0.0, h),
    ];
    let doors = [(1usize, 2usize), (3usize, 4usize)];
    let ring: Vec<(usize, usize)> = (0..8).map(|i| (i, (i + 1) % 8)).collect();

    let mut tri = triangulate_constrained(&pts, &ring).expect("hall triangulates");
    tri.compact();
    let regions = classify(&tri);
    for (a, b) in doors {
        tri.constraints.remove(&edge_key(a, b));
    }
    tri.rebuild_adjacency();

    let exits = doors
        .iter()
        .map(|(a, b)| ExitSpan {
            a: pts[*a],
            b: pts[*b],
        })
        .collect();
    (NavMesh::with_regions(tri, regions), exits)
}

/// Fill a hall with `n` agents on a lattice and time `ticks` steps.
///
/// Spawning is excluded from the timing: it runs a path query per agent, which
/// is a one-off cost at the start of a run and would otherwise dominate a short
/// measurement.
fn measure(n: u32, params: SimParams) -> (f64, u32) {
    // Room enough for `n` bodies at a comfortable 1.4 persons/m².
    let area = n as f64 / 1.4;
    let w = (area * 2.0).sqrt().max(24.0);
    let h = (area / w).max(12.0);

    let (mesh, exits) = hall(w, h);
    let mut sim = Sim::new(mesh, exits, params, 20260807);

    let spacing = (1.0f64 / 1.4).sqrt();
    let mut placed = 0;
    let mut y = 1.5;
    'fill: while y < h - 1.0 {
        let mut x = 1.5;
        while x < w - 1.5 {
            if placed >= n {
                break 'fill;
            }
            sim.spawn_to_nearest_exit(SpawnParams {
                position: Vec2::new(x, y),
                radius_m: 0.23,
                desired_speed: 1.34,
                goal: 0,
                population: 0,
                entry: 0,
                state: AgentState::Walking,
            });
            placed += 1;
            x += spacing;
        }
        y += spacing;
    }

    // A few steps first so the spatial grid and route caches are warm.
    for _ in 0..5 {
        sim.step();
    }

    let ticks = 40;
    let t0 = Instant::now();
    for _ in 0..ticks {
        sim.step();
    }
    let per_step_ms = t0.elapsed().as_secs_f64() * 1000.0 / ticks as f64;
    (per_step_ms, placed)
}

/// Step cost against agent count, against the budget that actually applies.
///
/// # The budget is the tick, not the frame
///
/// It is tempting to measure a step against a 16.7 ms frame. That is the wrong
/// budget and flatters nothing — it *under*states the engine by a factor of
/// three. The physics runs at 20 Hz and rendering interpolates between ticks
/// rather than driving them (`sim.rs` module docs), so a step has the whole
/// **50 ms tick interval** to complete in order to keep up with real time. At
/// 60 fps that is one step every three frames.
///
/// So the number that matters is the realtime factor: steps per second divided
/// by the 20 per second a run needs.
#[test]
#[ignore = "measurement, not an assertion — run with --release"]
fn how_many_agents_step_in_real_time() {
    // dt = 0.05 s, so 20 steps buy one second of simulated time.
    const TICK_MS: f64 = 50.0;

    println!(
        "{:>8} {:>10} {:>10} {:>12}",
        "agents", "ms/step", "µs/agent", "x realtime"
    );
    for n in [500u32, 2_000, 5_000, 10_000, 25_000] {
        let (ms, placed) = measure(n, SimParams::default());
        println!(
            "{placed:>8} {ms:>10.2} {:>10.2} {:>11.2}x",
            ms * 1000.0 / placed as f64,
            TICK_MS / ms
        );
    }
    println!(
        "\ntarget: 25,000 agents at real time or better (docs/00-overview.md).\n\
         Native release build; wasm typically runs 1.5-2x slower, so halve these."
    );
}

/// How much of a step is routing.
///
/// `reconsider_exits` issues a path query per exit per reconsideration, and
/// `egressDistribution` now runs a whole scenario ten times. Flow fields are
/// the planned replacement (phase B4) — this says whether they are urgent or
/// merely eventual, which is not a question to answer from intuition.
#[test]
#[ignore = "measurement, not an assertion — run with --release"]
fn what_does_rerouting_cost() {
    for n in [2_000u32, 10_000] {
        let (with, placed) = measure(n, SimParams::default());
        let (without, _) = measure(
            n,
            SimParams {
                reroute_interval_s: 0.0,
                ..SimParams::default()
            },
        );
        let share = (with - without) / with * 100.0;
        println!(
            "{placed:>7} agents: {with:>7.2} ms/step with rerouting, \
             {without:>7.2} without — {share:>5.1}% of the step"
        );
    }
}
