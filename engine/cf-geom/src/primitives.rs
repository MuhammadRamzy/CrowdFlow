//! Geometry primitives for the authored document.
//!
//! Authored geometry is `f64` throughout. Only the simulation's runtime arrays
//! drop to `f32` — constructing a navmesh for a large venue in `f32` produces
//! degenerate triangles (see docs/04-track-b-simulation-engine.md §B1).
//!
//! `Vec2` serialises as a two-element array `[x, y]` rather than
//! `{"x":…,"y":…}`. For a venue with tens of thousands of vertices that is
//! roughly a 3x reduction in document size, and it matches how every CAD
//! interchange format writes coordinates.

#[cfg(feature = "serde")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A point or vector in floor-local metres.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn dot(self, o: Vec2) -> f64 {
        self.x * o.x + self.y * o.y
    }

    /// Z-component of the 3D cross product; positive when `o` is counter-clockwise of `self`.
    pub fn cross(self, o: Vec2) -> f64 {
        self.x * o.y - self.y * o.x
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    pub fn distance(self, o: Vec2) -> f64 {
        (self - o).length()
    }

    pub fn lerp(self, o: Vec2, t: f64) -> Vec2 {
        Vec2::new(self.x + (o.x - self.x) * t, self.y + (o.y - self.y) * t)
    }

    /// Unit vector, or `None` for a degenerate (zero-length) vector.
    pub fn normalized(self) -> Option<Vec2> {
        let len = self.length();
        if len > f64::EPSILON {
            Some(self * (1.0 / len))
        } else {
            None
        }
    }

    /// Rotated counter-clockwise by 90°. The outward normal of a wall segment.
    pub fn perp(self) -> Vec2 {
        Vec2::new(-self.y, self.x)
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl std::ops::Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}

impl std::ops::Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}

/// Scalar scaling: `v * k`.
impl std::ops::Mul<f64> for Vec2 {
    type Output = Vec2;
    fn mul(self, k: f64) -> Vec2 {
        Vec2::new(self.x * k, self.y * k)
    }
}

/// Scalar scaling the other way round: `k * v`.
impl std::ops::Mul<Vec2> for f64 {
    type Output = Vec2;
    fn mul(self, v: Vec2) -> Vec2 {
        v * self
    }
}

impl std::ops::Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

impl std::ops::AddAssign for Vec2 {
    fn add_assign(&mut self, o: Vec2) {
        self.x += o.x;
        self.y += o.y;
    }
}

impl std::ops::SubAssign for Vec2 {
    fn sub_assign(&mut self, o: Vec2) {
        self.x -= o.x;
        self.y -= o.y;
    }
}

#[cfg(feature = "serde")]
impl Serialize for Vec2 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeTuple;
        let mut t = s.serialize_tuple(2)?;
        t.serialize_element(&self.x)?;
        t.serialize_element(&self.y)?;
        t.end()
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for Vec2 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let [x, y] = <[f64; 2]>::deserialize(d)?;
        Ok(Vec2 { x, y })
    }
}

/// Hand-written so the schema matches what `Serialize` actually produces.
///
/// `#[schemars(with = "[f64; 2]")]` at the container level is silently ignored
/// by the derive — it generated an object `{x, y}` while serde wrote an array
/// `[x, y]`. Nothing failed: the Rust round-tripped fine, and the divergence
/// only surfaced when the generated TypeScript was first used to build a
/// document the engine then rejected. Delegating explicitly cannot drift.
#[cfg(feature = "serde")]
impl JsonSchema for Vec2 {
    fn schema_name() -> String {
        "Vec2".to_owned()
    }

    fn json_schema(gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = <[f64; 2]>::json_schema(gen);
        if let schemars::schema::Schema::Object(o) = &mut schema {
            o.metadata().description = Some("A 2D point in metres, as [x, y].".to_owned());
        }
        schema
    }
}

impl From<[f64; 2]> for Vec2 {
    fn from(a: [f64; 2]) -> Self {
        Vec2::new(a[0], a[1])
    }
}

/// An open chain of points. Walls are polylines, not single segments, so that a
/// run of collinear-ish wall survives import as one editable entity.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize, JsonSchema))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Polyline(pub Vec<Vec2>);

