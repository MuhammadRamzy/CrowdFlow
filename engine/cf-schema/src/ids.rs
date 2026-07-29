//! Typed identifiers.
//!
//! Every entity id is a distinct Rust type but a plain JSON string on the wire.
//! This makes it a compile error to pass a `WallId` where an `OpeningId` is
//! expected — a class of bug that is otherwise very easy to write and very hard
//! to see, because both are `String`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
            Serialize, Deserialize, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }
    };
}

id_type!(
    /// A venue document.
    VenueId
);
id_type!(
    /// An immutable venue version (a node in the version DAG).
    VersionId
);
id_type!(
    /// A drawing/visibility layer.
    LayerId
);
id_type!(
    /// A floor within a venue.
    FloorId
);
id_type!(
    /// A wall polyline.
    WallId
);
id_type!(
    /// An opening (door, gate, gap) parameterised along a wall.
    OpeningId
);
id_type!(
    /// A semantic zone polygon.
    ZoneId
);
id_type!(
    /// A non-wall obstacle polygon.
    ObstacleId
);
id_type!(
    /// A placed intelligent component.
    ComponentId
);
id_type!(
    /// A vertical link between two floors.
    LinkId
);
id_type!(
    /// A routing-graph waypoint.
    WaypointId
);
id_type!(
    /// A canvas annotation.
    AnnotationId
);
id_type!(
    /// A scenario document.
    ScenarioId
);
id_type!(
    /// An agent population within a scenario.
    PopulationId
);
id_type!(
    /// A simulation run.
    RunId
);
