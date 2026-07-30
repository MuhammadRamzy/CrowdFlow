//! Physical plausibility of the density readout, and of the contacts beneath it.
//!
//! These exist because the workspace UI reported 26 p/m² during a 500-agent
//! evacuation. Maximum physical packing of 0.23 m bodies is 5.46 p/m², so that
//! number could not be true — and a tool whose headline safety figure can be
//! impossible is worse than one carrying no figure at all.

use cf_geom::{Aabb, Vec2};
use cf_navmesh::{triangulate_constrained, NavMesh};
use cf_sim::density::DensityGrid;
use cf_sim::world::{AgentState, SpawnParams, World};
use cf_sim::{ExitSpan, Sim, SimParams};

/// Hexagonal close packing of radius-`r` discs, in persons/m².
fn packing_limit(r: f64) -> f64 {
    1.0 / (2.0 * 3.0f64.sqrt() * r * r)
}

/// A 20 x 12 hall with two 1.8 m doorways in the south wall — the M1 fixture,
/// built directly so this crate needs no dependency on `cf-compile`.
fn hall() -> NavMesh {
    let pts: Vec<Vec2> = [
        (0.0, 0.0),
        (4.1, 0.0),
        (5.9, 0.0),
        (14.1, 0.0),
        (15.9, 0.0),
        (20.0, 0.0),
        (20.0, 12.0),
        (0.0, 12.0),
    ]
    .iter()
    .map(|(x, y)| Vec2::new(*x, *y))
    .collect();

    // Doorways sealed for classification, then reopened — the same dance
    // cf-compile does, and for the same reason.
    let sealed = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 4),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 0),
    ];
    let mut tri = triangulate_constrained(&pts, &sealed).unwrap();
    tri.compact();
    let regions = cf_navmesh::classify(&tri);
    for (a, b) in [(1usize, 2usize), (3, 4)] {
        tri.constraints.remove(&cf_navmesh::edge_key(a, b));
    }
    tri.rebuild_adjacency();
    NavMesh::with_regions(tri, regions)
}

#[test]
fn density_of_a_maximally_packed_lattice_is_physically_bounded() {
    let r = 0.23;
    let limit = packing_limit(r);

    let mut w = World::new();
    let dx = 2.0 * r;
    let dy = dx * 3.0f64.sqrt() / 2.0;
    for row in 0..30 {
        for col in 0..30 {
            let x = 5.0 + col as f64 * dx + if row % 2 == 1 { r } else { 0.0 };
            let y = 3.0 + row as f64 * dy;
            w.spawn(SpawnParams {
                position: Vec2::new(x, y),
                radius_m: r as f32,
                desired_speed: 1.34,
                goal: 0,
                population: 0,
                entry: 0,
                state: AgentState::Walking,
            });
        }
    }

    let mut g = DensityGrid::new(
        Aabb {
            min: Vec2::new(0.0, 0.0),
            max: Vec2::new(20.0, 12.0),
        },
        0.25,
    );
    g.accumulate(&w);
    let peak = g.max_current() as f64;

    // A window centred on a body counts that body plus its whole first ring, so
    // the peak reads above the lattice mean by construction — about 7/5.46, or
    // 1.3x. That bias is inherent to window counting; it is precisely why
    // Steffen & Seyfried use Voronoi density instead. Living with a bounded
    // bias is fine. Reading several times the physical limit is not.
    assert!(
        peak < limit * 1.5,
        "peak {peak:.2} p/m² exceeds 1.5x the {limit:.2} p/m² packing limit"
    );
    assert!(
        peak > limit * 0.8,
        "peak {peak:.2} p/m² is far below the {limit:.2} p/m² packing limit — \
         the kernel is over-smoothing and would hide a crush"
    );
}

/// The test that actually mattered: does the contact solve hold under a real
/// doorway jam, or do bodies pass through each other when the crowd presses?
#[test]
fn contacts_hold_under_a_doorway_jam() {
    let mesh = hall();
    let exits = vec![
        ExitSpan {
            a: Vec2::new(4.1, 0.0),
            b: Vec2::new(5.9, 0.0),
        },
        ExitSpan {
            a: Vec2::new(14.1, 0.0),
            b: Vec2::new(15.9, 0.0),
        },
    ];

    let mut sim = Sim::new(mesh, exits, SimParams::default(), 20260803);
    for i in 0..500 {
        let x = 1.0 + (i % 25) as f64 * 0.72;
        let y = 1.5 + (i / 25) as f64 * 0.5;
        sim.spawn_to_nearest_exit(SpawnParams {
            position: Vec2::new(x, y),
            radius_m: 0.23,
            desired_speed: 1.34,
            goal: 0,
            population: 0,
            entry: 0,
            state: AgentState::Walking,
        });
    }

    let mut worst_overlap = 0.0f32;
    let mut worst_density = 0.0f32;
    for _ in 0..3000 {
        let st = sim.step();
        worst_overlap = worst_overlap.max(st.max_overlap);
        worst_density = worst_density.max(sim.density().max_current());
        if st.active == 0 {
            break;
        }
    }

    println!("worst residual overlap = {worst_overlap:.4} m");
    println!("worst density          = {worst_density:.2} p/m²");

    // Bodies may touch. Interpenetrating by a large fraction of a radius means
    // the solve is not holding, and every density figure downstream is fiction.
    assert!(
        worst_overlap < 0.10,
        "bodies interpenetrated by {worst_overlap:.3} m — the contact solve is not holding"
    );

    // Even allowing the window-centring bias, nothing can plausibly exceed
    // roughly 1.5x the packing limit.
    let bound = packing_limit(0.23) * 1.5;
    assert!(
        (worst_density as f64) < bound,
        "density reached {worst_density:.2} p/m², above the {bound:.2} p/m² physical bound"
    );
}
