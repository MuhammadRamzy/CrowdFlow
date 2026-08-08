//! The run's timeline.
//!
//! A dossier full of totals says how a venue performed but not what happened.
//! A reviewer reconstructing an evacuation reads a sequence.

use cf_geom::Vec2;
use cf_navmesh::{classify, edge_key, triangulate_constrained, NavMesh};
use cf_sim::events::EventKind;
use cf_sim::world::{AgentState, SpawnParams};
use cf_sim::{ExitSpan, Sim, SimParams};

fn hall() -> (NavMesh, Vec<ExitSpan>) {
    let pts = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(2.0, 0.0),
        Vec2::new(4.0, 0.0),
        Vec2::new(16.0, 0.0),
        Vec2::new(18.0, 0.0),
        Vec2::new(20.0, 0.0),
        Vec2::new(20.0, 12.0),
        Vec2::new(0.0, 12.0),
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

fn person(x: f64, y: f64) -> SpawnParams {
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

fn crowd(n: u32) -> Sim {
    let (mesh, exits) = hall();
    let mut sim = Sim::new(mesh, exits, SimParams::default(), 20260808);
    for i in 0..n {
        sim.spawn_to_nearest_exit(person(
            1.5 + (i % 12) as f64 * 0.7,
            2.0 + (i / 12) as f64 * 0.7,
        ));
    }
    sim
}

fn run(sim: &mut Sim) {
    for _ in 0..12000 {
        sim.step();
        if sim.stats().active == 0 {
            break;
        }
    }
}

#[test]
fn the_log_records_the_milestones_in_order() {
    let mut sim = crowd(80);
    run(&mut sim);
    let log = sim.events();

    println!("timeline:");
    for e in log {
        println!("  {:6.1}s  {:?}", e.at_s, e.kind);
    }

    let first = log
        .iter()
        .position(|e| e.kind == EventKind::FirstDeparture)
        .expect("no first departure");
    let half = log
        .iter()
        .position(|e| e.kind == EventKind::HalfCleared)
        .expect("no halfway mark");
    let last = log
        .iter()
        .position(|e| e.kind == EventKind::LastDeparture)
        .expect("the venue never reported emptying");

    assert!(first < half && half < last, "milestones out of order");
    // Timestamps must be non-decreasing, or the timeline reads as nonsense.
    for w in log.windows(2) {
        assert!(w[0].at_s <= w[1].at_s, "the log went backwards in time");
    }
}

#[test]
fn each_milestone_fires_exactly_once() {
    // A log that repeats itself is one nobody reads to the end.
    let mut sim = crowd(60);
    run(&mut sim);
    for kind in [
        EventKind::FirstDeparture,
        EventKind::HalfCleared,
        EventKind::LastDeparture,
    ] {
        let n = sim.events().iter().filter(|e| e.kind == kind).count();
        assert_eq!(n, 1, "{kind:?} fired {n} times");
    }
}

#[test]
fn halfway_is_measured_against_who_will_actually_leave() {
    // Not against who was spawned. An agent that was never placed cannot
    // clear, and counting it would put the halfway mark somewhere nobody
    // crossed — so it would never fire at all.
    let mut sim = crowd(40);
    run(&mut sim);
    let half = sim
        .events()
        .iter()
        .find(|e| e.kind == EventKind::HalfCleared)
        .expect("halfway never fired");
    let last = sim
        .events()
        .iter()
        .find(|e| e.kind == EventKind::LastDeparture)
        .expect("no last departure");
    assert!(half.at_s < last.at_s);
}

#[test]
fn closing_a_door_appears_in_the_timeline() {
    let mut sim = crowd(80);
    for _ in 0..200 {
        sim.step();
    }
    assert!(sim.close_exit(0));
    run(&mut sim);

    let closed = sim
        .events()
        .iter()
        .find(|e| matches!(e.kind, EventKind::ExitClosed { .. }))
        .expect("a door shut and the log did not mention it");
    assert!(closed.at_s >= 10.0, "closed at {:.1}s", closed.at_s);
}

#[test]
fn the_alarm_records_how_many_it_moved() {
    let mut sim = crowd(50);
    for _ in 0..20 {
        sim.step();
    }
    let moved = sim.evacuate_all();
    assert!(moved > 0);

    let alarm = sim
        .events()
        .iter()
        .find(|e| matches!(e.kind, EventKind::AlarmSounded { .. }))
        .expect("the alarm sounded and the log did not mention it");
    match alarm.kind {
        EventKind::AlarmSounded { rerouted } => assert_eq!(rerouted, moved),
        _ => unreachable!(),
    }
}

#[test]
fn a_density_band_is_announced_once_and_only_upward() {
    // A crowd hovering at a boundary would otherwise announce the same
    // crossing every few ticks.
    let mut sim = crowd(200);
    run(&mut sim);

    let bands: Vec<u32> = sim
        .events()
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::DensityThreshold { tenths_per_m2 } => Some(tenths_per_m2),
            _ => None,
        })
        .collect();

    println!("density bands crossed: {bands:?}");
    let mut seen = std::collections::BTreeSet::new();
    for b in &bands {
        assert!(seen.insert(*b), "band {b} announced twice");
    }
    // Strictly increasing: a band is only ever crossed upward.
    for w in bands.windows(2) {
        assert!(w[0] < w[1], "bands announced out of order: {bands:?}");
    }
}

#[test]
fn an_empty_run_has_an_empty_log() {
    let (mesh, exits) = hall();
    let mut sim = Sim::new(mesh, exits, SimParams::default(), 1);
    for _ in 0..100 {
        sim.step();
    }
    assert!(sim.events().is_empty(), "{:?}", sim.events());
}

#[test]
fn the_same_seed_gives_the_same_timeline() {
    let mut a = crowd(60);
    let mut b = crowd(60);
    run(&mut a);
    run(&mut b);
    assert_eq!(a.events(), b.events(), "the timeline is not reproducible");
}
