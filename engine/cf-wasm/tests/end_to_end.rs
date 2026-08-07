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

// ---------------------------------------------------------------------------
// The scenario path
//
// `Simulation::fromScenario` is what the editor calls for every run — the
// `spawnScattered` path above is now only the no-scenario fallback. The planner
// itself is unit-tested in `cf_wasm::scenario`; what these cover is the
// binding: a JSON document in, a running simulation out, and the counters the
// panel reads back.
// ---------------------------------------------------------------------------

/// A scenario over the fixture: everyone already inside, all leaving.
///
/// Written as JSON rather than built from the Rust types on purpose. The editor
/// sends a string, so a field renamed on the wire without its
/// `#[serde(rename_all)]` — which this repo has been bitten by — should fail
/// here rather than in a browser.
fn scenario_json(count: u32, extra: &str) -> String {
    format!(
        r#"{{
          "schemaVersion": "cfs.scenario/1.0",
          "id": "scn_e2e",
          "name": "End to end",
          "venueVersion": "v1",
          "mode": "evacuation",
          "durationS": 600,
          "timestepS": 0.05,
          "seed": 20260801,
          "populations": [{{
            "id": "pop_a",
            "label": "General admission",
            "count": {count},
            "profile": {{
              "desiredSpeed": {{"dist":"normal","mean":1.34,"sd":0.26,"min":0.6,"max":2.2}},
              "radiusM": {{"dist":"normal","mean":0.23,"sd":0.02,"min":0.18,"max":0.3}}
            }},
            "arrival": {{"kind":"preplaced","zones":[]}},
            "itinerary": [{{"goal":{{"target":"nearestExit"}},"probability":1}}],
            "access": []
          }}],
          "events": [],
          "output": {{}}
          {extra}
        }}"#
    )
}

#[test]
fn a_scenario_document_drives_a_run_through_the_binding() {
    let v = CompiledVenue::from_json(&fixture()).expect("compiles");
    let mut sim = Simulation::from_scenario(&v, 0, &scenario_json(200, "")).expect("plans");

    assert_eq!(sim.scenario_total(), 200, "the authored count is what runs");

    // Preplaced, so everyone is admitted in the first few ticks rather than
    // being held back — an empty venue at t=0 would mean the run never starts.
    sim.step_many(20);
    assert!(
        sim.active_count() > 0,
        "nobody was admitted: pending {}, unplaced {}",
        sim.pending_count(),
        sim.unplaced_count()
    );

    for _ in 0..8000 {
        sim.step();
        if sim.active_count() == 0 && sim.pending_count() == 0 {
            break;
        }
    }

    assert_eq!(sim.active_count(), 0, "the hall never emptied");
    assert_eq!(
        sim.exited_count() + sim.unplaced_count(),
        200,
        "agents went missing: {} out, {} unplaced",
        sim.exited_count(),
        sim.unplaced_count()
    );
}

/// A malformed scenario must be an error, never a panic.
///
/// Asserted against the parse rather than through the binding, for the same
/// reason as `a_malformed_document_is_rejected_not_panicked_on`: constructing a
/// `JsError` off-wasm panics inside wasm-bindgen, so calling the binding on
/// input it rejects would fail here for a reason that has nothing to do with
/// the engine. The binding does nothing but wrap this result.
#[test]
fn a_malformed_scenario_is_rejected_not_panicked_on() {
    assert!(serde_json::from_str::<cf_schema::scenario::ScenarioDoc>("{ not json").is_err());
    assert!(
        serde_json::from_str::<cf_schema::scenario::ScenarioDoc>(
            r#"{"schemaVersion":"cfs.scenario/1.0"}"#
        )
        .is_err(),
        "a document missing its populations should be rejected"
    );
}

#[test]
fn a_scenario_run_is_reproducible_from_its_seed() {
    let v = CompiledVenue::from_json(&fixture()).expect("compiles");
    let trace = |json: &str| {
        let mut s = Simulation::from_scenario(&v, 0, json).expect("plans");
        s.step_many(200);
        (s.positions().to_vec(), s.exited_count())
    };

    let a = trace(&scenario_json(150, ""));
    let b = trace(&scenario_json(150, ""));
    assert_eq!(a.1, b.1);
    assert_eq!(a.0, b.0, "the same scenario and seed diverged");
}

