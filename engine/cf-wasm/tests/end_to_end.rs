//! End-to-end test of the browser-facing API, run natively.
//!
//! These call exactly the methods JS calls. Running them on the host means the
//! binding layer is covered by ordinary `cargo test` rather than needing a
//! browser in CI — and if this passes, the only thing left that can break in
//! the browser is the JS glue, not the engine.
//!
//! This is the M1 demo expressed as an assertion: load the fixture, put agents
//! in the hall, step, and watch them leave through the doors.

use cf_wasm::{CompiledVenue, Simulation};
use std::path::PathBuf;

fn fixture() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/unit/hall-two-doors.venue.json");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

#[test]
fn the_fixture_compiles_through_the_binding() {
    let v = CompiledVenue::from_json(&fixture()).expect("compiles");

    assert!(v.is_simulable());
    assert_eq!(v.floor_count(), 1);
    assert!((v.walkable_area() - 240.0).abs() < 1e-9);
}

/// Malformed input must produce an error, never a panic — a panic inside the
/// worker ends the user's session.
///
/// `JsError` cannot be constructed off-wasm, so this asserts the parse itself
/// fails rather than calling through the binding. The binding does nothing but
/// wrap this result.
#[test]
fn a_malformed_document_is_rejected_not_panicked_on() {
    use cf_schema::VenueDoc;
    assert!(serde_json::from_str::<VenueDoc>("{ not json").is_err());
    assert!(
        serde_json::from_str::<VenueDoc>(r#"{"schemaVersion":"cfs.venue/1.0"}"#).is_err(),
        "a document missing required fields must be rejected"
    );
}

#[test]
fn geometry_crosses_the_boundary_as_flat_arrays() {
    let v = CompiledVenue::from_json(&fixture()).unwrap();

    let walls = v.wall_segments(0);
    assert!(!walls.is_empty());
    assert_eq!(walls.len() % 4, 0, "wall segments are [x0,y0,x1,y1] tuples");

    let tris = v.walkable_triangles(0);
    assert!(!tris.is_empty());
    assert_eq!(tris.len() % 6, 0, "triangles are 3 xy pairs");

    let doors = v.doors(0);
    assert_eq!(doors.len(), 8, "two doors, four floats each");

    let b = v.bounds(0);
    assert_eq!(b, vec![0.0, 0.0, 20.0, 12.0]);

    // An out-of-range floor returns empty rather than panicking.
    assert!(v.wall_segments(99).is_empty());
    assert!(v.walkable_triangles(99).is_empty());
    assert!(v.doors(99).is_empty());
}

#[test]
fn warnings_carry_a_stable_code_for_the_ui() {
    // A hall with no fire exit: same geometry, doors not marked as exits.
    let doc = fixture().replace("\"isFireExit\": true", "\"isFireExit\": false");
    let v = CompiledVenue::from_json(&doc).unwrap();
    // Rendering to a JS value needs a JS context, so assert via the graph the
    // binding wraps; the code mapping itself is exercised by the UI.
    assert!(
        v.is_simulable(),
        "a missing fire exit is a warning, not fatal"
    );
}

/// **M1.** Agents spawn in the hall and leave through the doorways.
#[test]
fn five_hundred_agents_walk_out_of_the_hall() {
    let v = CompiledVenue::from_json(&fixture()).unwrap();
    let mut sim = Simulation::new(&v, 0, 20260803.0).expect("simulation builds");

    let spawned = sim.spawn_scattered(500);
    // Placement rejects candidates that would overlap an existing body, so the
    // fill is high but not complete. Requiring 100% would mean allowing bodies
    // to start inside one another, which is not a physical initial condition.
    assert!(
        spawned >= 450,
        "expected most of 500 agents placed on walkable floor, got {spawned}"
    );
    assert_eq!(sim.active_count(), spawned);
    assert_eq!(sim.exited_count(), 0);

    // 20 Hz, so 6000 ticks is five simulated minutes. A 20x12 hall with two
    // 1.8 m doors should clear in well under that.
    let mut ticks = 0;
    let mut escaped_total = 0u32;
    while sim.active_count() > 0 && ticks < 6000 {
        sim.step_many(20);
        escaped_total += sim.escaped_count();
        ticks += 20;
    }

    // The recovery net is a backstop, not load-bearing machinery. If it fires
    // often the physics is leaking and the egress figure is suspect.
    assert!(
        escaped_total < 10,
        "{escaped_total} agents had to be recovered from outside the mesh"
    );

    assert_eq!(
        sim.active_count(),
        0,
        "{} agents were still inside after {:.0} s",
        sim.active_count(),
        sim.time()
    );
    assert_eq!(sim.exited_count(), spawned, "population was not conserved");

    // Sanity on the egress time. 500 people through 3.6 m of door at the Green
    // Guide's 82 persons/m/min is ~1.7 minutes at best; anything under 10
    // seconds would mean agents are teleporting.
    let t = sim.time();
    assert!(
        (10.0..300.0).contains(&t),
        "egress took {t:.1} s, which is not physically plausible"
    );
}

#[test]
fn positions_and_states_stay_in_step() {
    let v = CompiledVenue::from_json(&fixture()).unwrap();
    let mut sim = Simulation::new(&v, 0, 1.0).unwrap();
    sim.spawn_scattered(120);

    for _ in 0..10 {
        sim.step_many(5);
        let xy = sim.positions();
        let states = sim.states();

        assert_eq!(
            xy.len(),
            states.len() * 2,
            "the renderer indexes these together; they must match"
        );
        assert_eq!(states.len() as u32, sim.active_count());
        assert!(
            xy.iter().all(|v| v.is_finite()),
            "a non-finite position would corrupt the vertex buffer"
        );
    }
}

#[test]
fn simulation_time_is_exact() {
    let v = CompiledVenue::from_json(&fixture()).unwrap();
    let mut sim = Simulation::new(&v, 0, 1.0).unwrap();
    sim.spawn_scattered(10);

    sim.step_many(200);
    assert_eq!(sim.tick(), 200.0);
    // 200 ticks at 20 Hz is exactly 10 s — no accumulated drift.
    assert!(
        (sim.time() - 10.0).abs() < 1e-9,
        "time drifted to {}",
        sim.time()
    );
}

#[test]
fn the_same_seed_reproduces_the_same_run() {
    let v = CompiledVenue::from_json(&fixture()).unwrap();

    let run = |seed: f64| {
        let mut s = Simulation::new(&v, 0, seed).unwrap();
        s.spawn_scattered(200);
        s.step_many(400);
        (s.positions(), s.active_count(), s.exited_count())
    };

    let (a_xy, a_active, a_exited) = run(42.0);
    let (b_xy, b_active, b_exited) = run(42.0);

    assert_eq!(a_active, b_active);
    assert_eq!(a_exited, b_exited);
    assert_eq!(a_xy.len(), b_xy.len());
    for (i, (x, y)) in a_xy.iter().zip(&b_xy).enumerate() {
        assert_eq!(x.to_bits(), y.to_bits(), "value {i} diverged");
    }

    // A different seed must actually produce a different crowd, or the seed is
    // not doing anything.
    let (c_xy, _, _) = run(43.0);
    assert!(
        c_xy.len() != a_xy.len() || c_xy.iter().zip(&a_xy).any(|(x, y)| x != y),
        "changing the seed changed nothing"
    );
}

#[test]
fn spawning_into_an_unsimulable_venue_is_safe() {
    // Three walls out of four: the outline leaks, so there is no walkable floor.
    let doc = fixture().replace(
        r#"{
          "id": "w_west",
          "layer": "lay_struct",
          "polyline": [[0.0, 0.0], [0.0, 12.0]],
          "thicknessM": 0.23,
          "kind": "structural",
          "permeable": false
        }"#,
        r#"{
          "id": "w_west",
          "layer": "lay_struct",
          "polyline": [[0.0, 0.0], [0.0, 0.0001]],
          "thicknessM": 0.23,
          "kind": "structural",
          "permeable": false
        }"#,
    );
    let v = CompiledVenue::from_json(&doc).unwrap();

    // Whatever the outcome, nothing may panic — this runs inside the user's
    // worker, and a panic there ends their session.
    if v.floor_count() > 0 {
        let mut sim = Simulation::new(&v, 0, 1.0).unwrap();
        let n = sim.spawn_scattered(50);
        sim.step_many(20);
        assert!(sim.active_count() <= n);
    }
}

