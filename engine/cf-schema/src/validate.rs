//! Structural and referential validation.
//!
//! This is *schema-level* validation only: do the ids referenced actually
//! exist, are the numbers physically possible, are required per-type params
//! present. It deliberately does **not** check anything requiring geometry
//! processing — "is this room reachable", "is this mesh watertight" are
//! `cf-compile`'s job and surface as `CompileWarning`s.
//!
//! Splitting it this way means the editor can validate on every keystroke
//! without triangulating, and only pays for a compile when geometry changes.

use crate::scenario::*;
use crate::venue::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// The document cannot be compiled or simulated.
    Error,
    /// Simulable, but the result is likely not what the author intended.
    Warning,
    /// Worth surfacing in the validation panel, harmless to ignore.
    Info,
}

/// One validation finding, addressed to a specific element so the editor can
/// pan to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub severity: Severity,
    /// Stable machine-readable code, e.g. `opening.orphan_wall`.
    pub code: String,
    /// Id of the offending element, if there is one.
    pub element: Option<String>,
    pub message: String,
}

impl Issue {
    fn new(severity: Severity, code: &str, element: Option<String>, message: String) -> Self {
        Self {
            severity,
            code: code.to_owned(),
            element,
            message,
        }
    }

    fn error(code: &str, element: impl Into<String>, message: String) -> Self {
        Self::new(Severity::Error, code, Some(element.into()), message)
    }

    fn warn(code: &str, element: impl Into<String>, message: String) -> Self {
        Self::new(Severity::Warning, code, Some(element.into()), message)
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.element {
            Some(e) => write!(
                f,
                "[{:?}] {} ({}): {}",
                self.severity, self.code, e, self.message
            ),
            None => write!(f, "[{:?}] {}: {}", self.severity, self.code, self.message),
        }
    }
}

/// Collected findings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub issues: Vec<Issue>,
}

impl Report {
    pub fn is_ok(&self) -> bool {
        !self.has_errors()
    }

    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &Issue> {
        self.issues.iter().filter(|i| i.severity == Severity::Error)
    }

    pub fn count(&self, s: Severity) -> usize {
        self.issues.iter().filter(|i| i.severity == s).count()
    }

    fn push(&mut self, i: Issue) {
        self.issues.push(i);
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.issues.is_empty() {
            return write!(f, "no issues");
        }
        for i in &self.issues {
            writeln!(f, "{i}")?;
        }
        Ok(())
    }
}

/// Minimum clear width for an accessible egress door, in metres.
/// NFPA 101 requires 32 in (0.813 m) clear; we warn below 0.85 m to leave
/// margin for door hardware.
const MIN_EGRESS_WIDTH_M: f64 = 0.85;

