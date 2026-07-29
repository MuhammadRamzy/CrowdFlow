//! Acceptance test for M1: compile the shared fixture and walk out of it.
//!
//! `fixtures/unit/hall-two-doors.venue.json` is the venue both tracks target
//! for milestone M1 (`docs/05-roadmap-and-risks.md`). If this passes, the
//! schema, the geometry layer, the navmesh and the compiler all agree.

use cf_compile::{compile, CompileWarning};
use cf_geom::Vec2;
use cf_navmesh::path_length;
use cf_schema::VenueDoc;
use std::path::PathBuf;

fn load_fixture() -> VenueDoc {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/unit/hall-two-doors.venue.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("fixture parses")
}

#[test]
fn the_fixture_compiles_cleanly() {
    let doc = load_fixture();
    let graph = compile(&doc);

    assert!(
        graph.is_simulable(),
        "fatal warnings: {:#?}",
        graph.fatal_warnings().collect::<Vec<_>>()
    );
    assert_eq!(graph.floors.len(), 1);
    assert_eq!(graph.compiler_version, cf_compile::COMPILER_VERSION);
}

/// The headline number: a 20 x 12 hall is 240 m² of floor.
///
/// Walls are centrelines, so the walkable region is bounded by them exactly.
/// Getting this wrong by the wall thickness would be a silent 4% error in every
/// occupant-load calculation downstream.
#[test]
fn walkable_area_is_the_full_hall() {
    let doc = load_fixture();
    let graph = compile(&doc);
    let floor = &graph.floors[0];

    assert!(
        (floor.walkable_area() - 240.0).abs() < 1e-9,
        "walkable area {} should be 240 m²",
        floor.walkable_area()
    );
}

/// Doorways are gaps in the south wall. Region classification must still see a
/// closed outline — this is the door-sealing step, and it is the single thing
/// most likely to regress here.
#[test]
fn doorways_do_not_leak_the_interior() {
    let doc = load_fixture();
    let graph = compile(&doc);

    assert!(
        !graph
            .warnings
            .iter()
            .any(|w| matches!(w, CompileWarning::NoWalkableArea { .. })),
        "the exterior fill leaked through a doorway: {:#?}",
        graph.warnings
    );
    assert!(
        !graph
            .warnings
            .iter()
            .any(|w| matches!(w, CompileWarning::UnclosedOutline { .. })),
        "outline reported as unclosed: {:#?}",
        graph.warnings
    );
}

#[test]
fn both_doors_are_found_and_bound_to_the_floor() {
    let doc = load_fixture();
    let graph = compile(&doc);
    let floor = &graph.floors[0];

    assert_eq!(floor.doors.len(), 2, "{:#?}", floor.doors);

    for door in &floor.doors {
        assert!(
            (door.width_m - 1.8).abs() < 1e-9,
            "door {} is {} m wide",
            door.opening,
            door.width_m
        );
        assert!(
            door.inside.is_some(),
            "door {} is not bound to a walkable triangle",
            door.opening
        );
        assert!(door.is_fire_exit, "both fixture doors are fire exits");
    }

    // Both sit on the south wall, at x = 15 and x = 5.
    let mut centres: Vec<f64> = floor.doors.iter().map(|d| d.midpoint().x).collect();
    centres.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!((centres[0] - 5.0).abs() < 1e-9, "centres {centres:?}");
    assert!((centres[1] - 15.0).abs() < 1e-9, "centres {centres:?}");
    for d in &floor.doors {
        assert!(d.midpoint().y.abs() < 1e-9, "doors sit on y = 0");
    }

    assert_eq!(floor.fire_exits().count(), 2);
}

/// The M1 demo in one assertion: stand in the hall, walk to a door.
#[test]
fn an_agent_can_path_from_the_hall_to_a_door() {
    let doc = load_fixture();
    let graph = compile(&doc);
    let floor = &graph.floors[0];

    let standing = Vec2::new(10.0, 6.0);
    assert!(
        floor.mesh.locate(standing).is_some(),
        "the middle of the hall must be walkable"
    );

    for door in &floor.doors {
        // Aim just inside the doorway — the threshold itself is the boundary.
        let target = door.midpoint() + Vec2::new(0.0, 0.05);
        let path = floor
            .mesh
            .find_path(standing, target)
            .unwrap_or_else(|| panic!("no path to door {}", door.opening));

        assert_eq!(path.first(), Some(&standing));
        assert_eq!(path.last(), Some(&target));

        // The hall is convex and empty, so the route is a straight line.
        let straight = standing.distance(target);
        assert!(
            (path_length(&path) - straight).abs() < 1e-9,
            "path to {} is not straight: {path:?}",
            door.opening
        );
    }
}

#[test]
fn the_mesh_is_structurally_sound() {
    let doc = load_fixture();
    let graph = compile(&doc);
    let mesh = &graph.floors[0].mesh;

    let errs = mesh.tri.check_invariants();
    assert!(errs.is_empty(), "{errs:#?}");

    // Every walkable triangle is reachable from every other.
    let walkable: Vec<_> = mesh
        .tri
        .live()
        .filter(|(i, _)| mesh.is_walkable(*i))
        .map(|(i, _)| i)
        .collect();
    assert!(walkable.len() >= 2);
    for w in &walkable[1..] {
        assert!(
            mesh.find_path(mesh.centroids[walkable[0]], mesh.centroids[*w])
                .is_some(),
            "triangle {w} is cut off from the rest of the floor"
        );
    }
}

/// Compiling is pure: the same document must produce the same mesh every time.
/// A compiler that varies run to run would break the determinism guarantee that
/// makes the browser preview and the server report agree.
#[test]
fn compilation_is_deterministic() {
    let doc = load_fixture();
    let a = compile(&doc);
    let b = compile(&doc);

    assert_eq!(a.floors.len(), b.floors.len());
    let (fa, fb) = (&a.floors[0], &b.floors[0]);

    assert_eq!(fa.mesh.tri.triangles.len(), fb.mesh.tri.triangles.len());
    assert_eq!(fa.mesh.tri.points, fb.mesh.tri.points);
    assert_eq!(fa.mesh.portals.len(), fb.mesh.portals.len());
    assert_eq!(fa.walkable_area().to_bits(), fb.walkable_area().to_bits());

    for (x, y) in fa.doors.iter().zip(&fb.doors) {
        assert_eq!(x.opening, y.opening);
        assert_eq!(x.inside, y.inside);
        assert_eq!(x.width_m.to_bits(), y.width_m.to_bits());
    }
}
