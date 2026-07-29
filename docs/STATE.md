# STATE — living handoff note

> **Both sessions read this first and update it last.** Keep it short and current.
> If it disagrees with your assumptions, it wins. Stale entries are worse than none —
> delete rather than accumulate.

---

## Right now

**Phase:** B1 — Geometry, navmesh & compiler (started; target W3–W9)
**Last updated:** 2026-07-29 by Ramzy's session
**Tree status:** green — 58 tests passing, clippy clean, fmt clean, wasm32 builds

### Just finished

- **`cf-geom` crate** — the bottom layer of the engine (ADR 0003):
  - `primitives` — `Vec2`/`Polyline`/`Polygon`/`Aabb`/`Transform`, **moved here from
    `cf-schema`** so `cf-sim` can use geometry without depending on document types.
    `cf-schema` now depends on `cf-geom` and re-exports them, so its public API is unchanged.
  - `predicates` — exact orientation and in-circle via Shewchuk adaptive precision
    (`robust` crate). **Never compute an orientation by hand** — see the module docs for why.
  - `segment` — intersection with every degenerate case classified explicitly
    (shared endpoint, T-junction, collinear overlap, zero-length), plus closest-point and
    distance queries.
  - `polygon_ops` — winding, convexity, `validate` returning all defects, and exact point
    location that reports `Boundary` as a distinct answer.
- Serde/schemars behind a default-on `serde` feature so `cf-sim`'s wasm bundle can drop them.
  CI now builds `--no-default-features` on native **and** wasm32.
- ADR 0003.

### Next up — pick from the top

1. **`cf-geom`: polygon offsetting** — needed to turn wall centrelines + `thicknessM` into
   obstacle polygons for the navmesh. Start with the simple convex-ish case (offset each
   edge, intersect adjacent offset lines, handle the reflex case by adding a bevel).
   Miter-limit handling matters: a sharp wall corner offsets to a spike.
2. **`cf-navmesh`: constrained Delaunay triangulation.** The big one, and the real critical
   path to M1. Suggested slice: unconstrained Bowyer–Watson first with the exact predicates
   already in `cf-geom`, tested against a known triangulation; *then* add constraint edge
   insertion. Do not try to write both at once.
3. **Codegen: TypeScript types** from `schema/*.json` into `web/src/schema/`
   (`json-schema-to-typescript`), wired into gate G1. Small; unblocks Track A.
4. **Codegen: Pydantic models** (`datamodel-code-generator`). Small.

Item 2 is the critical path. Items 3 and 4 are each ~an hour and unblock the other track,
so they are good picks for a short session.

### Open questions

- **Repo visibility.** `docs/07-infrastructure-and-cost.md` §4.4 recommends `engine/` public
  (free CI, supports the papers) but filing provisionals **before** any public push. The repo
  is currently public. **Both of you + VIT's IP office should confirm this is what you want**
  — public disclosure can start or forfeit filing windows.

### Gotchas discovered — don't rediscover these

- `#[serde(rename_all)]` on an **enum** renames variants, **not** their fields. Struct
  variants with multi-word fields (e.g. `Lognormal { mu_ln }`) need their own
  `#[serde(rename_all = "camelCase")]`. Guarded by `every_schema_property_is_camel_case`.
- `Distribution::sample_icdf(0.0)` on an unbounded Normal used to return `-inf`. Uniform
  PRNGs do emit exactly 0.0, so this would have injected NaN agents. Now clamped away from
  **both** endpoints (`U_EPS`).
- When adding a `serde`-gated type to `cf-geom`, **field-level** attributes need
  `cfg_attr` too, not just the derive. `#[serde(default)]` on a field is easy to miss and
  only breaks the `--no-default-features` build. CI catches it now.
- `schemars` is pinned at 0.8 (1.x has a different API). If you upgrade, the manual
  `#[schemars(with = ...)]` on `Vec2` needs revisiting.
- Clippy rejects methods named `add`/`sub` on a type (shadows `std::ops`). `Vec2` implements
  the real operator traits — use `a + b`, `a - b`, `v * k`.
- Acklam's coefficients in `dist.rs` carry a digit more than f64 holds. The
  `excessive_precision` lint is allowed there deliberately so the constants stay checkable
  against the published algorithm. Don't "fix" it.

---

## How to update this file

At the end of a session, `/handoff` will prompt you through it. By hand:

1. Set **Phase**, **Last updated**, **Tree status**.
2. Replace **Just finished** with what *this* session did (not cumulative history — that's
   what `git log` is for).
3. Rewrite **Next up** as a short ranked list. Be specific enough that someone with zero
   context can start. "Continue the engine" is useless; "add `cf-geom::segment_intersect`
   with the degenerate-case tests from B1" is useful.
4. Add anything learned the hard way to **Gotchas**.
5. Commit and push.
