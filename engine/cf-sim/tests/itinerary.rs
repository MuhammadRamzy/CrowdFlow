//! Multi-leg itineraries: arrive, do something, then leave.
//!
//! A single goal can only express the last of those, and a venue where nobody
//! is doing anything before the alarm is not the venue anyone is analysing.

use cf_geom::Vec2;
use cf_navmesh::{classify, edge_key, triangulate_constrained, NavMesh};
use cf_sim::sim::Leg;
use cf_sim::world::{AgentState, SpawnParams};
use cf_sim::{ExitSpan, Sim, SimParams};

fn hall() -> (NavMesh, Vec<ExitSpan>) {
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
    tri.constraints.remove(&edge_key(1, 2));
    tri.rebuild_adjacency();
    let exits = vec![ExitSpan {
        a: pts[1],
        b: pts[2],
    }];
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

#[test]
fn an_agent_visits_each_leg_in_turn() {
    let (mesh, exits) = hall();
    let mut sim = Sim::new(mesh, exits, SimParams::default(), 20260807);

    let bar = Vec2::new(16.0, 9.0);
    let seat = Vec2::new(4.0, 9.0);
    let door = Vec2::new(9.0, 0.0);

    sim.spawn_with_itinerary(
        person(2.0, 2.0),
        &[
            Leg {
                goal: bar,
                dwell_s: 0.0,
                to_exit: false,
            },
            Leg {
                goal: seat,
                dwell_s: 0.0,
                to_exit: false,
            },
            Leg {
                goal: door,
                dwell_s: 0.0,
                to_exit: true,
            },
        ],
    );

    let (mut saw_bar, mut saw_seat) = (false, false);
    for _ in 0..4000 {
        sim.step();
        if sim.stats().active == 0 {
            break;
        }
        let p = Vec2::new(sim.world.pos_x[0] as f64, sim.world.pos_y[0] as f64);
        if p.distance(bar) < 1.0 {
            saw_bar = true;
        }
        // Order matters: reaching the seat before the bar would mean the legs
        // were taken out of sequence, which a straight line from spawn to exit
        // would also satisfy if the chain were being ignored.
        if saw_bar && p.distance(seat) < 1.0 {
            saw_seat = true;
        }
    }

    assert!(saw_bar, "never visited the first leg");
    assert!(saw_seat, "never visited the second leg after the first");
    assert_eq!(sim.stats().active, 0, "never left");
    assert_eq!(sim.stats().exited, 1);
}

#[test]
fn a_dwell_holds_an_agent_where_it_arrived() {
    let (mesh, exits) = hall();
    let mut sim = Sim::new(mesh, exits, SimParams::default(), 1);

    let seat = Vec2::new(16.0, 9.0);
    sim.spawn_with_itinerary(
        person(2.0, 2.0),
        &[
            Leg {
                goal: seat,
                dwell_s: 30.0,
                to_exit: false,
            },
            Leg {
                goal: Vec2::new(9.0, 0.0),
                dwell_s: 0.0,
                to_exit: true,
            },
        ],
    );

    let mut dwell_ticks = 0;
    let mut arrived_at = None;
    for tick in 0..6000 {
        sim.step();
        if sim.stats().active == 0 {
            break;
        }
        if sim.world.state[0] == AgentState::Dwelling {
            dwell_ticks += 1;
            arrived_at.get_or_insert(tick);
        }
    }

    let held = dwell_ticks as f64 * SimParams::default().dt;
    println!("dwelt {held:.1} s against a declared 30 s");
    assert!(
        (held - 30.0).abs() < 1.0,
        "dwelt {held:.1} s against a declared 30 s"
    );
    assert_eq!(sim.stats().exited, 1, "never moved on after dwelling");
}

#[test]
fn a_crowd_jostling_at_a_goal_does_not_restart_its_wait() {
    // Arriving is tested by distance, and a crowd milling around a goal crosses
    // that threshold repeatedly. Restarting the wait each time would hold a
    // busy venue there forever.
    let (mesh, exits) = hall();
    let mut sim = Sim::new(mesh, exits, SimParams::default(), 7);

    let seat = Vec2::new(16.0, 9.0);
    for i in 0..25 {
        sim.spawn_with_itinerary(
            person(2.0 + (i % 5) as f64 * 0.6, 2.0 + (i / 5) as f64 * 0.6),
            &[
                Leg {
                    goal: seat,
                    dwell_s: 10.0,
                    to_exit: false,
                },
                Leg {
                    goal: Vec2::new(9.0, 0.0),
                    dwell_s: 0.0,
                    to_exit: true,
                },
            ],
        );
    }

    for _ in 0..8000 {
        sim.step();
        if sim.stats().active == 0 {
            break;
        }
    }
    assert_eq!(sim.stats().exited, 25, "{} never left", sim.stats().active);
}

#[test]
fn an_itinerary_survives_a_replan() {
    // `retarget` rebuilds the route. Losing the chain there would silently end
    // an agent's plans the first time it got stuck — which is the sort of thing
    // that shows up as "some agents leave early" and takes a day to find.
    let (mesh, exits) = hall();
    let mut sim = Sim::new(mesh, exits, SimParams::default(), 3);

    let bar = Vec2::new(16.0, 9.0);
    sim.spawn_with_itinerary(
        person(2.0, 2.0),
        &[
            Leg {
                goal: bar,
                dwell_s: 0.0,
                to_exit: false,
            },
            Leg {
                goal: Vec2::new(9.0, 0.0),
                dwell_s: 0.0,
                to_exit: true,
            },
        ],
    );

    // Force a replan before the first leg is reached.
    sim.step();
    sim.retarget(0, bar, false);

    let mut saw_bar = false;
    for _ in 0..4000 {
        sim.step();
        if sim.stats().active == 0 {
            break;
        }
        let p = Vec2::new(sim.world.pos_x[0] as f64, sim.world.pos_y[0] as f64);
        if p.distance(bar) < 1.0 {
            saw_bar = true;
        }
    }
    assert!(saw_bar);
    assert_eq!(sim.stats().exited, 1, "the itinerary was lost on replan");
}

#[test]
fn no_itinerary_behaves_exactly_as_before() {
    let (mesh, exits) = hall();
    let mut sim = Sim::new(mesh, exits, SimParams::default(), 11);
    for i in 0..30 {
        sim.spawn_to_nearest_exit(person(
            2.0 + (i % 6) as f64 * 0.6,
            6.0 + (i / 6) as f64 * 0.6,
        ));
    }
    for _ in 0..6000 {
        sim.step();
        if sim.stats().active == 0 {
            break;
        }
    }
    assert_eq!(sim.stats().exited, 30);
}