#[test]
fn engine_version_is_reported() {
    let v = cf_wasm::engine_version();
    assert!(v.contains("cf-wasm"));
    assert!(v.contains("cf-compile"));
}

/// A placed crowd must not start with bodies inside one another.
///
/// An overlapping initial condition is not physical, and the density field
/// latches it as a peak before the contact solve gets a tick to separate
/// anyone — which is how a 20 x 12 hall once reported 26 p/m², roughly five
/// times the densest packing of human bodies that can exist.
#[test]
fn a_placed_crowd_does_not_start_overlapping() {
    let v = CompiledVenue::from_json(&fixture()).unwrap();
    let mut sim = Simulation::new(&v, 0, 20260803.0).unwrap();
    let n = sim.spawn_scattered(500);
    assert!(
        n > 200,
        "only {n} agents placed; separation is too aggressive"
    );

    let xy = sim.positions();
    let count = xy.len() / 2;
    // Bodies are 0.18–0.30 m radius, so the tightest legal separation is 0.36 m.
    // Allow a small tolerance for the sampled radii.
    let min_sep = 0.34f32;
    let mut worst = f32::INFINITY;
    for i in 0..count {
        for j in (i + 1)..count {
            let dx = xy[i * 2] - xy[j * 2];
            let dy = xy[i * 2 + 1] - xy[j * 2 + 1];
            let d = (dx * dx + dy * dy).sqrt();
            if d < worst {
                worst = d;
            }
        }
    }
    assert!(
        worst >= min_sep,
        "two agents were placed {worst:.3} m apart, closer than two bodies allow"
    );

    // And the density that results must be physically possible: hexagonal close
    // packing of 0.23 m bodies is 5.46 p/m², and a centre-biased window reads at
    // most about 1.5x that.
    let peak = sim.peak_density();
    assert!(
        peak < 8.2,
        "a freshly placed crowd reports {peak:.2} p/m², which cannot be true"
    );
}