/// Validate a venue document.
pub fn validate_venue(v: &VenueDoc) -> Report {
    let mut r = Report::default();

    if v.schema_version != VENUE_SCHEMA_VERSION {
        r.push(Issue::new(
            Severity::Error,
            "venue.schema_version",
            None,
            format!(
                "expected schemaVersion '{}', got '{}'",
                VENUE_SCHEMA_VERSION, v.schema_version
            ),
        ));
    }

    if v.floors.is_empty() {
        r.push(Issue::new(
            Severity::Error,
            "venue.no_floors",
            None,
            "venue has no floors".into(),
        ));
    }

    if let Some(scale) = &v.scale {
        if scale.source_px_per_meter <= 0.0 {
            r.push(Issue::new(
                Severity::Error,
                "venue.bad_scale",
                None,
                "scale.sourcePxPerMeter must be > 0".into(),
            ));
        }
        if !scale.confirmed {
            // A wrong scale silently invalidates every compliance number, so an
            // unconfirmed one blocks committing rather than merely warning.
            r.push(Issue::new(
                Severity::Warning,
                "venue.scale_unconfirmed",
                None,
                "imported scale has not been confirmed by a human".into(),
            ));
        }
    }

    let layer_ids: HashSet<_> = v.layers.iter().map(|l| l.id.clone()).collect();
    let floor_ids: HashSet<_> = v.floors.iter().map(|f| f.id.clone()).collect();
    let mut seen_floors = HashSet::new();

    for f in &v.floors {
        if !seen_floors.insert(f.id.clone()) {
            r.push(Issue::error(
                "floor.duplicate_id",
                f.id.as_str(),
                format!("duplicate floor id '{}'", f.id),
            ));
        }
        validate_floor(f, &layer_ids, &mut r);
    }

    // Vertical links
    for l in &v.links {
        if l.ends.len() != 2 {
            r.push(Issue::error(
                "link.bad_ends",
                l.id.as_str(),
                format!("link must have exactly 2 ends, has {}", l.ends.len()),
            ));
        }
        for e in &l.ends {
            if !floor_ids.contains(&e.floor) {
                r.push(Issue::error(
                    "link.orphan_floor",
                    l.id.as_str(),
                    format!("references unknown floor '{}'", e.floor),
                ));
            }
            if e.footprint.len() < 3 {
                r.push(Issue::error(
                    "link.degenerate_footprint",
                    l.id.as_str(),
                    format!("footprint on floor '{}' has < 3 points", e.floor),
                ));
            }
        }
        if l.ends.len() == 2 && l.ends[0].floor == l.ends[1].floor {
            r.push(Issue::warn(
                "link.same_floor",
                l.id.as_str(),
                "both ends are on the same floor".into(),
            ));
        }
        if l.width_m <= 0.0 {
            r.push(Issue::error(
                "link.bad_width",
                l.id.as_str(),
                format!("widthM must be > 0 (got {})", l.width_m),
            ));
        }
        if let Some(cw) = l.clear_width_m {
            if cw > l.width_m {
                r.push(Issue::warn(
                    "link.clear_width_exceeds_width",
                    l.id.as_str(),
                    format!("clearWidthM ({cw}) > widthM ({})", l.width_m),
                ));
            }
        }
        if matches!(l.kind, LinkKind::Stair) && l.flow_rate_ppmm.is_none() {
            r.push(Issue::new(
                Severity::Info,
                "link.no_flow_rate",
                Some(l.id.to_string()),
                "stair has no flowRatePpmm; Green Guide default of 66 will be applied".into(),
            ));
        }
    }

    // Routing graph
    let wp_ids: HashSet<_> = v.routing.waypoints.iter().map(|w| w.id.clone()).collect();
    for w in &v.routing.waypoints {
        if !floor_ids.contains(&w.floor) {
            r.push(Issue::error(
                "waypoint.orphan_floor",
                w.id.as_str(),
                format!("references unknown floor '{}'", w.floor),
            ));
        }
    }
    for e in &v.routing.edges {
        if !wp_ids.contains(&e.from) {
            r.push(Issue::error(
                "edge.orphan_waypoint",
                e.from.as_str(),
                format!("edge references unknown waypoint '{}'", e.from),
            ));
        }
        if !wp_ids.contains(&e.to) {
            r.push(Issue::error(
                "edge.orphan_waypoint",
                e.to.as_str(),
                format!("edge references unknown waypoint '{}'", e.to),
            ));
        }
    }

    let all_zone_ids: HashSet<_> = v
        .floors
        .iter()
        .flat_map(|f| f.zones.iter().map(|z| z.id.clone()))
        .collect();
    for fc in &v.routing.flow_constraints {
        if !all_zone_ids.contains(&fc.zone) {
            r.push(Issue::error(
                "flow_constraint.orphan_zone",
                fc.zone.as_str(),
                format!("references unknown zone '{}'", fc.zone),
            ));
        }
        if !(0.0..=1.0).contains(&fc.strength) {
            r.push(Issue::error(
                "flow_constraint.bad_strength",
                fc.zone.as_str(),
                format!("strength must be in [0,1] (got {})", fc.strength),
            ));
        }
        if matches!(fc.kind, FlowConstraintKind::OneWay) && fc.heading_deg.is_none() {
            r.push(Issue::error(
                "flow_constraint.no_heading",
                fc.zone.as_str(),
                "one-way constraint requires headingDeg".into(),
            ));
        }
    }

    r
}

