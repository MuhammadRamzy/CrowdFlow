//! Tests for the compiler's diagnostics.
//!
//! `CompileWarning` is the editor's validation panel
//! (`docs/03-track-a-venue-designer.md` §A2) and the mechanism by which the
//! engine tells an author what is wrong with their geometry. A warning that
//! does not fire is a silently broken venue.

use cf_compile::{compile, CompileWarning};
use cf_geom::{Polygon, Polyline, Vec2};
use cf_schema::ids::{ObstacleId, OpeningId, WallId, ZoneId};
use cf_schema::venue::*;

fn poly(pts: &[(f64, f64)]) -> Polygon {
    Polygon(pts.iter().map(|(x, y)| Vec2::new(*x, *y)).collect())
}

fn wall(id: &str, pts: &[(f64, f64)]) -> Wall {
    Wall {
        id: WallId::new(id),
        layer: None,
        polyline: Polyline(pts.iter().map(|(x, y)| Vec2::new(*x, *y)).collect()),
        thickness_m: 0.2,
        kind: WallKind::Structural,
        permeable: false,
        provenance: None,
    }
}

fn opening(id: &str, wall_id: &str, t: f64, width_m: f64, fire: bool) -> Opening {
    Opening {
        id: OpeningId::new(id),
        wall: WallId::new(wall_id),
        t,
        width_m,
        kind: OpeningKind::Door,
        swing: Swing::Both,
        is_fire_exit: fire,
        capacity_factor: 1.0,
        schedule: Vec::new(),
        provenance: None,
    }
}

/// A closed 20 x 12 rectangle, four walls, no openings.
fn rect_floor() -> Floor {
    let mut f = Floor::empty("f0", "Ground", 0.0);
    f.walls = vec![
        wall("w_s", &[(0.0, 0.0), (20.0, 0.0)]),
        wall("w_e", &[(20.0, 0.0), (20.0, 12.0)]),
        wall("w_n", &[(20.0, 12.0), (0.0, 12.0)]),
        wall("w_w", &[(0.0, 12.0), (0.0, 0.0)]),
    ];
    f
}

fn venue(floor: Floor) -> VenueDoc {
    let mut v = VenueDoc::empty("vnu_test", "Test");
    v.floors = vec![floor];
    v
}

fn has<F: Fn(&CompileWarning) -> bool>(g: &cf_compile::NavGraph, f: F) -> bool {
    g.warnings.iter().any(f)
}

#[test]
fn a_sealed_rectangle_compiles_without_complaint() {
    let g = compile(&venue(rect_floor()));
    assert!(g.is_simulable(), "{:#?}", g.warnings);
    assert!((g.total_walkable_area() - 240.0).abs() < 1e-9);
    // No openings at all, so no fire-exit warning either — that only fires
    // when doors exist but none is an exit.
    assert!(!has(&g, |w| matches!(w, CompileWarning::NoFireExit { .. })));
}

#[test]
fn a_missing_wall_leaves_no_walkable_area() {
    let mut f = rect_floor();
    f.walls.pop(); // remove the west wall
    let g = compile(&venue(f));

    assert!(
        has(&g, |w| matches!(w, CompileWarning::NoWalkableArea { .. })),
        "an open outline must be reported: {:#?}",
        g.warnings
    );
    assert!(!g.is_simulable(), "this venue cannot be simulated");
}

#[test]
fn a_narrow_door_is_flagged_but_still_compiles() {
    let mut f = rect_floor();
    f.openings = vec![opening("op_narrow", "w_s", 0.5, 0.6, true)];
    let g = compile(&venue(f));

    assert!(
        g.is_simulable(),
        "a narrow door is a warning, not a blocker"
    );
    match g
        .warnings
        .iter()
        .find(|w| matches!(w, CompileWarning::OpeningTooNarrow { .. }))
    {
        Some(CompileWarning::OpeningTooNarrow {
            opening,
            width_m,
            minimum_m,
        }) => {
            assert_eq!(opening.as_str(), "op_narrow");
            assert!((width_m - 0.6).abs() < 1e-9);
            assert!((minimum_m - cf_compile::MIN_EGRESS_WIDTH_M).abs() < 1e-9);
        }
        _ => panic!("expected OpeningTooNarrow: {:#?}", g.warnings),
    }
}

#[test]
fn an_opening_on_a_nonexistent_wall_is_reported() {
    let mut f = rect_floor();
    f.openings = vec![opening("op_ghost", "w_does_not_exist", 0.5, 1.8, true)];
    let g = compile(&venue(f));

    assert!(
        has(&g, |w| matches!(
            w,
            CompileWarning::OpeningOrphaned { opening, .. } if opening.as_str() == "op_ghost"
        )),
        "{:#?}",
        g.warnings
    );
}

#[test]
fn doors_with_no_fire_exit_are_flagged() {
    let mut f = rect_floor();
    f.openings = vec![opening("op_a", "w_s", 0.5, 1.8, false)];
    let g = compile(&venue(f));

    assert!(
        has(&g, |w| matches!(w, CompileWarning::NoFireExit { .. })),
        "a floor whose only doors are not exits must be flagged: {:#?}",
        g.warnings
    );
}

