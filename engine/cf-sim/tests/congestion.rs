//! Where a crowd loses time.
//!
//! A density map says where people were packed; this says where the building
//! cost them time, which is a different question and the one a planner acts on.

use cf_geom::Vec2;
use cf_navmesh::{classify, edge_key, triangulate_constrained, NavMesh};
use cf_sim::world::{AgentState, SpawnParams};
use cf_sim::{ExitSpan, Sim, SimParams};

/// A 12 x 10 hall with a doorway of the given width in the south wall.
fn room(door_w: f64) -> (NavMesh, Vec<ExitSpan>, Vec2) {
    let (w, h) = (12.0, 10.0);
    let (mid, half) = (w / 2.0, door_w / 2.0);
    let pts = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(mid - half, 0.0),
        Vec2::new(mid + half, 0.0),
        Vec2::new(w, 0.0),
        Vec2::new(w, h),
        Vec2::new(0.0, h),
    ];
    let ring: Vec<(usize, usize)> = (0..6).map(|i| (i, (i + 1) % 6)).collect();
    let mut tri = triangulate_constrained(&pts, &ring).expect("room triangulates");
    tri.compact();
    let regions = classify(&tri);
    tri.constraints.remove(&edge_key(1, 2));
    tri.rebuild_adjacency();
    let exits = vec![ExitSpan {
        a: pts[1],
        b: pts[2],
    }];
    (
        NavMesh::with_regions(tri, regions),
        exits,
        Vec2::new(mid, 0.0),
    )
}

fn crowd(door_w: f64, n: u32) -> (Sim, Vec2) {
    let (mesh, exits, door) = room(door_w);
    let mut sim = Sim::new(mesh, exits, SimParams::default(), 20260808);
    let mut placed = 0;
    'fill: for row in 0..30 {
        for col in 0..18 {
            if placed >= n {
                break 'fill;
            }
            sim.spawn_to_nearest_exit(SpawnParams {
                position: Vec2::new(1.0 + col as f64 * 0.55, 1.5 + row as f64 * 0.55),
                radius_m: 0.23,
                desired_speed: 1.34,
                goal: 0,
                population: 0,
                entry: 0,
                state: AgentState::Walking,
            });
            placed += 1;
        }
    }
    for _ in 0..8000 {
        sim.step();
        if sim.stats().active == 0 {
            break;
        }
    }
    (sim, door)
}

#[test]
fn the_worst_congestion_is_at_the_doorway() {
    // A hundred people through one metre. If the ranking cannot find that, it
    // will not find anything subtler either.
    let (sim, door) = crowd(1.0, 100);
    let spots = sim.hotspots(5);

    assert!(!spots.is_empty(), "no congestion recorded at all");
    let worst = spots[0];
    println!(
        "worst: ({:.1}, {:.1}) cost {:.0} person-s, {:.0}% of all delay",
        worst.x,
        worst.y,
        worst.lost_person_s,
        worst.share * 100.0
    );

    // Binned at 2 m, so the doorway cell centre can sit a bin away from the
    // opening itself. Anything further than that is not the doorway.
    assert!(
        (worst.x - door.x).abs() < 3.0 && worst.y < 4.0,
        "worst congestion at ({:.1}, {:.1}), nowhere near the door at ({:.1}, {:.1})",
        worst.x,
        worst.y,
        door.x,
        door.y
    );
}

#[test]
fn a_narrower_door_costs_the_crowd_more_time() {
    // The whole point of the measurement: it has to rank a worse building as
    // worse. The same crowd through half the opening must lose more time.
    let (wide, _) = crowd(2.0, 100);
    let (narrow, _) = crowd(1.0, 100);

    println!(
        "2.0 m door: {:.0} person-s lost; 1.0 m door: {:.0}",
        wide.lost_person_s(),
        narrow.lost_person_s()
    );
    assert!(
        narrow.lost_person_s() > wide.lost_person_s() * 1.2,
        "narrowing the door did not cost measurably more time: {:.0} vs {:.0}",
        narrow.lost_person_s(),
        wide.lost_person_s()
    );
}

#[test]
fn an_empty_run_has_no_hotspots() {
    // A table of places where nothing happened is worse than no table: a
    // reviewer reads it as a finding.
    let (mesh, exits, _) = room(1.0);
    let mut sim = Sim::new(mesh, exits, SimParams::default(), 1);
    for _ in 0..50 {
        sim.step();
    }
    assert!(sim.hotspots(5).is_empty());
    assert_eq!(sim.lost_person_s(), 0.0);
}

#[test]
fn a_lone_walker_loses_almost_nothing() {
    // One person in an empty hall is delayed only by their own acceleration.
    // If this reads as congestion, the measurement is charging the building for
    // the crowd model's own behaviour and every venue will look obstructed.
    let (mesh, exits, _) = room(2.0);
    let mut sim = Sim::new(mesh, exits, SimParams::default(), 2);
    sim.spawn_to_nearest_exit(SpawnParams {
        position: Vec2::new(6.0, 8.0),
        radius_m: 0.23,
        desired_speed: 1.34,
        goal: 0,
        population: 0,
        entry: 0,
        state: AgentState::Walking,
    });
    for _ in 0..4000 {
        sim.step();
        if sim.stats().active == 0 {
            break;
        }
    }
    println!("lone walker lost {:.2} person-s", sim.lost_person_s());
    assert!(
        sim.lost_person_s() < 3.0,
        "a single walker in an empty hall lost {:.1} person-s",
        sim.lost_person_s()
    );
}

#[test]
fn hotspots_come_back_worst_first_and_reproducibly() {
    let (a, _) = crowd(1.0, 80);
    let (b, _) = crowd(1.0, 80);

    let sa = a.hotspots(6);
    let sb = b.hotspots(6);
    assert_eq!(sa.len(), sb.len());

    for (x, y) in sa.iter().zip(sb.iter()) {
        assert_eq!(x, y, "the same run gave a different ranking");
    }
    for w in sa.windows(2) {
        assert!(
            w[0].lost_person_s >= w[1].lost_person_s,
            "hotspots are not ordered worst first"
        );
    }
    // Shares are a fraction of the whole and cannot exceed it.
    let total: f64 = sa.iter().map(|h| h.share).sum();
    assert!(total <= 1.0 + 1e-9, "shares sum to {total}");
}
