# 0003 — Geometry is the bottom layer, and its predicates are exact

**Status:** accepted · **Date:** 2026-07-29 · **Session:** Ramzy

## Context

Two questions came up at the start of phase B1, and they turned out to be linked.

**Where do geometry primitives live?** `cf-schema` originally defined `Vec2`, `Polyline`
and `Polygon` alongside the document types. But `cf-sim` needs `Vec2` and does *not* need
`VenueDoc` — it consumes a compiled `NavGraph`. Leaving the primitives in `cf-schema` would
have made every simulation crate depend on the document schema, and would have pulled
`serde` and `schemars` into a wasm binary that ships to browsers.

**How are orientation tests computed?** The constrained Delaunay triangulation in
`cf-navmesh` rests entirely on the orientation and in-circle predicates. The naive
`(b-a).cross(c-a) > 0.0` returns the *wrong sign* when its rounding error exceeds its own
magnitude — which happens precisely for nearly-collinear input. Architectural drawings are
full of nearly-collinear input, because buildings are full of parallel walls that almost
line up.

## Decision

**Layering.** `cf-geom` is the bottom crate. It defines the primitives and knows nothing
about venues, documents or simulation. `cf-schema` depends on it and re-exports the types,
so callers still see one coherent surface and there is exactly one `Vec2` in the project.
Serialisation is behind a default-on `serde` feature so `cf-sim` can drop `serde` and
`schemars` entirely.

**Predicates.** All orientation and in-circle tests go through Shewchuk's adaptive-precision
predicates via the `robust` crate (v1.2, permissively licensed). They apply a fast
floating-point filter first and escalate to exact arithmetic only when the error bound says
the sign is not yet certain, so the common case costs roughly what the naive version does.

Everything built on top — segment intersection, polygon validity, point location — uses
these rather than tolerance comparisons. `cf-geom`'s module docs state the rule directly:
never compute an orientation by hand.

## Consequences

**Easier:** the CDT can be written without defensive tolerance-tuning. Segment intersection
classifies degenerate cases (shared endpoints, T-junctions, collinear overlap) exactly
rather than approximately, which is what import topology repair actually needs — an
L-junction and a true crossing require different repairs. Point location reports `Boundary`
as a distinct answer instead of guessing.

**Harder:** two crates to keep in step, and the `serde` feature needs `cfg_attr` on every
derive and every field attribute. Missing one is a compile error only on the
`--no-default-features` path, so CI now builds that path explicitly on both native and
wasm32 — otherwise it would have stayed broken until `cf-sim` existed and surfaced as a
wall of errors.

**Cost:** exact predicates are slower than naive ones in the worst case. This is acceptable
because they run during *compile*, not in the simulation hot loop. If a profile ever shows
them dominating a navmesh build, the fix is fewer predicate calls, not less exact ones.

**Revisit if:** `robust` becomes unmaintained. The predicates are a well-defined, published
algorithm, so vendoring is a viable fallback.

## Alternatives considered

- **`cf-geom` depends on `cf-schema`.** Rejected: inverts the layering and drags document
  types into the simulation.
- **Duplicate `Vec2` in each crate.** Rejected: conversion boilerplate at every boundary,
  and two places for a bug to differ.
- **Naive predicates with an epsilon.** Rejected. Epsilon tolerances do not fix the sign
  problem — they relabel some wrong answers as "collinear", which produces a *different*
  malformed triangulation. There is no epsilon that makes an inexact determinant correct.
- **`geo`/`geos` crates.** Heavier than needed, and `geos` binds a C library, which
  complicates the wasm target. `robust` is one focused dependency.
