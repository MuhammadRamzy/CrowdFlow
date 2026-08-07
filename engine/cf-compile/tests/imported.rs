//! The Python importer's output, compiled by the Rust engine.
//!
//! This is the seam that matters between the two tracks. Both sides generate
//! their types from `schema/` (ADR 0001), so they *should* agree — but "should"
//! has been wrong here twice, and both times every test on each side passed
//! while the boundary was broken.
//!
//! `fixtures/unit/imported-hall.venue.json` is real importer output, not a
//! hand-written approximation of it. Regenerate with:
//!
//! ```text
//! cd services && python3 -m pytest tests/   # writes the fixture
//! ```

use cf_compile::compile;
use cf_schema::VenueDoc;

#[test]
fn a_document_from_the_importer_compiles_and_is_simulable() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/unit/imported-hall.venue.json"
    );
    let text = std::fs::read_to_string(path).expect("fixture exists");

    // Deserialising at all is the contract check: if the Python side emitted a
    // field under a different name, this is where it fails.
    let doc: VenueDoc = serde_json::from_str(&text).expect("importer output parses");
    let g = compile(&doc);

    assert_eq!(g.floors.len(), 1, "{:#?}", g.warnings);
    let f = &g.floors[0];

    // The drawing was a 20 x 12 m hall. Anything wildly off means a scaling
    // bug survived the boundary — which is exactly the failure the importer's
    // own tests were written to catch, checked here from the other side.
    let area = f.walkable_area();
    assert!(
        (100.0..400.0).contains(&area),
        "walkable area {area:.1} m^2 is not a 20 x 12 hall — suspect scaling"
    );

    // The inferred doorway has to survive as a real opening.
    assert!(!f.doors.is_empty(), "the imported doorway did not compile");
    assert!(
        g.fatal_warnings().next().is_none(),
        "imported venue is not simulable: {:#?}",
        g.warnings
    );
}