#[test]
fn a_floor_with_no_walls_is_fatal() {
    let g = compile(&venue(Floor::empty("f0", "Ground", 0.0)));

    assert!(has(&g, |w| matches!(w, CompileWarning::EmptyFloor { .. })));
    assert!(!g.is_simulable());
    assert!(
        g.floors.is_empty(),
        "a floor with no geometry is not emitted"
    );
}

/// An obstacle must be carved out of the walkable area — this is the number
/// NFPA occupant load is computed from, so a pillar counted as floor would
/// inflate the permitted occupancy.
#[test]
fn an_obstacle_reduces_the_walkable_area() {
    let mut f = rect_floor();
    f.obstacles = vec![Obstacle {
        id: ObstacleId::new("ob_pillar"),
        layer: None,
        polygon: poly(&[(9.0, 5.0), (11.0, 5.0), (11.0, 7.0), (9.0, 7.0)]),
        kind: ObstacleKind::Pillar,
        height_m: None,
        traversable: false,
        provenance: None,
    }];
    let g = compile(&venue(f));

    assert!(g.is_simulable(), "{:#?}", g.warnings);
    assert!(
        (g.total_walkable_area() - (240.0 - 4.0)).abs() < 1e-9,
        "the 2x2 pillar must be excluded: got {}",
        g.total_walkable_area()
    );
}

#[test]
fn a_traversable_obstacle_does_not_reduce_the_area() {
    let mut f = rect_floor();
    f.obstacles = vec![Obstacle {
        id: ObstacleId::new("ob_rug"),
        layer: None,
        polygon: poly(&[(9.0, 5.0), (11.0, 5.0), (11.0, 7.0), (9.0, 7.0)]),
        kind: ObstacleKind::Generic,
        height_m: None,
        traversable: true,
        provenance: None,
    }];
    let g = compile(&venue(f));

    assert!((g.total_walkable_area() - 240.0).abs() < 1e-9);
}

#[test]
fn a_zone_off_the_floor_is_reported() {
    let mut f = rect_floor();
    f.zones = vec![Zone {
        id: ZoneId::new("z_outside"),
        name: None,
        layer: None,
        // Well clear of the 20 x 12 hall.
        polygon: poly(&[(100.0, 100.0), (110.0, 100.0), (110.0, 110.0)]),
        kind: ZoneKind::AssemblyConcentrated,
        olf_override: None,
        olf_justification: None,
        access: Vec::new(),
        speed_multiplier: 1.0,
        attractors: Vec::new(),
        is_void: false,
        provenance: None,
    }];
    let g = compile(&venue(f));

    assert!(
        has(&g, |w| matches!(
            w,
            CompileWarning::ZoneNotOnFloor { zone, .. } if zone.as_str() == "z_outside"
        )),
        "{:#?}",
        g.warnings
    );
}

#[test]
fn a_zone_on_the_floor_is_not_reported() {
    let mut f = rect_floor();
    f.zones = vec![Zone {
        id: ZoneId::new("z_hall"),
        name: None,
        layer: None,
        polygon: poly(&[(1.0, 1.0), (19.0, 1.0), (19.0, 11.0), (1.0, 11.0)]),
        kind: ZoneKind::AssemblyConcentrated,
        olf_override: None,
        olf_justification: None,
        access: Vec::new(),
        speed_multiplier: 1.0,
        attractors: Vec::new(),
        is_void: false,
        provenance: None,
    }];
    let g = compile(&venue(f));

    assert!(
        !has(&g, |w| matches!(w, CompileWarning::ZoneNotOnFloor { .. })),
        "{:#?}",
        g.warnings
    );
}

#[test]
fn overlapping_doors_are_merged_and_reported() {
    let mut f = rect_floor();
    f.openings = vec![
        opening("op_a", "w_s", 0.50, 4.0, true),
        opening("op_b", "w_s", 0.58, 4.0, true),
    ];
    let g = compile(&venue(f));

    assert!(
        has(&g, |w| matches!(w, CompileWarning::OpeningsOverlap { .. })),
        "{:#?}",
        g.warnings
    );
    // Merged into a single doorway rather than producing crossing geometry.
    assert_eq!(g.floors[0].doors.len(), 1);
    assert!(g.is_simulable());
}

#[test]
fn a_degenerate_wall_is_reported_and_skipped() {
    let mut f = rect_floor();
    f.walls.push(wall("w_zero", &[(5.0, 5.0), (5.0, 5.0)]));
    let g = compile(&venue(f));

    assert!(
        has(&g, |w| matches!(
            w,
            CompileWarning::DegenerateWall { wall, .. } if wall.as_str() == "w_zero"
        )),
        "{:#?}",
        g.warnings
    );
    // The rest of the floor still compiles.
    assert!(g.is_simulable());
    assert!((g.total_walkable_area() - 240.0).abs() < 1e-9);
}

/// Warnings must name the element they concern so the canvas can pan to it.
#[test]
fn warnings_render_with_their_element() {
    let mut f = rect_floor();
    f.openings = vec![opening("op_narrow", "w_s", 0.5, 0.6, true)];
    let g = compile(&venue(f));

    let text: Vec<String> = g.warnings.iter().map(|w| w.to_string()).collect();
    assert!(
        text.iter()
            .any(|t| t.contains("op_narrow") && t.contains("0.60")),
        "warning text should name the opening and its width: {text:#?}"
    );
}
