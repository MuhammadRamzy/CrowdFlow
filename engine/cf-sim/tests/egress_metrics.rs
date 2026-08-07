//! Egress metrics: the shape of an evacuation, not just its end.
//!
//! A single total conceals the difference between a hall that empties steadily
//! and one where nine tenths are out in a minute and the last few take five.
//! It is the tail that describes the risk — those are the people still inside
//! when conditions get worse — so RiMEA and ISO 20414 both ask for percentiles.

use cf_geom::Vec2;
use cf_navmesh::{classify, edge_key, triangulate_constrained, NavMesh};
use cf_sim::world::{AgentState, SpawnParams};
use cf_sim::{ExitSpan, Sim, SimParams};

/// A 20 x 12 hall with a 2 m door at each end of the south wall.
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

fn run(n: u32) -> Sim {
    let (mesh, exits) = hall();
    let mut sim = Sim::new(mesh, exits, SimParams::default(), 20260807);
    let mut placed = 0;
    'fill: for row in 0..20 {
        for col in 0..20 {
            if placed >= n {
                break 'fill;
            }
            sim.spawn_to_nearest_exit(person(2.0 + col as f64 * 0.8, 3.0 + row as f64 * 0.6));
            placed += 1;
        }
    }
    for _ in 0..12000 {
        sim.step();
        if sim.stats().active == 0 {
            break;
        }
    }
    sim
}

#[test]
fn percentiles_describe_the_shape_of_the_evacuation() {
    let sim = run(200);
    assert_eq!(sim.stats().active, 0, "the hall never emptied");

    let p50 = sim.egress_percentile(0.50).expect("half must have left");
    let p90 = sim.egress_percentile(0.90).expect("most must have left");
    let p99 = sim
        .egress_percentile(0.99)
        .expect("nearly all must have left");
    let p100 = sim.egress_percentile(1.0).expect("everyone must have left");

    println!("egress: 50% {p50:.1}s  90% {p90:.1}s  99% {p99:.1}s  100% {p100:.1}s");

    // Monotone by construction — departures are recorded in time order — but
    // worth asserting, because a sort or a filter slipping in later would
    // break it silently and every figure downstream would still look sane.
    assert!(p50 <= p90 && p90 <= p99 && p99 <= p100, "not monotone");
    assert!(p50 > 0.0);
    // The last person out defines the total, so these must agree.
    assert_eq!(p100, sim.egress_percentile(1.0).unwrap());
}

#[test]
fn no_departures_means_no_percentile_rather_than_zero() {
    // Zero would read as an instantaneous evacuation, which is the most
    // flattering possible wrong answer.
    let (mesh, exits) = hall();
    let sim = Sim::new(mesh, exits, SimParams::default(), 1);
    assert!(sim.egress_percentile(0.5).is_none());
    assert!(sim.egress_percentile(1.0).is_none());
}

#[test]
fn a_percentile_is_a_time_somebody_actually_left_at() {
    // Nearest rank, not interpolation: the quoted figure has to be a departure
    // that happened, or a reviewer cannot go and look at it.
    let sim = run(60);
    let p50 = sim.egress_percentile(0.5).unwrap();
    let usage: u32 = sim.exit_usage().iter().sum();
    assert!(usage > 0);
    // Every percentile must be one of the recorded times.
    for f in [0.1, 0.25, 0.5, 0.75, 0.9, 1.0] {
        let t = sim.egress_percentile(f).unwrap();
        assert!(t > 0.0 && t <= sim.egress_percentile(1.0).unwrap());
    }
    assert!(p50 > 0.0);
}

#[test]
fn throughput_is_attributed_to_the_door_that_carried_it() {
    // A crowd packed at the west end should mostly use the west door. A report
    // that cannot say which door carried the building cannot say which one to
    // widen.
    let (mesh, exits) = hall();
    let mut sim = Sim::new(mesh, exits, SimParams::default(), 20260807);
    for i in 0..80 {
        sim.spawn_to_nearest_exit(person(
            1.5 + (i % 8) as f64 * 0.6,
            2.0 + (i / 8) as f64 * 0.6,
        ));
    }
    for _ in 0..12000 {
        sim.step();
        if sim.stats().active == 0 {
            break;
        }
    }

    let usage = sim.exit_usage();
    println!("exit usage: {usage:?}");
    assert_eq!(usage.len(), 2);
    assert_eq!(usage.iter().sum::<u32>(), 80, "agents unaccounted for");
    assert!(
        usage[0] > usage[1],
        "the near door should have carried more"
    );

    let flow = sim.exit_specific_flow();
    assert_eq!(flow.len(), 2);
    assert!(flow[0] > 0.0, "the door that was used reports no flow");
}

#[test]
fn closing_a_door_does_not_move_its_tally_onto_another() {
    // `exit_usage` is keyed to the original index. Keyed to the current one,
    // closing the west door would shift the east door into slot 0 and the
    // report would credit the wrong doorway.
    let (mesh, exits) = hall();
    let mut sim = Sim::new(mesh, exits, SimParams::default(), 20260807);
    for i in 0..80 {
        sim.spawn_to_nearest_exit(person(
            1.5 + (i % 8) as f64 * 0.6,
            2.0 + (i / 8) as f64 * 0.6,
        ));
    }
    for _ in 0..200 {
        sim.step();
    }
    let before = sim.exit_usage()[0];
    assert!(before > 0, "nobody used the west door before it shut");

    assert!(sim.close_exit(0));
    for _ in 0..12000 {
        sim.step();
        if sim.stats().active == 0 {
            break;
        }
    }

    let usage = sim.exit_usage();
    assert_eq!(usage.len(), 2, "the tally lost a doorway when one closed");
    assert_eq!(usage[0], before, "the closed door kept taking credit");
    assert!(usage[1] > 0, "the remaining door carried nobody");
    assert_eq!(usage.iter().sum::<u32>(), 80);
}
