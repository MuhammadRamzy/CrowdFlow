//! Life-safety rules, as data rather than code.
//!
//! # Why this is a rule *pack* and not a function
//!
//! The rules a venue is judged against depend on where it is and what it is.
//! NFPA 101 governs in the United States, the Green Guide at UK sports grounds,
//! NBC in India, NFPA 130 on rail platforms — and each is revised on its own
//! schedule. Written as code, adding a jurisdiction means a release; written as
//! data, it means a file, and a fire engineer can read the file.
//!
//! That last point is the whole argument. `docs/06-validation.md` requires
//! **external review of every rule by someone with fire-engineering
//! knowledge**, and nobody outside this project is going to review a `match`
//! arm in Rust. A pack is a table of clause references, thresholds and units.
//!
//! # Facts, not simulations
//!
//! A rule evaluates [`Facts`] — an area, a width, a count, a time. It never
//! touches the engine. That is deliberate: a pack can be checked against
//! hand-worked figures from the standard itself with no simulation in the
//! picture, which is what makes the check meaningful rather than circular.
//!
//! # Every finding shows its working
//!
//! A compliance figure a reader cannot reproduce is a figure they have to take
//! on trust, and life-safety documents do not get taken on trust. Every
//! [`Finding`] carries the arithmetic that produced it in the form the standard
//! states it.

use serde::{Deserialize, Serialize};

/// Everything a rule may ask about a venue and its run.
///
/// Optional fields are genuinely unknown rather than zero — a venue that has
/// not been simulated has no egress time, and a rule that needs one reports
/// [`Status::NotAssessed`] instead of judging it against nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Facts {
    /// Walkable floor, m².
    pub walkable_area_m2: f64,
    /// People to be evacuated. The peak, not the instantaneous count.
    pub occupancy: u32,
    /// Doorways counted as means of egress.
    pub exit_count: u32,
    /// Sum of clear widths of those doorways, metres.
    pub total_exit_width_m: f64,
    /// The narrowest of them, metres.
    pub narrowest_exit_m: f64,
    /// Seconds for the venue to clear, if it has been simulated.
    pub egress_time_s: Option<f64>,
    /// Highest crowd density reached, persons/m², if simulated.
    pub peak_density: Option<f64>,
    /// Longest distance anyone must walk to reach an exit, metres.
    pub travel_distance_m: Option<f64>,
}

/// What a rule measures.
///
/// A closed set rather than an expression language. An expression language
/// would be more flexible and would also mean a rule pack could compute
/// anything at all, which is exactly what a reviewer cannot check by reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Subject {
    /// Occupancy against the load the floor area permits.
    OccupantLoad,
    /// Number of separate means of egress.
    ExitCount,
    /// Total clear egress width, metres.
    ExitWidth,
    /// Clear width of the narrowest exit, metres.
    NarrowestExit,
    /// Time to clear the venue, seconds.
    EgressTime,
    /// Peak crowd density, persons/m².
    PeakDensity,
    /// Longest travel distance to an exit, metres.
    TravelDistance,
}

/// How the measured value is compared against the limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Compare {
    /// The measured value must not exceed the limit.
    AtMost,
    /// The measured value must reach the limit.
    AtLeast,
}

/// Where the limit comes from.
/// # A trap this walked straight into
///
/// `rename_all` on an **enum** renames its variants, not their fields — a
/// gotcha already recorded in `docs/STATE.md` from the last time it bit this
/// repo. Each struct variant needs its own attribute, or `m2_per_person`
/// stays snake_case on the wire while every other type in the project is
/// camelCase, and the pack fails to parse with a message about a field nobody
/// wrote.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Limit {
    /// A fixed figure the standard states outright.
    Fixed { value: f64 },
    /// Floor area divided by an occupant load factor, m² per person.
    ///
    /// NFPA 101 Table 7.3.1.2 and its equivalents. The *limit* is a person
    /// count derived from the venue, not a constant.
    #[serde(rename_all = "camelCase")]
    AreaPerPerson { m2_per_person: f64 },
    /// A width derived from occupancy, metres per person.
    ///
    /// The capacity method: 5 mm per person for level egress in NFPA 101, and
    /// the Green Guide's equivalent expressed as a rate of passage.
    #[serde(rename_all = "camelCase")]
    WidthPerPerson { m_per_person: f64 },
    /// Occupancy divided by a rate of passage, persons per metre per minute.
    ///
    /// The Green Guide's hydraulic form: a venue must clear in the time its
    /// exits can pass its crowd.
    #[serde(rename_all = "camelCase")]
    FlowRate {
        persons_per_m_per_min: f64,
        within_s: f64,
    },
}

