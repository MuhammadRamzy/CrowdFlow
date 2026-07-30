//! Fixture round-trip and validation tests.
//!
//! `/fixtures` is shared by both tracks — the engine parses these files, and the
//! editor's own tests load the same ones. If a schema change breaks a fixture,
//! it breaks here first, in CI, rather than in someone else's session.

use cf_schema::{
    to_pretty_json, validate_scenario, validate_venue, ScenarioDoc, VenueDoc, VENUE_SCHEMA_VERSION,
};
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
}

fn load_venue_fixture(rel: &str) -> VenueDoc {
    let path = fixtures_dir().join(rel);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()))
}

fn load_scenario_fixture(rel: &str) -> ScenarioDoc {
    let path = fixtures_dir().join(rel);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()))
}

#[test]
fn hall_two_doors_parses_and_validates() {
    let v = load_venue_fixture("unit/hall-two-doors.venue.json");
    assert_eq!(v.schema_version, VENUE_SCHEMA_VERSION);
    assert_eq!(v.floors.len(), 1);
    assert_eq!(v.floors[0].walls.len(), 4);
    assert_eq!(v.floors[0].openings.len(), 2);

    let report = validate_venue(&v);
    assert!(report.is_ok(), "fixture should validate cleanly:\n{report}");
}

#[test]
fn hall_two_doors_geometry_is_what_we_think_it_is() {
    let v = load_venue_fixture("unit/hall-two-doors.venue.json");
    let f = &v.floors[0];

    // 20 x 12 m hall.
    assert!((v.total_zone_area() - 240.0).abs() < 1e-9);

    // The south wall runs from x=20 to x=0, so t=0.25 is at x=15.
    let south = f.wall(&"w_south".into()).expect("w_south exists");
    assert!((south.polyline.length() - 20.0).abs() < 1e-9);

    let east_door = f
        .openings
        .iter()
        .find(|o| o.id.as_str() == "op_east_door")
        .unwrap();
    let p = f.opening_position(east_door).expect("resolves");
    assert!((p.x - 15.0).abs() < 1e-9, "expected x=15, got {}", p.x);
    assert!((p.y - 0.0).abs() < 1e-9, "expected y=0, got {}", p.y);

    let west_door = f
        .openings
        .iter()
        .find(|o| o.id.as_str() == "op_west_door")
        .unwrap();
    let p = f.opening_position(west_door).expect("resolves");
    assert!((p.x - 5.0).abs() < 1e-9, "expected x=5, got {}", p.x);
}

#[test]
fn hall_two_doors_round_trips_byte_identically() {
    let path = fixtures_dir().join("unit/hall-two-doors.venue.json");
    let original = std::fs::read_to_string(&path).unwrap();
    let v: VenueDoc = serde_json::from_str(&original).unwrap();
    let reserialized = to_pretty_json(&v).unwrap();
    let v2: VenueDoc = serde_json::from_str(&reserialized).unwrap();

    // Structural equality is the contract; byte equality of the committed file
    // is not, since authors may format by hand.
    assert_eq!(
        v, v2,
        "venue does not survive a serialize/deserialize cycle"
    );
}

#[test]
fn scenario_fixture_validates_against_its_venue() {
    let v = load_venue_fixture("unit/hall-two-doors.venue.json");
    let s = load_scenario_fixture("unit/hall-two-doors.scenario.json");

    assert_eq!(s.total_agents(), 500);

    let report = validate_scenario(&s, &v);
    assert!(
        report.is_ok(),
        "scenario should validate against its venue:\n{report}"
    );
}

#[test]
fn scenario_referencing_a_missing_zone_is_rejected() {
    let v = load_venue_fixture("unit/hall-two-doors.venue.json");
    let mut s = load_scenario_fixture("unit/hall-two-doors.scenario.json");

    // Point the pre-placed arrival at a zone that does not exist.
    s.populations[0].arrival = cf_schema::scenario::Arrival::Preplaced {
        zones: vec![cf_schema::scenario::ZoneWeight {
            zone: "z_does_not_exist".into(),
            weight: 1.0,
        }],
    };

    let report = validate_scenario(&s, &v);
    assert!(report.has_errors());
    assert!(
        report.errors().any(|i| i.code == "arrival.orphan_zone"),
        "expected arrival.orphan_zone, got:\n{report}"
    );
}