fn validate_floor(f: &Floor, layer_ids: &HashSet<crate::ids::LayerId>, r: &mut Report) {
    let wall_ids: HashSet<_> = f.walls.iter().map(|w| w.id.clone()).collect();
    let zone_ids: HashSet<_> = f.zones.iter().map(|z| z.id.clone()).collect();

    let check_layer = |layer: &Option<crate::ids::LayerId>, elem: &str, r: &mut Report| {
        if let Some(l) = layer {
            if !layer_ids.contains(l) {
                r.push(Issue::warn(
                    "element.orphan_layer",
                    elem,
                    format!("references unknown layer '{l}'"),
                ));
            }
        }
    };

    for w in &f.walls {
        check_layer(&w.layer, w.id.as_str(), r);
        if w.polyline.len() < 2 {
            r.push(Issue::error(
                "wall.degenerate",
                w.id.as_str(),
                format!("polyline has {} point(s), needs >= 2", w.polyline.len()),
            ));
        }
        if !w.polyline.points().iter().all(|p| p.is_finite()) {
            r.push(Issue::error(
                "wall.non_finite",
                w.id.as_str(),
                "polyline contains non-finite coordinates".into(),
            ));
        }
        if w.thickness_m <= 0.0 {
            r.push(Issue::error(
                "wall.bad_thickness",
                w.id.as_str(),
                format!("thicknessM must be > 0 (got {})", w.thickness_m),
            ));
        }
        if w.polyline.length() < 0.01 {
            r.push(Issue::warn(
                "wall.near_zero_length",
                w.id.as_str(),
                format!("wall is only {:.4} m long", w.polyline.length()),
            ));
        }
    }

    for op in &f.openings {
        if !wall_ids.contains(&op.wall) {
            r.push(Issue::error(
                "opening.orphan_wall",
                op.id.as_str(),
                format!("references unknown wall '{}'", op.wall),
            ));
            continue;
        }
        if !(0.0..=1.0).contains(&op.t) {
            r.push(Issue::error(
                "opening.t_out_of_range",
                op.id.as_str(),
                format!("t must be in [0,1] (got {})", op.t),
            ));
        }
        if op.width_m <= 0.0 {
            r.push(Issue::error(
                "opening.bad_width",
                op.id.as_str(),
                format!("widthM must be > 0 (got {})", op.width_m),
            ));
        } else if op.width_m < MIN_EGRESS_WIDTH_M && op.is_fire_exit {
            r.push(Issue::warn(
                "opening.narrow_fire_exit",
                op.id.as_str(),
                format!(
                    "fire exit is {:.2} m wide, below the {:.2} m minimum clear width",
                    op.width_m, MIN_EGRESS_WIDTH_M
                ),
            ));
        }
        // An opening cannot be wider than the wall it sits in.
        if let Some(w) = f.wall(&op.wall) {
            let wall_len = w.polyline.length();
            if op.width_m > wall_len && wall_len > 0.0 {
                r.push(Issue::error(
                    "opening.wider_than_wall",
                    op.id.as_str(),
                    format!(
                        "opening is {:.2} m wide but wall '{}' is only {:.2} m long",
                        op.width_m, op.wall, wall_len
                    ),
                ));
            }
        }
        if op.capacity_factor <= 0.0 {
            r.push(Issue::error(
                "opening.bad_capacity_factor",
                op.id.as_str(),
                format!("capacityFactor must be > 0 (got {})", op.capacity_factor),
            ));
        }
        validate_schedule(&op.schedule, op.id.as_str(), r);
    }

    for z in &f.zones {
        check_layer(&z.layer, z.id.as_str(), r);
        if z.polygon.len() < 3 {
            r.push(Issue::error(
                "zone.degenerate",
                z.id.as_str(),
                format!("polygon has {} point(s), needs >= 3", z.polygon.len()),
            ));
            continue;
        }
        if z.polygon.area() < 0.01 {
            r.push(Issue::warn(
                "zone.near_zero_area",
                z.id.as_str(),
                format!("zone area is only {:.4} m²", z.polygon.area()),
            ));
        }
        if let Some(olf) = z.olf_override {
            if olf <= 0.0 {
                r.push(Issue::error(
                    "zone.bad_olf",
                    z.id.as_str(),
                    format!("olfOverride must be > 0 (got {olf})"),
                ));
            }
            if z.olf_justification.is_none() {
                r.push(Issue::warn(
                    "zone.unjustified_olf",
                    z.id.as_str(),
                    "olfOverride set without olfJustification; an auditor will ask".into(),
                ));
            }
        }
        if z.speed_multiplier <= 0.0 {
            r.push(Issue::error(
                "zone.bad_speed_multiplier",
                z.id.as_str(),
                format!("speedMultiplier must be > 0 (got {})", z.speed_multiplier),
            ));
        }
    }

    for o in &f.obstacles {
        check_layer(&o.layer, o.id.as_str(), r);
        if o.polygon.len() < 3 {
            r.push(Issue::error(
                "obstacle.degenerate",
                o.id.as_str(),
                format!("polygon has {} point(s), needs >= 3", o.polygon.len()),
            ));
        }
    }

    for c in &f.components {
        check_layer(&c.layer, c.id.as_str(), r);
        if let Some(q) = &c.queue_area {
            if !zone_ids.contains(q) {
                r.push(Issue::error(
                    "component.orphan_queue_area",
                    c.id.as_str(),
                    format!("queueArea references unknown zone '{q}'"),
                ));
            }
        }
        validate_component_params(c, r);
        validate_schedule(&c.schedule, c.id.as_str(), r);
    }
}