/// One rule from one standard.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    /// The clause this comes from, verbatim enough to look up.
    pub clause: String,
    pub title: String,
    pub subject: Subject,
    pub compare: Compare,
    pub limit: Limit,
    /// What a reader should understand by a failure. Shown beside the finding.
    #[serde(default)]
    pub note: String,
}

/// A set of rules from one standard.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RulePack {
    pub id: String,
    pub name: String,
    /// The document and edition, so a finding can be traced to a source.
    pub source: String,
    /// Whether a qualified fire engineer has checked this pack.
    ///
    /// **False on every pack shipped so far**, and the report says so. A pack
    /// that looks authoritative without that review is worse than one that is
    /// obviously provisional: the first gets relied on.
    #[serde(default)]
    pub reviewed_by_fire_engineer: bool,
    pub rules: Vec<Rule>,
}

/// Whether a rule passed, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Pass,
    Fail,
    /// The facts needed are not available — usually because the venue has not
    /// been simulated. Distinct from a pass, and must never be shown as one.
    NotAssessed,
}

/// The outcome of one rule against one venue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub rule_id: String,
    pub clause: String,
    pub title: String,
    pub status: Status,
    /// What was measured.
    pub measured: Option<f64>,
    /// What it had to be.
    pub limit: Option<f64>,
    /// The arithmetic, in the form the standard states it.
    ///
    /// A compliance figure a reader cannot reproduce is one they must take on
    /// trust, and these documents do not get taken on trust.
    pub working: String,
    pub note: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("rule pack is not valid JSON: {0}")]
    Parse(#[from] serde_json::Error),
}

impl RulePack {
    /// Load a pack from JSON.
    pub fn from_json(text: &str) -> Result<Self, PackError> {
        Ok(serde_json::from_str(text)?)
    }

    /// Evaluate every rule against a venue.
    ///
    /// Findings come back in pack order, not sorted by severity. A reviewer
    /// reads a standard in clause order and expects to find the clauses where
    /// the standard puts them; sorting failures to the top would break that,
    /// and the caller can sort if it wants to.
    pub fn evaluate(&self, facts: &Facts) -> Vec<Finding> {
        self.rules.iter().map(|r| evaluate(r, facts)).collect()
    }
}

/// Evaluate one rule.
pub fn evaluate(rule: &Rule, facts: &Facts) -> Finding {
    let Some((measured, measured_label)) = measure(rule.subject, facts) else {
        return Finding {
            rule_id: rule.id.clone(),
            clause: rule.clause.clone(),
            title: rule.title.clone(),
            status: Status::NotAssessed,
            measured: None,
            limit: None,
            working: format!(
                "not assessed — {} is not known for this venue",
                subject_name(rule.subject)
            ),
            note: rule.note.clone(),
        };
    };

    let Some((limit, limit_working)) = resolve_limit(&rule.limit, facts) else {
        return Finding {
            rule_id: rule.id.clone(),
            clause: rule.clause.clone(),
            title: rule.title.clone(),
            status: Status::NotAssessed,
            measured: Some(measured),
            limit: None,
            working: "not assessed — the limit could not be derived from this venue".into(),
            note: rule.note.clone(),
        };
    };

    // A limit of zero is not a limit. It arises from a venue with no floor area
    // or no occupancy, and comparing against it would fail every rule for a
    // building that has simply not been drawn yet.
    if limit <= 0.0 {
        return Finding {
            rule_id: rule.id.clone(),
            clause: rule.clause.clone(),
            title: rule.title.clone(),
            status: Status::NotAssessed,
            measured: Some(measured),
            limit: None,
            working: format!("not assessed — {limit_working} gives no usable limit"),
            note: rule.note.clone(),
        };
    }

    let pass = match rule.compare {
        Compare::AtMost => measured <= limit,
        Compare::AtLeast => measured >= limit,
    };

    Finding {
        rule_id: rule.id.clone(),
        clause: rule.clause.clone(),
        title: rule.title.clone(),
        status: if pass { Status::Pass } else { Status::Fail },
        measured: Some(measured),
        limit: Some(limit),
        working: format!(
            "{measured_label} = {measured:.4} {unit}; {limit_working} = {limit:.4} {unit}; \
             requirement: {cmp} {limit:.4}",
            unit = subject_unit(rule.subject),
            cmp = match rule.compare {
                Compare::AtMost => "at most",
                Compare::AtLeast => "at least",
            },
        ),
        note: rule.note.clone(),
    }
}

