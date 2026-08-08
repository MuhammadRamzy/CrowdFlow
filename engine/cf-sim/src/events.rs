//! What happened, and when.
//!
//! A dossier full of totals says how a venue performed. It does not say what
//! *happened* — when the first person got out, when the crowd peaked, when a
//! door shut, whether the physics misbehaved and at what moment. A reviewer
//! reconstructing an evacuation reads a timeline, and without one they are
//! left inferring the sequence from a handful of aggregates.
//!
//! # Recorded, not inferred
//!
//! Every entry here is written at the tick it happened. Nothing is reconstructed
//! afterwards from stored state, because the two disagree in exactly the cases
//! that matter: a peak that was later exceeded, a door that closed and reopened,
//! a warning whose cause has since cleared.
//!
//! # Bounded
//!
//! A 90-minute run at 20 Hz is 108,000 ticks, and an event per tick per agent
//! would be a memory leak with a nice name. Only things a person would want to
//! read about are recorded, and the milestone events fire once each.

/// Something worth telling a reviewer about.
///
/// No serde here. `cf-sim` deliberately carries none — it is why `cf-geom`'s
/// serde is a feature — so that the wasm bundle does not ship a JSON parser it
/// never uses. `cf-wasm` marshals these on the way out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// The first person left the venue.
    FirstDeparture,
    /// Half of those who would leave had left.
    HalfCleared,
    /// The venue emptied.
    LastDeparture,
    /// Crowd density passed a threshold worth noting, in tenths of a person
    /// per m² so the type stays `Eq` and the log stays comparable.
    DensityThreshold { tenths_per_m2: u32 },
    /// A doorway was shut by a scenario event.
    ExitClosed { exit: usize },
    /// The alarm sounded and everyone was sent to an exit.
    AlarmSounded { rerouted: u32 },
    /// Agents were found outside the mesh and put back.
    ///
    /// Should never happen. It is logged rather than merely counted because a
    /// leak at 400 s and a leak at 4 s have different causes, and the count
    /// alone cannot tell them apart.
    AgentsRecovered { count: u32 },
}

/// One entry in the log.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    /// Simulated seconds, derived from the tick count so it cannot drift.
    pub at_s: f64,
    pub kind: EventKind,
}

/// A run's timeline.
#[derive(Clone, Debug, Default)]
pub struct EventLog {
    entries: Vec<Event>,
    /// Highest density threshold already announced, so a crowd hovering around
    /// a boundary does not fill the log with the same crossing.
    density_high_water: u32,
    seen_first: bool,
    seen_half: bool,
    seen_last: bool,
}

/// Densities worth announcing, persons/m².
///
/// The crowd-science bands, not round numbers: 2 is where the engine's own
/// validated envelope ends (ADR 0007), 4 is where movement becomes difficult,
/// and 6 is where forward progress ceases and crush risk begins.
const THRESHOLDS: [f64; 3] = [2.0, 4.0, 6.0];

impl EventLog {
    pub fn entries(&self) -> &[Event] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn push(&mut self, at_s: f64, kind: EventKind) {
        self.entries.push(Event { at_s, kind });
    }

    /// Note a departure. `exited` is the running total, `expected` how many
    /// were ever going to leave.
    ///
    /// "Half cleared" is measured against the number who *will* leave rather
    /// than the number spawned, because agents that were never placed cannot
    /// clear and counting them would move the halfway mark somewhere nobody
    /// crossed.
    pub fn on_departure(&mut self, at_s: f64, exited: u32, expected: u32) {
        if !self.seen_first && exited >= 1 {
            self.seen_first = true;
            self.push(at_s, EventKind::FirstDeparture);
        }
        if !self.seen_half && expected > 0 && exited * 2 >= expected {
            self.seen_half = true;
            self.push(at_s, EventKind::HalfCleared);
        }
    }

    /// Note that the venue emptied. Fires once.
    pub fn on_empty(&mut self, at_s: f64) {
        if !self.seen_last {
            self.seen_last = true;
            self.push(at_s, EventKind::LastDeparture);
        }
    }

    /// Note the current peak density, announcing any band newly crossed.
    ///
    /// Only ever upward. A crowd hovering at a boundary would otherwise
    /// announce the same crossing every few ticks, and a log that repeats
    /// itself is one nobody reads to the end.
    pub fn on_density(&mut self, at_s: f64, peak: f64) {
        for t in THRESHOLDS {
            let tenths = (t * 10.0).round() as u32;
            if peak >= t && self.density_high_water < tenths {
                self.density_high_water = tenths;
                self.push(
                    at_s,
                    EventKind::DensityThreshold {
                        tenths_per_m2: tenths,
                    },
                );
            }
        }
    }

    pub fn on_exit_closed(&mut self, at_s: f64, exit: usize) {
        self.push(at_s, EventKind::ExitClosed { exit });
    }

    pub fn on_alarm(&mut self, at_s: f64, rerouted: u32) {
        self.push(at_s, EventKind::AlarmSounded { rerouted });
    }

    /// Note agents recovered from off the mesh.
    ///
    /// Coalesced into the previous entry when it is the same event continuing,
    /// because a physics leak lasts many ticks and one line per tick would bury
    /// everything else in the log.
    pub fn on_recovered(&mut self, at_s: f64, count: u32) {
        if count == 0 {
            return;
        }
        if let Some(last) = self.entries.last_mut() {
            if let EventKind::AgentsRecovered { count: prev } = last.kind {
                last.kind = EventKind::AgentsRecovered {
                    count: prev + count,
                };
                return;
            }
        }
        self.push(at_s, EventKind::AgentsRecovered { count });
    }
}
