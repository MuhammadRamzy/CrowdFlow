//! Rule packs against figures worked by hand from the standards.
//!
//! Every expected number here is computed in the assertion's own comment from
//! the clause it cites, not read off the implementation. A compliance test that
//! agrees with the code it tests proves only that the code is self-consistent —
//! which is exactly the failure mode that matters here, because a rule can be
//! confidently, reproducibly, and completely wrong.
//!
//! These are also the fixtures a fire engineer would review. They are written
//! to be read by someone who knows the standards and not this codebase.

use cf_compliance::{Compare, Facts, Limit, Rule, RulePack, Status, Subject};

fn pack(name: &str) -> RulePack {
    let path = format!("{}/packs/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    RulePack::from_json(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn find<'a>(findings: &'a [cf_compliance::Finding], id: &str) -> &'a cf_compliance::Finding {
    findings
        .iter()
        .find(|f| f.rule_id == id)
        .unwrap_or_else(|| panic!("no finding for {id}"))
}

/// A 20 x 12 m hall — 240 m² — with two 1.6 m doors and 300 people.
fn hall() -> Facts {
    Facts {
        walkable_area_m2: 240.0,
        occupancy: 300,
        exit_count: 2,
        total_exit_width_m: 3.2,
        narrowest_exit_m: 1.6,
        egress_time_s: Some(120.0),
        peak_density: Some(1.8),
        travel_distance_m: Some(22.0),
    }
}

// ---------------------------------------------------------------------------
// NFPA 101
// ---------------------------------------------------------------------------

#[test]
fn occupant_load_matches_table_7_3_1_2() {
    // 240 m² at 0.65 m²/person concentrated assembly = 369.23, floor 369.
    // 300 occupants is within it.
    let f = pack("nfpa101").evaluate(&hall());
    let r = find(&f, "nfpa101.occupant-load");
    assert_eq!(r.status, Status::Pass, "{}", r.working);
    assert_eq!(r.limit, Some(369.0), "{}", r.working);
    assert_eq!(r.measured, Some(300.0));
}

#[test]
fn occupant_load_rounds_down_never_up() {
    // 10 m² at 3 m²/person = 3.33. Three people, not four: 0.33 of a person is
    // not a person, and rounding up licenses a venue for someone it has no
    // room for.
    let rule = Rule {
        id: "t".into(),
        clause: "7.3.1.2".into(),
        title: "t".into(),
        subject: Subject::OccupantLoad,
        compare: Compare::AtMost,
        limit: Limit::AreaPerPerson { m2_per_person: 3.0 },
        note: String::new(),
    };
    let f = Facts {
        walkable_area_m2: 10.0,
        occupancy: 4,
        ..Facts::default()
    };
    let out = cf_compliance::evaluate(&rule, &f);
    assert_eq!(out.limit, Some(3.0));
    assert_eq!(out.status, Status::Fail, "{}", out.working);
}

#[test]
fn egress_width_matches_the_capacity_method() {
    // 7.3.3.1: 5 mm per person for level components.
    // 300 × 0.005 = 1.50 m required; 3.2 m provided.
    let f = pack("nfpa101").evaluate(&hall());
    let r = find(&f, "nfpa101.egress-width");
    assert_eq!(r.limit, Some(1.5), "{}", r.working);
    assert_eq!(r.status, Status::Pass);
}

#[test]
fn a_thirty_inch_door_fails_the_minimum_clear_width() {
    // 7.2.1.2.3.2 requires 32 in = 0.8128 m; the pack uses 0.815.
    // A 30 in door is 0.762 m.
    let mut facts = hall();
    facts.narrowest_exit_m = 0.762;
    let f = pack("nfpa101").evaluate(&facts);
    let r = find(&f, "nfpa101.min-door-width");
    assert_eq!(r.status, Status::Fail, "{}", r.working);
}

#[test]
fn one_exit_fails_the_two_exit_rule() {
    let mut facts = hall();
    facts.exit_count = 1;
    let f = pack("nfpa101").evaluate(&facts);
    assert_eq!(find(&f, "nfpa101.two-exits").status, Status::Fail);
}

// ---------------------------------------------------------------------------
// Green Guide
// ---------------------------------------------------------------------------

#[test]
fn the_eight_minute_rule_matches_the_hydraulic_calculation() {
    // 9.10: 82 persons/m/min over 8 minutes.
    // 300 ÷ (82 × 8) = 0.4573 m of egress width required.
    let f = pack("green-guide").evaluate(&hall());
    let r = find(&f, "greenGuide.eight-minute");
    let want = 300.0 / (82.0 * 8.0);
    assert!(
        (r.limit.unwrap() - want).abs() < 1e-9,
        "{} (wanted {want:.6})",
        r.working
    );
    assert_eq!(r.status, Status::Pass);
}

#[test]
fn a_ground_whose_exits_cannot_pass_its_crowd_fails() {
    // 20,000 people through 2 m of exit: 20000 ÷ (82 × 8) = 30.49 m required.
    let facts = Facts {
        occupancy: 20_000,
        total_exit_width_m: 2.0,
        ..hall()
    };
    let f = pack("green-guide").evaluate(&facts);
    let r = find(&f, "greenGuide.eight-minute");
    assert_eq!(r.status, Status::Fail, "{}", r.working);
    assert!((r.limit.unwrap() - 20_000.0 / (82.0 * 8.0)).abs() < 1e-9);
}

#[test]
fn the_crush_threshold_is_six_persons_per_square_metre() {
    let mut facts = hall();
    facts.peak_density = Some(6.5);
    let f = pack("green-guide").evaluate(&facts);
    assert_eq!(find(&f, "greenGuide.crush-density").status, Status::Fail);
}

// ---------------------------------------------------------------------------
// Not assessed is not a pass
// ---------------------------------------------------------------------------

#[test]
fn an_unsimulated_venue_is_not_assessed_rather_than_passed() {
    // A venue nobody has run has no egress time and no peak density. Reporting
    // those as compliant is the single most dangerous thing this crate could
    // do: it turns "we did not check" into "we checked and it was fine".
    let facts = Facts {
        egress_time_s: None,
        peak_density: None,
        travel_distance_m: None,
        ..hall()
    };

    for pack_name in ["nfpa101", "green-guide"] {
        for f in pack(pack_name).evaluate(&facts) {
            match f.rule_id.as_str() {
                "greenGuide.rate-of-passage"
                | "greenGuide.crush-density"
                | "nfpa101.travel-distance" => {
                    assert_eq!(
                        f.status,
                        Status::NotAssessed,
                        "{} reported {:?} with no data: {}",
                        f.rule_id,
                        f.status,
                        f.working
                    );
                }
                _ => assert_ne!(f.status, Status::NotAssessed, "{}", f.rule_id),
            }
        }
    }
}

#[test]
fn an_undrawn_venue_is_not_assessed_rather_than_failed() {
    // Zero floor area gives an occupant load of zero, and comparing against it
    // would fail every rule for a building that has simply not been drawn yet.
    // A blank canvas is not a non-compliant venue.
    let f = pack("nfpa101").evaluate(&Facts::default());
    let r = find(&f, "nfpa101.occupant-load");
    assert_eq!(r.status, Status::NotAssessed, "{}", r.working);
}

// ---------------------------------------------------------------------------
// The pack itself
// ---------------------------------------------------------------------------

#[test]
fn every_shipped_pack_declares_it_has_not_been_reviewed() {
    // docs/06-validation.md requires external review by someone with
    // fire-engineering knowledge. None has happened. A pack that looks
    // authoritative without it is worse than one that is obviously
    // provisional, because the first one gets relied on.
    for name in ["nfpa101", "green-guide"] {
        let p = pack(name);
        assert!(
            !p.reviewed_by_fire_engineer,
            "{name} claims review that has not happened"
        );
        assert!(!p.source.is_empty(), "{name} does not cite its source");
        for r in &p.rules {
            assert!(!r.clause.is_empty(), "{}: no clause reference", r.id);
            assert!(!r.note.is_empty(), "{}: no note explaining a failure", r.id);
        }
    }
}

#[test]
fn every_finding_shows_its_working() {
    // A compliance figure a reader cannot reproduce is one they must take on
    // trust, and these documents do not get taken on trust.
    for name in ["nfpa101", "green-guide"] {
        for f in pack(name).evaluate(&hall()) {
            assert!(!f.working.is_empty(), "{}: no working", f.rule_id);
            if f.status != Status::NotAssessed {
                assert!(
                    f.working.contains("requirement:"),
                    "{}: working does not state the requirement: {}",
                    f.rule_id,
                    f.working
                );
            }
        }
    }
}

#[test]
fn findings_come_back_in_clause_order() {
    // A reviewer reads a standard in clause order and expects to find the
    // clauses where the standard puts them. Sorting failures to the top would
    // be more convenient and would stop it being a document you can follow.
    let p = pack("nfpa101");
    let ids: Vec<&str> = p.rules.iter().map(|r| r.id.as_str()).collect();
    let out: Vec<String> = p.evaluate(&hall()).into_iter().map(|f| f.rule_id).collect();
    assert_eq!(ids, out.iter().map(|s| s.as_str()).collect::<Vec<_>>());
}

#[test]
fn a_pack_round_trips_through_json_with_camel_case_fields() {
    // Guards the trap above. `rename_all` on an enum renames variants, not
    // their fields, so `m2PerPerson` is only camelCase because each struct
    // variant carries its own attribute. Serialising and reading back catches
    // a regression here; reading the struct definition does not.
    let p = pack("nfpa101");
    let text = serde_json::to_string(&p).expect("serialises");

    assert!(
        text.contains("m2PerPerson"),
        "field is not camelCase: {text}"
    );
    assert!(text.contains("reviewedByFireEngineer"));
    assert!(
        !text.contains("m2_per_person"),
        "snake_case leaked onto the wire: {text}"
    );

    let back = RulePack::from_json(&text).expect("reads back");
    assert_eq!(back, p);
}

#[test]
fn an_unknown_limit_kind_is_an_error_not_a_default() {
    // A pack from a future version naming a limit this build cannot evaluate
    // must fail loudly. Falling back to something reasonable would apply a
    // rule the author did not write.
    let bad = r#"{
        "id": "x", "name": "x", "source": "x",
        "rules": [{
            "id": "r", "clause": "c", "title": "t",
            "subject": "exitWidth", "compare": "atLeast",
            "limit": { "kind": "somethingNewer", "value": 1.0 }
        }]
    }"#;
    assert!(RulePack::from_json(bad).is_err());
}