/// What the venue actually is, for this subject.
fn measure(subject: Subject, f: &Facts) -> Option<(f64, String)> {
    match subject {
        Subject::OccupantLoad => Some((f.occupancy as f64, "occupancy".into())),
        Subject::ExitCount => Some((f.exit_count as f64, "exits provided".into())),
        Subject::ExitWidth => Some((f.total_exit_width_m, "total clear egress width".into())),
        Subject::NarrowestExit => Some((f.narrowest_exit_m, "narrowest exit clear width".into())),
        Subject::EgressTime => f.egress_time_s.map(|t| (t, "time to clear".into())),
        Subject::PeakDensity => f.peak_density.map(|d| (d, "peak crowd density".into())),
        Subject::TravelDistance => f
            .travel_distance_m
            .map(|d| (d, "longest travel distance".into())),
    }
}

/// The threshold, and the arithmetic that produced it.
fn resolve_limit(limit: &Limit, f: &Facts) -> Option<(f64, String)> {
    match *limit {
        Limit::Fixed { value } => Some((value, format!("limit stated as {value:.4}"))),

        Limit::AreaPerPerson { m2_per_person } => {
            if m2_per_person <= 0.0 || f.walkable_area_m2 <= 0.0 {
                return None;
            }
            // Rounded down: 0.9 of a person is not a person, and rounding up
            // licenses a venue for someone it has no room for.
            let load = (f.walkable_area_m2 / m2_per_person).floor();
            Some((
                load,
                format!(
                    "{:.2} m² ÷ {m2_per_person:.2} m²/person, rounded down",
                    f.walkable_area_m2
                ),
            ))
        }

        Limit::WidthPerPerson { m_per_person } => {
            if m_per_person <= 0.0 {
                return None;
            }
            let need = f.occupancy as f64 * m_per_person;
            Some((
                need,
                format!("{} persons × {m_per_person:.4} m/person", f.occupancy),
            ))
        }

        Limit::FlowRate {
            persons_per_m_per_min,
            within_s,
        } => {
            if persons_per_m_per_min <= 0.0 || within_s <= 0.0 {
                return None;
            }
            // The width needed to pass this crowd in the allowed time.
            let minutes = within_s / 60.0;
            let need = f.occupancy as f64 / (persons_per_m_per_min * minutes);
            Some((
                need,
                format!(
                    "{} persons ÷ ({persons_per_m_per_min:.1} p/m/min × {minutes:.2} min)",
                    f.occupancy
                ),
            ))
        }
    }
}

fn subject_name(s: Subject) -> &'static str {
    match s {
        Subject::OccupantLoad => "occupancy",
        Subject::ExitCount => "the number of exits",
        Subject::ExitWidth => "total egress width",
        Subject::NarrowestExit => "the narrowest exit",
        Subject::EgressTime => "egress time",
        Subject::PeakDensity => "peak density",
        Subject::TravelDistance => "travel distance",
    }
}

fn subject_unit(s: Subject) -> &'static str {
    match s {
        Subject::OccupantLoad | Subject::ExitCount => "persons",
        Subject::ExitWidth | Subject::NarrowestExit | Subject::TravelDistance => "m",
        Subject::EgressTime => "s",
        Subject::PeakDensity => "persons/m²",
    }
}
