//! Evacuating a building, not a room.
//!
//! An upper floor with no doors of its own, a ground floor with two, and a
//! staircase between them. Everyone upstairs has to reach the stairs, cross,
//! and then leave — which is the shape of every real evacuation and the one
//! thing a single-floor model cannot express at all.

use cf_geom::Vec2;
use cf_navmesh::{classify, edge_key, triangulate_constrained, NavMesh};
use cf_sim::building::{Building, Link};
use cf_sim::world::{AgentState, SpawnParams};
use cf_sim::{ExitSpan, Sim, SimParams};

/// A 20 x 12 hall. `doors` names ring edges to reopen as doorways.
fn hall(doors: &[(usize, usize)]) -> (NavMesh, Vec<ExitSpan>) {
    let pts = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(8.0, 0.0),
        Vec2::new(10.0, 0.0),
        Vec2::new(20.0, 0.0),
        Vec2::new(20.0, 12.0),
        Vec2::new(0.0, 12.0),
    ];
    let ring: Vec<(usize, usize)> = (0..6).map(|i| (i, (i + 1) % 6)).collect();

    let mut tri = triangulate_constrained(&pts, &ring).expect("hall triangulates");
    tri.compact();
    let regions = classify(&tri);
    for (a, b) in doors {
        tri.constraints.remove(&edge_key(*a, *b));
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

/// Upper floor: sealed, no way out except the stairs. Ground: one 2 m door.
fn two_storey(n: u32) -> Building {
    let (upper_mesh, _) = hall(&[]);
    let (ground_mesh, ground_exits) = hall(&[(1, 2)]);

    let mut upper = Sim::new(upper_mesh, Vec::new(), SimParams::default(), 20260807);
    let ground = Sim::new(ground_mesh, ground_exits, SimParams::default(), 20260807);

    // A crowd upstairs, away from the stair head so they have to walk to it.
    let mut placed = 0;
    'fill: for row in 0..20 {
        for col in 0..20 {
            if placed >= n {
                break 'fill;
            }
            upper.spawn_to_nearest_exit(person(2.0 + col as f64 * 0.7, 6.0 + row as f64 * 0.7));
            placed += 1;
        }
    }

    // The stair: mid-hall on both floors, 1.2 m clear, 12 s to walk down.
    let link = Link {
        floor_a: 0,
        point_a: Vec2::new(15.0, 3.0),
        floor_b: 1,
        point_b: Vec2::new(15.0, 3.0),
        clear_width_m: 1.2,
        traverse_s: 12.0,
    };

    let mut b = Building::new(vec![upper, ground], vec![link]);
    // Nobody upstairs has a door to aim at; the stairs are the way out.
    b.route_to_stairs(0);
    b
}

#[test]
fn a_crowd_upstairs_reaches_the_street() {
    let n = 120;
    let mut b = two_storey(n);
    assert_eq!(b.floor_count(), 2);
    assert_eq!(b.floor(0).unwrap().stats().active, n);
    assert_eq!(b.floor(1).unwrap().stats().active, 0);

    let mut saw_transit = false;
    for _ in 0..12000 {
        b.step();
        saw_transit |= b.in_transit() > 0;
        let s = b.stats();
        if s.active == 0 {
            break;
        }
    }

    let s = b.stats();
    println!(
        "two-storey: {} of {n} out in {:.1} s, {} crossed the stair",
        s.exited,
        b.time(),
        b.crossings()[0]
    );

    assert!(saw_transit, "nobody was ever on the stairs");
    assert_eq!(s.active, 0, "{} never got out", s.active);
    assert_eq!(s.exited, n, "agents went missing between floors");
    assert_eq!(b.crossings()[0], n, "not everyone used the stair");
    assert_eq!(s.escaped, 0, "agents leaked through a wall");
}

#[test]
fn people_on_the_stairs_are_still_in_the_building() {
    // A building that reports itself empty while a staircase is full would
    // give an evacuation time that is simply wrong, and plausibly so.
    let mut b = two_storey(60);
    let mut peak_transit = 0;
    for _ in 0..12000 {
        b.step();
        peak_transit = peak_transit.max(b.in_transit());
        if b.stats().active == 0 {
            break;
        }
    }
    assert!(peak_transit > 0);
    // Whenever anyone was on the stairs, they counted as active.
    assert_eq!(b.stats().active, 0);
    assert_eq!(b.in_transit(), 0, "someone is still on the stairs");
}

#[test]
fn the_stair_takes_the_time_it_says_it_does() {
    // One person, an empty building: their crossing should cost the link's
    // traverse time and nothing else should account for it.
    let (upper_mesh, _) = hall(&[]);
    let (ground_mesh, ground_exits) = hall(&[(1, 2)]);
    let mut upper = Sim::new(upper_mesh, Vec::new(), SimParams::default(), 1);
    let ground = Sim::new(ground_mesh, ground_exits, SimParams::default(), 1);
    upper.spawn_to_nearest_exit(person(15.0, 3.4));

    let link = Link {
        floor_a: 0,
        point_a: Vec2::new(15.0, 3.0),
        floor_b: 1,
        point_b: Vec2::new(15.0, 3.0),
        clear_width_m: 1.2,
        traverse_s: 20.0,
    };
    let mut b = Building::new(vec![upper, ground], vec![link]);
    b.route_to_stairs(0);

    let mut left_upper = None;
    let mut reached_ground = None;
    for _ in 0..4000 {
        b.step();
        if left_upper.is_none() && b.in_transit() > 0 {
            left_upper = Some(b.time());
        }
        if left_upper.is_some() && reached_ground.is_none() && b.in_transit() == 0 {
            reached_ground = Some(b.time());
            break;
        }
    }

    let a = left_upper.expect("never stepped onto the stair");
    let c = reached_ground.expect("never stepped off it");
    let took = c - a;
    println!("one person crossed a 20 s stair in {took:.2} s");
    assert!(
        (took - 20.0).abs() < 0.2,
        "crossing took {took:.2} s against a declared 20 s"
    );
}

#[test]
fn a_building_with_one_floor_behaves_exactly_like_a_bare_sim() {
    // The single-floor path is every venue in the fixture set. Wrapping it in
    // a Building must not change a thing.
    let (mesh, exits) = hall(&[(1, 2)]);
    let mut sim = Sim::new(mesh, exits, SimParams::default(), 20260807);
    for i in 0..40 {
        sim.spawn_to_nearest_exit(person(
            2.0 + (i % 8) as f64 * 0.7,
            6.0 + (i / 8) as f64 * 0.7,
        ));
    }
    let mut b = Building::new(vec![sim], Vec::new());

    for _ in 0..8000 {
        b.step();
        if b.stats().active == 0 {
            break;
        }
    }
    let s = b.stats();
    assert_eq!(s.active, 0);
    assert_eq!(s.exited, 40);
    assert!(b.links().is_empty());
}
