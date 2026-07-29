//! The Scenario Document — populations, arrival curves, itineraries, events.
//!
//! Deliberately separate from [`crate::venue::VenueDoc`] so that one venue
//! supports many scenarios (Optimal Operations / Peak Load / Evacuation) without
//! duplicating geometry. That split is what makes Core Capability 8's
//! "compare contingency plans side by side" cheap instead of a special feature.

use crate::dist::Distribution;
use crate::ids::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SCENARIO_SCHEMA_VERSION: &str = "cfs.scenario/1.0";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioDoc {
    pub schema_version: String,
    pub id: ScenarioId,
    pub name: String,
    /// The exact venue version this scenario was authored against.
    pub venue_version: VersionId,

    #[serde(default)]
    pub mode: SimMode,
    pub duration_s: f64,
    #[serde(default = "default_timestep")]
    pub timestep_s: f64,
    /// Master PRNG seed. Same seed + same inputs must produce a bit-identical
    /// run on every target — see docs/04-track-b-simulation-engine.md §5.
    pub seed: u64,

    pub populations: Vec<Population>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<TimedEvent>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compliance: Option<ComplianceConfig>,

    #[serde(default)]
    pub output: OutputConfig,
}

impl ScenarioDoc {
    pub fn total_agents(&self) -> u64 {
        self.populations.iter().map(|p| p.count as u64).sum()
    }
}