/// Per-type required parameters. Kept here rather than in the type system so the
/// editor can change a component's type without discarding compatible params.
fn validate_component_params(c: &Component, r: &mut Report) {
    let p = &c.params;
    let id = c.id.as_str();

    if let Some(d) = &p.service_time {
        if let Err(e) = d.validate() {
            r.push(Issue::error(
                "component.bad_service_time",
                id,
                format!("serviceTime: {e}"),
            ));
        }
    }
    if let Some(d) = &p.secondary_time {
        if let Err(e) = d.validate() {
            r.push(Issue::error(
                "component.bad_secondary_time",
                id,
                format!("secondaryTime: {e}"),
            ));
        }
    }
    if let Some(rate) = p.secondary_rate {
        if !(0.0..=1.0).contains(&rate) {
            r.push(Issue::error(
                "component.bad_secondary_rate",
                id,
                format!("secondaryRate must be in [0,1] (got {rate})"),
            ));
        }
    }
    if let Some(t) = p.max_throughput_pph {
        if t <= 0.0 {
            r.push(Issue::error(
                "component.bad_throughput",
                id,
                format!("maxThroughputPph must be > 0 (got {t})"),
            ));
        }
    }
    if let Some(l) = p.lanes {
        if l == 0 {
            r.push(Issue::error(
                "component.zero_lanes",
                id,
                "lanes must be >= 1".into(),
            ));
        }
    }

    use ComponentType::*;
    match c.component_type {
        Turnstile => {
            if p.lanes.is_none() {
                r.push(Issue::error(
                    "component.missing_param",
                    id,
                    "turnstile requires 'lanes'".into(),
                ));
            }
            if p.service_time.is_none() && p.max_throughput_pph.is_none() {
                r.push(Issue::error(
                    "component.missing_param",
                    id,
                    "turnstile requires 'serviceTime' or 'maxThroughputPph'".into(),
                ));
            }
        }
        SecurityCheckpoint | RegistrationDesk | TicketCounter => {
            if p.service_time.is_none() {
                r.push(Issue::error(
                    "component.missing_param",
                    id,
                    format!("{:?} requires 'serviceTime'", c.component_type),
                ));
            }
            if p.lanes.is_none() {
                r.push(Issue::warn(
                    "component.missing_param",
                    id,
                    format!("{:?} has no 'lanes'; assuming 1 server", c.component_type),
                ));
            }
            if c.queue_area.is_none() {
                r.push(Issue::warn(
                    "component.no_queue_area",
                    id,
                    "no queueArea bound; agents will queue in free space".into(),
                ));
            }
        }
        SeatingBlock => {
            if p.rows.is_none() || p.cols.is_none() {
                r.push(Issue::error(
                    "component.missing_param",
                    id,
                    "seatingBlock requires 'rows' and 'cols'".into(),
                ));
            }
            if p.seat_pitch_m.is_none() || p.row_pitch_m.is_none() {
                r.push(Issue::warn(
                    "component.missing_param",
                    id,
                    "seatingBlock has no seatPitchM/rowPitchM; defaults will be applied".into(),
                ));
            }
        }
        Barricade | Stall => {
            if p.footprint.is_none() {
                r.push(Issue::error(
                    "component.missing_param",
                    id,
                    format!("{:?} requires 'footprint'", c.component_type),
                ));
            }
        }
        Sign => {
            if p.heading_deg.is_none() {
                r.push(Issue::error(
                    "component.missing_param",
                    id,
                    "sign requires 'headingDeg'".into(),
                ));
            }
            if let Some(cr) = p.compliance_rate {
                if !(0.0..=1.0).contains(&cr) {
                    r.push(Issue::error(
                        "component.bad_compliance_rate",
                        id,
                        format!("complianceRate must be in [0,1] (got {cr})"),
                    ));
                }
            }
        }
    }
}