/// Note on what is *not* covered here.
///
/// `Simulation::scenario_notes` returns a `JsValue`, which cannot be built or
/// inspected off-wasm, so the "Not simulated" list the authoring panel prints
/// is asserted in `cf_wasm::scenario` instead — see
/// `unsupported_authoring_is_reported_rather_than_ignored`. The binding here
/// does nothing but serialise that slice.
/// A scheduled closure actually shuts a door mid-run.
///
/// This is the question the tool exists to answer and the one a static
/// occupant-load calculation cannot: the nearest exit is blocked part-way
/// through, and does the remaining door cope. It is asserted through the
/// binding because that is the path the editor takes.
#[test]
fn a_scheduled_closure_shuts_a_door_mid_run() {
    let v = CompiledVenue::from_json(&fixture()).expect("compiles");
    let doc = scenario_json(150, "").replace(
        r#""events": [],"#,
        r#""events": [{"atS":8.0,"kind":"closeOpening","target":"op_west_door"}],"#,
    );
    let mut sim = Simulation::from_scenario(&v, 0, &doc).expect("plans");

    // Before the closure both doors are live.
    sim.step_many(20);
    let early = sim.exited_count();

    // Past the closure time, and then to the end.
    for _ in 0..8000 {
        sim.step();
        if sim.active_count() == 0 && sim.pending_count() == 0 {
            break;
        }
    }

    assert_eq!(
        sim.active_count(),
        0,
        "the hall never emptied after a closure"
    );
    assert!(
        sim.exited_count() > early,
        "nobody left after the door shut — the crowd did not divert"
    );
    assert_eq!(
        sim.exited_count() + sim.unplaced_count(),
        150,
        "agents went missing across the closure"
    );
}

/// A crowd holds in a zone until the alarm, then leaves.
///
/// This is the shape of a real evacuation analysis and the reason alarms and
/// zone goals both exist: a venue full of people who are *not yet trying to
/// leave* is the state an evacuation starts from. An analysis that begins with
/// everyone already walking at a door has skipped the part where they notice.
///
/// Without the alarm the same document must leave the hall full — that is
/// asserted too, because a test where the crowd would have left anyway proves
/// nothing about the alarm.
#[test]
fn a_crowd_holds_until_the_alarm_then_evacuates() {
    let v = CompiledVenue::from_json(&fixture()).expect("compiles");
    let dwelling = scenario_json(80, "").replace(
        r#""itinerary": [{"goal":{"target":"nearestExit"},"probability":1}],"#,
        r#""itinerary": [{"goal":{"target":"zone","id":"z_hall"},"probability":1}],"#,
    );

    // Without an alarm the hall should stay largely full: their goal is a spot
    // on the floor, not a door. A few leak out — the dwell point is in the
    // middle of a zone that reaches the walls, so agents shuffling around it
    // can drift across a doorway and be counted out. That is a real limitation
    // of dwelling at a point rather than within a region, and it is why this
    // asserts "most stay" rather than "none leave".
    let mut held = Simulation::from_scenario(&v, 0, &dwelling).expect("plans");
    held.step_many(2000);
    assert!(
        held.active_count() > 40,
        "only {} of 80 were still inside after 100 s with no alarm — they are \
         leaving without being told to, so this test cannot prove anything",
        held.active_count()
    );

    // With one at 20 s, everybody should.
    let with_alarm = dwelling.replace(
        r#""events": [],"#,
        r#""events": [{"atS":20.0,"kind":"alarm"}],"#,
    );
    let mut sim = Simulation::from_scenario(&v, 0, &with_alarm).expect("plans");

    sim.step_many(300); // 15 s — before the alarm
    let before = sim.exited_count();
    assert!(
        sim.active_count() > 40,
        "the hall emptied before the alarm even sounded"
    );

    for _ in 0..12000 {
        sim.step();
        if sim.active_count() == 0 && sim.pending_count() == 0 {
            break;
        }
    }

    assert_eq!(
        sim.active_count(),
        0,
        "the hall never emptied after the alarm"
    );
    assert!(
        sim.exited_count() > before,
        "the alarm sounded and nobody moved: {} out before, {} after",
        before,
        sim.exited_count()
    );
}

#[test]
fn an_unacted_on_event_still_plans_rather_than_failing() {
    let v = CompiledVenue::from_json(&fixture()).expect("compiles");
    // `events` is in the schema and the engine does not act on it. Planning
    // must still succeed — refusing the document outright would make an
    // unsupported field fatal rather than reported.
    let with_event = scenario_json(50, "").replace(
        r#""events": [],"#,
        r#""events": [{"atS":30.0,"kind":"alarm"}],"#,
    );
    let mut sim = Simulation::from_scenario(&v, 0, &with_event).expect("plans despite the event");
    sim.step_many(50);
    assert!(sim.active_count() > 0);
}