fn default_timestep() -> f64 {
    0.05
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SimMode {
    /// Normal operations under the authored arrival curve.
    #[default]
    EventFlow,
    /// Stress test at theoretical maximum occupancy.
    PeakLoad,
    /// Alarm-driven egress. Alters urgency, personal space, patience and
    /// wayfinding — see docs/04-track-b-simulation-engine.md §B4.
    Evacuation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Population {
    pub id: PopulationId,
    pub label: String,
    pub count: u32,
    pub profile: AgentProfile,
    pub arrival: Arrival,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub itinerary: Vec<ItineraryStep>,
    /// Access tags these agents carry, matched against zone and component rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub access: Vec<String>,
}

/// Per-agent parameter distributions.
///
/// Defaults are drawn from the pedestrian dynamics literature (Weidmann's
/// 1.34 m/s mean free walking speed, ~0.23 m shoulder radius) so that an
/// unconfigured population is already defensible rather than arbitrary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    #[serde(default = "default_speed")]
    pub desired_speed: Distribution,
    #[serde(default = "default_radius")]
    pub radius_m: Distribution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mass_kg: Option<Distribution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_size: Option<Distribution>,
    /// Seconds an agent will queue before considering an alternative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patience_s: Option<Distribution>,
    /// 0.0 = follows signage only, 1.0 = knows every route in the venue.
    #[serde(default = "default_familiarity")]
    pub familiarity: f64,
    #[serde(default)]
    pub mobility_impaired_frac: f64,
    /// Pre-movement delay after an alarm. Evacuation mode only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reaction_time_s: Option<Distribution>,
}

impl Default for AgentProfile {
    fn default() -> Self {
        Self {
            desired_speed: default_speed(),
            radius_m: default_radius(),
            mass_kg: None,
            group_size: None,
            patience_s: None,
            familiarity: default_familiarity(),
            mobility_impaired_frac: 0.0,
            reaction_time_s: None,
        }
    }
}

/// Weidmann (1993): mean free walking speed 1.34 m/s, sd 0.26.
fn default_speed() -> Distribution {
    Distribution::Normal {
        mean: 1.34,
        sd: 0.26,
        min: Some(0.60),
        max: Some(2.20),
    }
}

fn default_radius() -> Distribution {
    Distribution::Normal {
        mean: 0.23,
        sd: 0.02,
        min: Some(0.18),
        max: Some(0.30),
    }
}

fn default_familiarity() -> f64 {
    0.6
}

/// How agents enter the simulation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Arrival {
    /// Cumulative arrival curve: `points` are `(t_seconds, cumulative_fraction)`,
    /// monotonically non-decreasing in both, ending at fraction 1.0.
    Curve {
        points: Vec<[f64; 2]>,
        entries: Vec<EntryWeight>,
    },
    /// Constant rate over the whole duration.
    Uniform { entries: Vec<EntryWeight> },
    /// Everyone present at t=0, distributed across zones. The starting condition
    /// for an evacuation study.
    Preplaced { zones: Vec<ZoneWeight> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EntryWeight {
    pub opening: OpeningId,
    #[serde(default = "one")]
    pub weight: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZoneWeight {
    pub zone: ZoneId,
    #[serde(default = "one")]
    pub weight: f64,
}

fn one() -> f64 {
    1.0
}

/// One leg of an agent's plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItineraryStep {
    pub goal: Goal,
    /// Fraction of the population that performs this step. Defaults to all.
    #[serde(default = "one")]
    pub probability: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dwell: Option<Distribution>,
}

/// A destination.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "target", rename_all = "camelCase")]
pub enum Goal {
    Zone {
        id: ZoneId,
    },
    Component {
        id: ComponentId,
    },
    Waypoint {
        id: WaypointId,
    },
    Opening {
        id: OpeningId,
    },
    /// Nearest exit by traversable distance, recomputed if blocked.
    NearestExit,
}

/// A scheduled change to the world mid-run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TimedEvent {
    pub at_s: f64,
    #[serde(flatten)]
    pub event: EventKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EventKind {
    /// Trigger evacuation behaviour.
    #[serde(rename_all = "camelCase")]
    Alarm {
        #[serde(default)]
        scope: AlarmScope,
        #[serde(default)]
        egress_policy: EgressPolicy,
    },
    CloseOpening {
        target: OpeningId,
    },
    OpenOpening {
        target: OpeningId,
    },
    BlockLink {
        target: LinkId,
    },
    UnblockLink {
        target: LinkId,
    },
    CloseComponent {
        target: ComponentId,
    },
    OpenComponent {
        target: ComponentId,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AlarmScope {
    #[default]
    All,
    Floor {
        id: FloorId,
    },
    Zone {
        id: ZoneId,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum EgressPolicy {
    #[default]
    NearestAvailable,
    /// Use the exit the agent originally entered by — models the well-documented
    /// tendency to leave the way you came in, which usually worsens egress time.
    FamiliarRoute,
    AssignedExit,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceConfig {
    /// Rule packs to evaluate, e.g. `["NFPA101", "GreenGuide"]`.
    pub codes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_egress_s: Option<f64>,
    #[serde(default)]
    pub occupancy_basis: OccupancyBasis,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum OccupancyBasis {
    /// Use the peak occupancy the simulation actually produced.
    #[default]
    Simulated,
    /// Use the code-derived occupant load regardless of simulated population.
    Declared,
}

/// Controls the size and resolution of run artifacts.
///
/// Defaults are chosen so a 100k-agent 30-minute run produces well under 150 MB
/// rather than the ~1.4 GB that storing raw per-tick positions would cost
/// (docs/02-data-model.md §6).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutputConfig {
    /// Density/dwell grid cell size in metres.
    #[serde(default = "default_grid_m")]
    pub density_grid_m: f64,
    /// Temporal bucket for density accumulation, in seconds.
    #[serde(default = "default_bucket_s")]
    pub density_bucket_s: f64,
    /// Fraction of agents whose full trajectory is recorded, 0.0–1.0.
    #[serde(default = "default_traj_sample")]
    pub trajectory_sample_rate: f64,
    #[serde(default = "default_traj_hz")]
    pub trajectory_hz: f64,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            density_grid_m: default_grid_m(),
            density_bucket_s: default_bucket_s(),
            trajectory_sample_rate: default_traj_sample(),
            trajectory_hz: default_traj_hz(),
        }
    }
}

fn default_grid_m() -> f64 {
    0.5
}
fn default_bucket_s() -> f64 {
    5.0
}
fn default_traj_sample() -> f64 {
    0.02
}
fn default_traj_hz() -> f64 {
    2.0
}