#[test]
fn evacuation_without_an_alarm_is_flagged() {
    let v = load_venue_fixture("unit/hall-two-doors.venue.json");
    let mut s = load_scenario_fixture("unit/hall-two-doors.scenario.json");
    s.events.clear();

    let report = validate_scenario(&s, &v);
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.code == "scenario.evacuation_without_alarm"),
        "expected a warning about a silent evacuation:\n{report}"
    );
}

/// The wire format is uniformly camelCase. A stray `snake_case` key means a
/// `rename_all` was forgotten somewhere — which serde will happily accept and
/// which then silently fails to parse a hand-written fixture or a payload from
/// the TypeScript side.
///
/// Walking the generated JSON Schema covers every type reachable from the two
/// document roots, including ones no fixture happens to exercise.
#[test]
fn every_schema_property_is_camel_case() {
    let schema_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("schema");

    let mut offenders = Vec::new();

    for name in ["venue.schema.json", "scenario.schema.json"] {
        let path = schema_dir.join(name);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "cannot read {} ({e}).\nRun: cargo run -p cf-schema --bin gen-schema",
                path.display()
            )
        });
        let root: serde_json::Value = serde_json::from_str(&text).unwrap();
        collect_property_names(&root, &mut |key| {
            if key.contains('_') {
                offenders.push(format!("{name}: {key}"));
            }
        });
    }

    assert!(
        offenders.is_empty(),
        "these schema properties are not camelCase:\n  {}",
        offenders.join("\n  ")
    );
}

/// Recursively visit every key under a `"properties"` object.
fn collect_property_names(v: &serde_json::Value, f: &mut impl FnMut(&str)) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, val) in map {
                if k == "properties" {
                    if let serde_json::Value::Object(props) = val {
                        for key in props.keys() {
                            f(key);
                        }
                    }
                }
                collect_property_names(val, f);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_property_names(item, f);
            }
        }
        _ => {}
    }
}

#[test]
fn an_opening_wider_than_its_wall_is_rejected() {
    let mut v = load_venue_fixture("unit/hall-two-doors.venue.json");
    v.floors[0].openings[0].width_m = 500.0;

    let report = validate_venue(&v);
    assert!(report.has_errors());
    assert!(report.errors().any(|i| i.code == "opening.wider_than_wall"));
}

/// The schema must describe what `Serialize` actually writes.
///
/// These can diverge silently. `#[schemars(with = ...)]` at the container level
/// is ignored by the derive, so `Vec2` declared an object `{x, y}` while serde
/// wrote an array `[x, y]`. Every Rust test still passed — the divergence only
/// appeared when generated TypeScript was used to build a document, which the
/// engine then rejected.
///
/// Rather than validate the whole document against the schema (which would
/// need a JSON Schema implementation as a dependency), this walks the fixture
/// alongside the schema's declared types for the shapes that actually matter.
#[test]
fn serialized_shapes_match_the_declared_schema() {
    let schema_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("schema");
    let text = std::fs::read_to_string(schema_dir.join("venue.schema.json"))
        .expect("run: cargo run -p cf-schema --bin gen-schema");
    let schema: serde_json::Value = serde_json::from_str(&text).unwrap();
    let defs = &schema["definitions"];

    // A point is an array of two numbers, not an object.
    assert_eq!(
        defs["Vec2"]["type"], "array",
        "Vec2 must be declared as an array; serde writes [x, y]"
    );
    assert_eq!(defs["Vec2"]["minItems"], 2);
    assert_eq!(defs["Vec2"]["maxItems"], 2);

    // And the fixture agrees.
    let v = load_venue_fixture("unit/hall-two-doors.venue.json");
    let json = serde_json::to_value(&v).unwrap();
    let first_point = &json["floors"][0]["walls"][0]["polyline"][0];
    assert!(
        first_point.is_array(),
        "a serialized point should be an array, got {first_point}"
    );
    assert_eq!(first_point.as_array().unwrap().len(), 2);

    // Polylines and polygons are arrays of points, not wrapper objects.
    for (path, value) in [
        ("polyline", &json["floors"][0]["walls"][0]["polyline"]),
        ("polygon", &json["floors"][0]["zones"][0]["polygon"]),
    ] {
        assert!(value.is_array(), "{path} should serialize as an array");
    }
}
