//! Vertical links, compiled to something the simulation can route to.
//!
//! An authored `Link` carries a footprint polygon per end, which is right for
//! drawing and wrong for routing. The compiler resolves each to a landing point
//! on walkable floor. A link that cannot be resolved is **reported**, never
//! silently dropped: a staircase that exists in the drawing and not in the model
//! makes an egress analysis describe a different building.

use cf_compile::{compile, CompileWarning};
use cf_geom::Vec2;
use cf_schema::ids::WallId;
use cf_schema::ids::{FloorId, LinkId};
use cf_schema::venue::{Floor, Link, LinkEnd, LinkKind, Wall, WallKind};
use cf_schema::{Polygon, Polyline, VenueDoc};

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

/// A sealed 20 x 12 rectangle at the given level.
fn floor(id: &str, level: f64) -> Floor {
    let mut f = Floor::empty(id, id, level);
    f.walls = vec![
        wall("w_s", &[(0.0, 0.0), (20.0, 0.0)]),
        wall("w_e", &[(20.0, 0.0), (20.0, 12.0)]),
        wall("w_n", &[(20.0, 12.0), (0.0, 12.0)]),
        wall("w_w", &[(0.0, 12.0), (0.0, 0.0)]),
    ];
    f
}

fn poly(pts: &[(f64, f64)]) -> Polygon {
    Polygon(pts.iter().map(|(x, y)| Vec2::new(*x, *y)).collect())
}

fn stair(a: &str, b: &str, footprint: &[(f64, f64)]) -> Link {
    Link {
        id: LinkId::new("lnk_stair"),
        kind: LinkKind::Stair,
        name: None,
        ends: vec![
            LinkEnd {
                floor: FloorId::new(a),
                footprint: poly(footprint),
            },
            LinkEnd {
                floor: FloorId::new(b),
                footprint: poly(footprint),
            },
        ],
        width_m: 1.4,
        clear_width_m: Some(1.2),
        steps: Some(18),
        riser_m: Some(0.17),
        going_m: Some(0.28),
        direction: Default::default(),
        flow_rate_ppmm: None,
        speed_multiplier_up: None,
        speed_multiplier_down: None,
        schedule: Vec::new(),
    }
}

fn two_floors(link: Option<Link>) -> VenueDoc {
    let mut v = VenueDoc::empty("vnu_t", "Two floors");
    v.floors = vec![floor("f0", 0.0), floor("f1", 4.0)];
    v.links = link.into_iter().collect();
    v
}

#[test]
fn a_stair_resolves_to_a_landing_on_each_floor() {
    let g = compile(&two_floors(Some(stair(
        "f0",
        "f1",
        &[(8.0, 4.0), (12.0, 4.0), (12.0, 8.0), (8.0, 8.0)],
    ))));

    assert_eq!(g.links.len(), 1, "{:#?}", g.warnings);
    let l = &g.links[0];
    assert_eq!(l.ends[0].floor, 0);
    assert_eq!(l.ends[1].floor, 1);
    // Both landings must be inside the footprint, and on walkable floor.
    for e in &l.ends {
        assert!(g.floors[e.floor].mesh.locate(e.point).is_some());
        assert!(e.point.x >= 8.0 && e.point.x <= 12.0);
    }
    // Clear width is what egress capacity is computed from, not nominal width.
    assert!((l.clear_width_m - 1.2).abs() < 1e-9);
}

#[test]
fn stairs_default_to_the_green_guide_ratio() {
    // 66 persons/m/min on stairs against 82 on the level. A document that does
    // not state a multiplier should not silently get 1.0 — a staircase walked
    // at foyer pace is the optimistic error.
    let g = compile(&two_floors(Some(stair(
        "f0",
        "f1",
        &[(8.0, 4.0), (12.0, 4.0), (12.0, 8.0), (8.0, 8.0)],
    ))));
    let l = &g.links[0];
    assert!((l.speed_up - 66.0 / 82.0).abs() < 1e-9);
    assert!((l.speed_down - 66.0 / 82.0).abs() < 1e-9);
}

#[test]
fn an_authored_multiplier_wins_over_the_default() {
    let mut link = stair(
        "f0",
        "f1",
        &[(8.0, 4.0), (12.0, 4.0), (12.0, 8.0), (8.0, 8.0)],
    );
    link.speed_multiplier_up = Some(0.5);
    link.speed_multiplier_down = Some(0.7);
    let g = compile(&two_floors(Some(link)));
    assert!((g.links[0].speed_up - 0.5).abs() < 1e-9);
    assert!((g.links[0].speed_down - 0.7).abs() < 1e-9);
}

#[test]
fn a_link_off_the_floor_is_reported_not_dropped() {
    let g = compile(&two_floors(Some(stair(
        "f0",
        "f1",
        // Well outside the 20 x 12 hall.
        &[(90.0, 90.0), (95.0, 90.0), (95.0, 95.0), (90.0, 95.0)],
    ))));

    assert!(g.links.is_empty());
    assert!(
        g.warnings.iter().any(|w| matches!(
            w,
            CompileWarning::LinkNotUsable { link, .. } if link.as_str() == "lnk_stair"
        )),
        "{:#?}",
        g.warnings
    );
}

#[test]
fn a_link_naming_a_floor_that_did_not_compile_is_reported() {
    let mut v = two_floors(Some(stair(
        "f0",
        "f_missing",
        &[(8.0, 4.0), (12.0, 4.0), (12.0, 8.0), (8.0, 8.0)],
    )));
    v.links[0].ends[1].floor = FloorId::new("f_missing");
    let g = compile(&v);

    assert!(g.links.is_empty());
    assert!(g
        .warnings
        .iter()
        .any(|w| matches!(w, CompileWarning::LinkNotUsable { .. })));
}

#[test]
fn a_void_is_not_a_route_between_floors() {
    // An atrium edge connects nothing. It should not become a staircase, and
    // it should not warn either — it is not a broken link, it is not a link.
    let mut link = stair(
        "f0",
        "f1",
        &[(8.0, 4.0), (12.0, 4.0), (12.0, 8.0), (8.0, 8.0)],
    );
    link.kind = LinkKind::Opening;
    let g = compile(&two_floors(Some(link)));

    assert!(g.links.is_empty());
    assert!(!g
        .warnings
        .iter()
        .any(|w| matches!(w, CompileWarning::LinkNotUsable { .. })));
}

#[test]
fn a_venue_with_no_links_compiles_to_none() {
    let g = compile(&two_floors(None));
    assert!(g.links.is_empty());
    assert_eq!(g.floors.len(), 2);
}
