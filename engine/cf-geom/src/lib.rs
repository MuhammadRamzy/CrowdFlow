//! # cf-geom
//!
//! Geometry primitives and exact predicates for CrowdFlow Studio.
//!
//! This is the lowest layer of the engine. It deliberately knows nothing about
//! venues, documents or simulation — so `cf-sim`, which ships to browsers as
//! wasm, can depend on it without dragging in the document schema. `cf-schema`
//! re-exports the primitives from here, so there is exactly one `Vec2` in the
//! project.
//!
//! ## Layout
//!
//! - [`primitives`] — `Vec2`, `Polyline`, `Polygon`, `Aabb`, `Transform`
//! - [`predicates`] — exact orientation and in-circle tests
//! - [`segment`] — intersection, distance and closest-point queries
//! - [`polygon_ops`] — winding, convexity, validity, point location
//!
//! ## The rule
//!
//! **Never compute an orientation, a side test, or a crossing test by hand.**
//! Use [`predicates::orient`] and the routines built on it. The reasoning is in
//! the [`predicates`] module documentation; the short version is that naive
//! floating-point orientation returns the wrong *sign* for nearly-collinear
//! input, architectural drawings are full of nearly-collinear input, and the
//! resulting corrupt triangulation surfaces far away from its cause.
//!
//! ## Units and precision
//!
//! All coordinates are metres, `f64`. The simulation's runtime arrays drop to
//! `f32` for cache density, but geometry *construction* stays `f64` — building
//! a navmesh for a 400 m venue in `f32` yields degenerate triangles.
//!
//! ## Example
//!
//! ```
//! use cf_geom::{Vec2, Polygon, polygon_ops::{contains_point, is_valid}};
//!
//! let hall = Polygon(vec![
//!     Vec2::new(0.0, 0.0),
//!     Vec2::new(20.0, 0.0),
//!     Vec2::new(20.0, 12.0),
//!     Vec2::new(0.0, 12.0),
//! ]);
//!
//! assert!(is_valid(&hall));
//! assert!(contains_point(&hall, Vec2::new(10.0, 6.0)));
//! ```

pub mod polygon_ops;
pub mod predicates;
pub mod primitives;
pub mod segment;

pub use polygon_ops::{contains_point, locate_point, PointLocation, Winding};
pub use predicates::{collinear, in_circle, orient, Orientation};
pub use primitives::{Aabb, Polygon, Polyline, Transform, Vec2};
pub use segment::{segment_distance, Intersection, Segment};

/// Distance below which two coordinates are treated as the same point during
/// import cleanup, in metres.
///
/// 1 mm. Chosen because architectural drawings are dimensioned in millimetres,
/// so anything finer is scanning noise rather than intent. This is a *cleanup*
/// tolerance — it is never used inside a predicate, where exactness is the
/// whole point.
pub const SNAP_EPSILON_M: f64 = 0.001;

/// Angular tolerance for treating two directions as parallel, in degrees.
///
/// Used by import cleanup when merging collinear wall runs.
pub const PARALLEL_EPSILON_DEG: f64 = 2.0;