impl Polyline {
    pub fn points(&self) -> &[Vec2] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Total arc length in metres.
    pub fn length(&self) -> f64 {
        self.0.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    /// Point at normalised arc-length parameter `t` in `[0, 1]`.
    ///
    /// This is how an [`crate::venue::Opening`] resolves its position: openings
    /// are stored parametrically so that moving or re-snapping a wall carries
    /// its doors along automatically.
    pub fn point_at(&self, t: f64) -> Option<Vec2> {
        if self.0.is_empty() {
            return None;
        }
        if self.0.len() == 1 {
            return Some(self.0[0]);
        }
        let total = self.length();
        if total <= f64::EPSILON {
            return Some(self.0[0]);
        }
        let target = t.clamp(0.0, 1.0) * total;
        let mut walked = 0.0;
        for w in self.0.windows(2) {
            let seg = w[0].distance(w[1]);
            if walked + seg >= target || seg <= f64::EPSILON {
                let local = if seg > f64::EPSILON {
                    (target - walked) / seg
                } else {
                    0.0
                };
                return Some(w[0].lerp(w[1], local));
            }
            walked += seg;
        }
        self.0.last().copied()
    }

    /// Unit tangent at normalised parameter `t`, used to orient door swings and
    /// to compute the opening's perpendicular.
    pub fn tangent_at(&self, t: f64) -> Option<Vec2> {
        if self.0.len() < 2 {
            return None;
        }
        let total = self.length();
        if total <= f64::EPSILON {
            return None;
        }
        let target = t.clamp(0.0, 1.0) * total;
        let mut walked = 0.0;
        for w in self.0.windows(2) {
            let seg = w[0].distance(w[1]);
            if walked + seg >= target {
                return (w[1] - w[0]).normalized();
            }
            walked += seg;
        }
        (self.0[self.0.len() - 1] - self.0[self.0.len() - 2]).normalized()
    }

    pub fn is_closed(&self, tol: f64) -> bool {
        match (self.0.first(), self.0.last()) {
            (Some(a), Some(b)) => self.0.len() > 2 && a.distance(*b) <= tol,
            _ => false,
        }
    }
}

/// An implicitly-closed ring. The last point is *not* repeated.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize, JsonSchema))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Polygon(pub Vec<Vec2>);

impl Polygon {
    pub fn points(&self) -> &[Vec2] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Signed area; positive when the ring winds counter-clockwise.
    pub fn signed_area(&self) -> f64 {
        let n = self.0.len();
        if n < 3 {
            return 0.0;
        }
        let mut acc = 0.0;
        for i in 0..n {
            let a = self.0[i];
            let b = self.0[(i + 1) % n];
            acc += a.cross(b);
        }
        acc * 0.5
    }

    /// Unsigned area in m². This is the number NFPA occupant load is computed from,
    /// so it is deliberately independent of winding order.
    pub fn area(&self) -> f64 {
        self.signed_area().abs()
    }

    pub fn perimeter(&self) -> f64 {
        let n = self.0.len();
        if n < 2 {
            return 0.0;
        }
        (0..n)
            .map(|i| self.0[i].distance(self.0[(i + 1) % n]))
            .sum()
    }

    pub fn centroid(&self) -> Option<Vec2> {
        let n = self.0.len();
        if n == 0 {
            return None;
        }
        let a = self.signed_area();
        if a.abs() <= f64::EPSILON {
            // Degenerate ring: fall back to the vertex mean.
            let sum = self.0.iter().fold(Vec2::ZERO, |acc, p| acc + *p);
            return Some(sum * (1.0 / n as f64));
        }
        let mut cx = 0.0;
        let mut cy = 0.0;
        for i in 0..n {
            let p = self.0[i];
            let q = self.0[(i + 1) % n];
            let cr = p.cross(q);
            cx += (p.x + q.x) * cr;
            cy += (p.y + q.y) * cr;
        }
        Some(Vec2::new(cx / (6.0 * a), cy / (6.0 * a)))
    }

    pub fn contains(&self, p: Vec2) -> bool {
        let n = self.0.len();
        if n < 3 {
            return false;
        }
        let mut inside = false;
        let mut j = n - 1;
        for i in 0..n {
            let a = self.0[i];
            let b = self.0[j];
            if (a.y > p.y) != (b.y > p.y) {
                let t = (p.y - a.y) / (b.y - a.y);
                if p.x < a.x + t * (b.x - a.x) {
                    inside = !inside;
                }
            }
            j = i;
        }
        inside
    }

    pub fn bounds(&self) -> Option<Aabb> {
        Aabb::of(self.0.iter().copied())
    }
}

/// Axis-aligned bounding box.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize, JsonSchema))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Aabb {
    pub min: Vec2,
    pub max: Vec2,
}

impl Aabb {
    pub fn of(points: impl IntoIterator<Item = Vec2>) -> Option<Aabb> {
        let mut it = points.into_iter();
        let first = it.next()?;
        let mut bb = Aabb {
            min: first,
            max: first,
        };
        for p in it {
            bb.min.x = bb.min.x.min(p.x);
            bb.min.y = bb.min.y.min(p.y);
            bb.max.x = bb.max.x.max(p.x);
            bb.max.y = bb.max.y.max(p.y);
        }
        Some(bb)
    }

    pub fn width(&self) -> f64 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f64 {
        self.max.y - self.min.y
    }

    pub fn union(self, o: Aabb) -> Aabb {
        Aabb {
            min: Vec2::new(self.min.x.min(o.min.x), self.min.y.min(o.min.y)),
            max: Vec2::new(self.max.x.max(o.max.x), self.max.y.max(o.max.y)),
        }
    }
}

/// Placement of a component on a floor.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize, JsonSchema))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Transform {
    /// Origin in floor-local metres.
    pub p: Vec2,
    /// Rotation in degrees, counter-clockwise from +x.
    #[cfg_attr(feature = "serde", serde(default))]
    pub rot_deg: f64,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            p: Vec2::ZERO,
            rot_deg: 0.0,
        }
    }
}
