//! # cf-schema
//!
//! The canonical data contract for CrowdFlow Studio.
//!
//! This crate is the **single source of truth** for the Venue, Scenario and Run
//! document formats. The JSON Schema files in `/schema`, the TypeScript types in
//! `/web/src/schema` and the Pydantic models in `/services/api` are all
//! *generated* from the types defined here — see `cargo run -p cf-schema --bin
//! gen-schema`.
//!
//! Changing anything in this crate changes the contract between Track A and
//! Track B. CI gate G1 enforces that generated artifacts are regenerated and
//! committed in the same change (docs/01-architecture.md §6).
//!
//! ## Layout
//!
//! - [`geom`] — `Vec2`, `Polyline`, `Polygon`, `Aabb`, `Transform`
//! - [`dist`] — probability distributions with RNG-free inverse-CDF sampling
//! - [`ids`] — typed, string-backed identifiers
//! - [`venue`] — the authored venue document
//! - [`scenario`] — populations, arrivals, itineraries, events
//! - [`validate`] — structural and referential validation
//!
//! ## Example
//!
//! ```
//! use cf_schema::{VenueDoc, validate_venue};
//!
//! let venue = VenueDoc::empty("vnu_demo", "Demo Hall");
//! let report = validate_venue(&venue);
//! assert!(report.is_ok());
//! ```

pub mod dist;
pub mod geom;
pub mod ids;
pub mod scenario;
pub mod validate;
pub mod venue;

pub use dist::Distribution;
pub use geom::{Aabb, Polygon, Polyline, Transform, Vec2};
pub use ids::*;
pub use scenario::{ScenarioDoc, SimMode, SCENARIO_SCHEMA_VERSION};
pub use validate::{validate_scenario, validate_venue, Issue, Report, Severity};
pub use venue::{VenueDoc, VENUE_SCHEMA_VERSION};

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("json error in {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("document failed validation:\n{0}")]
    Invalid(Report),
}

/// Load and validate a venue document.
pub fn load_venue(path: impl AsRef<Path>) -> Result<VenueDoc, Error> {
    let path = path.as_ref();
    let display = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: display.clone(),
        source,
    })?;
    let doc: VenueDoc = serde_json::from_str(&text).map_err(|source| Error::Json {
        path: display,
        source,
    })?;
    let report = validate_venue(&doc);
    if report.has_errors() {
        return Err(Error::Invalid(report));
    }
    Ok(doc)
}

/// Serialise a venue document as pretty-printed JSON.
///
/// Uses two-space indentation so that committed fixtures produce readable diffs
/// — these files are reviewed by humans in pull requests.
pub fn to_pretty_json<T: serde::Serialize>(value: &T) -> Result<String, serde_json::Error> {
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b"  ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    value.serialize(&mut ser)?;
    let mut s = String::from_utf8(buf).expect("serde_json emits valid utf-8");
    s.push('\n');
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_venue_is_valid() {
        let v = VenueDoc::empty("vnu_test", "Test");
        let r = validate_venue(&v);
        assert!(r.is_ok(), "{r}");
    }

    #[test]
    fn empty_venue_round_trips() {
        let v = VenueDoc::empty("vnu_test", "Test");
        let json = to_pretty_json(&v).unwrap();
        let back: VenueDoc = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn wrong_schema_version_is_an_error() {
        let mut v = VenueDoc::empty("vnu_test", "Test");
        v.schema_version = "cfs.venue/999.0".into();
        let r = validate_venue(&v);
        assert!(r.has_errors());
        assert!(r.errors().any(|i| i.code == "venue.schema_version"));
    }
}