fn validate_schedule(s: &[ScheduleEntry], elem: &str, r: &mut Report) {
    let mut prev = f64::NEG_INFINITY;
    for e in s {
        if !e.from_s.is_finite() || e.from_s < 0.0 {
            r.push(Issue::error(
                "schedule.bad_time",
                elem,
                format!("fromS must be finite and >= 0 (got {})", e.from_s),
            ));
        }
        if e.from_s < prev {
            r.push(Issue::error(
                "schedule.out_of_order",
                elem,
                format!(
                    "schedule entries must be sorted by fromS ({} after {prev})",
                    e.from_s
                ),
            ));
        }
        prev = e.from_s;
    }
}

/// Validate a scenario against the venue it targets.
pub fn validate_scenario(s: &ScenarioDoc, v: &VenueDoc) -> Report {
    let mut r = Report::default();

    if s.schema_version != SCENARIO_SCHEMA_VERSION {
        r.push(Issue::new(
            Severity::Error,
            "scenario.schema_version",
            None,
            format!(
                "expected schemaVersion '{}', got '{}'",
                SCENARIO_SCHEMA_VERSION, s.schema_version
            ),
        ));
    }
    if s.duration_s <= 0.0 {
        r.push(Issue::new(
            Severity::Error,
            "scenario.bad_duration",
            None,
            format!("durationS must be > 0 (got {})", s.duration_s),
        ));
    }
    if s.timestep_s <= 0.0 || s.timestep_s > 0.5 {
        r.push(Issue::new(
            Severity::Error,
            "scenario.bad_timestep",
            None,
            format!(
                "timestepS must be in (0, 0.5] for a stable contact solve (got {})",
                s.timestep_s
            ),
        ));
    }
    if s.populations.is_empty() {
        r.push(Issue::new(
            Severity::Warning,
            "scenario.no_populations",
            None,
            "scenario has no populations; nothing will be simulated".into(),
        ));
    }

    let opening_ids: HashSet<_> = v
        .floors
        .iter()
        .flat_map(|f| f.openings.iter().map(|o| o.id.clone()))
        .collect();
    let zone_ids: HashSet<_> = v
        .floors
        .iter()
        .flat_map(|f| f.zones.iter().map(|z| z.id.clone()))
        .collect();
    let comp_ids: HashSet<_> = v
        .floors
        .iter()
        .flat_map(|f| f.components.iter().map(|c| c.id.clone()))
        .collect();
    let wp_ids: HashSet<_> = v.routing.waypoints.iter().map(|w| w.id.clone()).collect();
    let link_ids: HashSet<_> = v.links.iter().map(|l| l.id.clone()).collect();

    for p in &s.populations {
        let pid = p.id.as_str();
        if p.count == 0 {
            r.push(Issue::warn(
                "population.zero_count",
                pid,
                "count is 0".into(),
            ));
        }
        for (name, d) in [
            ("desiredSpeed", Some(&p.profile.desired_speed)),
            ("radiusM", Some(&p.profile.radius_m)),
            ("massKg", p.profile.mass_kg.as_ref()),
            ("groupSize", p.profile.group_size.as_ref()),
            ("patienceS", p.profile.patience_s.as_ref()),
            ("reactionTimeS", p.profile.reaction_time_s.as_ref()),
        ] {
            if let Some(d) = d {
                if let Err(e) = d.validate() {
                    r.push(Issue::error(
                        "population.bad_distribution",
                        pid,
                        format!("{name}: {e}"),
                    ));
                }
            }
        }
        if !(0.0..=1.0).contains(&p.profile.familiarity) {
            r.push(Issue::error(
                "population.bad_familiarity",
                pid,
                format!(
                    "familiarity must be in [0,1] (got {})",
                    p.profile.familiarity
                ),
            ));
        }
        if !(0.0..=1.0).contains(&p.profile.mobility_impaired_frac) {
            r.push(Issue::error(
                "population.bad_impaired_frac",
                pid,
                format!(
                    "mobilityImpairedFrac must be in [0,1] (got {})",
                    p.profile.mobility_impaired_frac
                ),
            ));
        }

        match &p.arrival {
            Arrival::Curve { points, entries } => {
                validate_arrival_curve(points, pid, &mut r);
                validate_entries(entries, &opening_ids, pid, &mut r);
            }
            Arrival::Uniform { entries } => validate_entries(entries, &opening_ids, pid, &mut r),
            Arrival::Preplaced { zones } => {
                if zones.is_empty() {
                    r.push(Issue::error(
                        "arrival.no_zones",
                        pid,
                        "preplaced arrival has no zones".into(),
                    ));
                }
                for z in zones {
                    if !zone_ids.contains(&z.zone) {
                        r.push(Issue::error(
                            "arrival.orphan_zone",
                            pid,
                            format!("references unknown zone '{}'", z.zone),
                        ));
                    }
                }
            }
        }

        for (i, step) in p.itinerary.iter().enumerate() {
            if !(0.0..=1.0).contains(&step.probability) {
                r.push(Issue::error(
                    "itinerary.bad_probability",
                    pid,
                    format!(
                        "step {i}: probability must be in [0,1] (got {})",
                        step.probability
                    ),
                ));
            }
            if let Some(d) = &step.dwell {
                if let Err(e) = d.validate() {
                    r.push(Issue::error(
                        "itinerary.bad_dwell",
                        pid,
                        format!("step {i}: dwell: {e}"),
                    ));
                }
            }
            let missing = match &step.goal {
                Goal::Zone { id } => (!zone_ids.contains(id)).then(|| format!("zone '{id}'")),
                Goal::Component { id } => {
                    (!comp_ids.contains(id)).then(|| format!("component '{id}'"))
                }
                Goal::Waypoint { id } => (!wp_ids.contains(id)).then(|| format!("waypoint '{id}'")),
                Goal::Opening { id } => {
                    (!opening_ids.contains(id)).then(|| format!("opening '{id}'"))
                }
                Goal::NearestExit => None,
            };
            if let Some(what) = missing {
                r.push(Issue::error(
                    "itinerary.orphan_goal",
                    pid,
                    format!("step {i}: references unknown {what}"),
                ));
            }
        }
    }

    let mut prev_t = f64::NEG_INFINITY;
    for e in &s.events {
        if e.at_s < 0.0 || e.at_s > s.duration_s {
            r.push(Issue::new(
                Severity::Warning,
                "event.outside_duration",
                None,
                format!("event at t={} is outside [0, {}]", e.at_s, s.duration_s),
            ));
        }
        if e.at_s < prev_t {
            r.push(Issue::new(
                Severity::Warning,
                "event.out_of_order",
                None,
                format!("events should be sorted by atS ({} after {prev_t})", e.at_s),
            ));
        }
        prev_t = e.at_s;

        let missing = match &e.event {
            EventKind::CloseOpening { target } | EventKind::OpenOpening { target } => {
                (!opening_ids.contains(target)).then(|| format!("opening '{target}'"))
            }
            EventKind::BlockLink { target } | EventKind::UnblockLink { target } => {
                (!link_ids.contains(target)).then(|| format!("link '{target}'"))
            }
            EventKind::CloseComponent { target } | EventKind::OpenComponent { target } => {
                (!comp_ids.contains(target)).then(|| format!("component '{target}'"))
            }
            EventKind::Alarm { .. } => None,
        };
        if let Some(what) = missing {
            r.push(Issue::new(
                Severity::Error,
                "event.orphan_target",
                None,
                format!("event at t={} references unknown {what}", e.at_s),
            ));
        }
    }

    if matches!(s.mode, SimMode::Evacuation)
        && !s
            .events
            .iter()
            .any(|e| matches!(e.event, EventKind::Alarm { .. }))
    {
        r.push(Issue::new(
            Severity::Warning,
            "scenario.evacuation_without_alarm",
            None,
            "evacuation mode with no alarm event; egress will never be triggered".into(),
        ));
    }

    if s.output.density_grid_m <= 0.0 {
        r.push(Issue::new(
            Severity::Error,
            "output.bad_grid",
            None,
            "densityGridM must be > 0".into(),
        ));
    }
    if !(0.0..=1.0).contains(&s.output.trajectory_sample_rate) {
        r.push(Issue::new(
            Severity::Error,
            "output.bad_sample_rate",
            None,
            "trajectorySampleRate must be in [0,1]".into(),
        ));
    }

    r
}

fn validate_arrival_curve(points: &[[f64; 2]], pid: &str, r: &mut Report) {
    if points.len() < 2 {
        r.push(Issue::error(
            "arrival.short_curve",
            pid,
            "arrival curve needs at least 2 points".into(),
        ));
        return;
    }
    let mut prev_t = f64::NEG_INFINITY;
    let mut prev_f = f64::NEG_INFINITY;
    for (i, [t, frac]) in points.iter().enumerate() {
        if !t.is_finite() || !frac.is_finite() {
            r.push(Issue::error(
                "arrival.non_finite",
                pid,
                format!("point {i} is non-finite"),
            ));
            return;
        }
        if *t < prev_t {
            r.push(Issue::error(
                "arrival.non_monotonic_time",
                pid,
                format!("point {i}: time {t} < previous {prev_t}"),
            ));
        }
        if *frac < prev_f {
            r.push(Issue::error(
                "arrival.non_monotonic_fraction",
                pid,
                format!("point {i}: cumulative fraction {frac} < previous {prev_f}"),
            ));
        }
        if !(0.0..=1.0).contains(frac) {
            r.push(Issue::error(
                "arrival.fraction_out_of_range",
                pid,
                format!("point {i}: cumulative fraction {frac} outside [0,1]"),
            ));
        }
        prev_t = *t;
        prev_f = *frac;
    }
    if let Some([_, last]) = points.last() {
        if (last - 1.0).abs() > 1e-6 {
            r.push(Issue::warn(
                "arrival.incomplete_curve",
                pid,
                format!("curve ends at cumulative fraction {last}, not 1.0; some agents will never spawn"),
            ));
        }
    }
}

fn validate_entries(
    entries: &[EntryWeight],
    opening_ids: &HashSet<crate::ids::OpeningId>,
    pid: &str,
    r: &mut Report,
) {
    if entries.is_empty() {
        r.push(Issue::error(
            "arrival.no_entries",
            pid,
            "no entry openings specified".into(),
        ));
        return;
    }
    let total: f64 = entries.iter().map(|e| e.weight).sum();
    if total <= 0.0 {
        r.push(Issue::error(
            "arrival.zero_weights",
            pid,
            "entry weights sum to 0".into(),
        ));
    }
    for e in entries {
        if !opening_ids.contains(&e.opening) {
            r.push(Issue::error(
                "arrival.orphan_opening",
                pid,
                format!("references unknown opening '{}'", e.opening),
            ));
        }
        if e.weight < 0.0 {
            r.push(Issue::error(
                "arrival.negative_weight",
                pid,
                format!("negative weight on opening '{}'", e.opening),
            ));
        }
    }
}
