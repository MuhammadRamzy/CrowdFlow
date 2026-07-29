# STATE — living handoff note

> **Both sessions read this first and update it last.** Keep it short and current.
> If it disagrees with your assumptions, it wins. Stale entries are worse than none —
> delete rather than accumulate.

---

## Right now

**Phase:** B1 — Geometry, navmesh & compiler (target W3–W9). Geometry and navmesh are
done; `cf-compile` is the remaining piece of B1.
**Last updated:** 2026-07-29 by Ramzy's session
**Tree status:** green — 106 tests passing, clippy clean on all feature paths, wasm32 builds

### Just finished

`cf-navmesh` is complete enough to answer "shortest walkable route from A to B".
Four crates now: `cf-geom`, `cf-navmesh`, `cf-schema` (+ the planned `cf-compile`).

- **Constrained Delaunay triangulation** — Bowyer–Watson, then constraint insertion by
  carving crossed triangles and re-triangulating the two pseudo-polygons. Vertices lying
  exactly on a constraint split it (what a T-junction looks like after import).
- **Region classification** — crossing-count from the exterior; odd nesting depth is
  walkable. Detects and reports unclosed wall runs instead of guessing.
- **Portals** — shared edges between walkable triangles, each carrying its clear width.
- **Pathfinding** — A\* over the triangle dual for the corridor, funnel algorithm to pull
  it taut into the true shortest path.
- ADR 0004 records the pipeline and the two things it forced.

### Next up — pick from the top

1. **`cf-compile` crate: `VenueDoc` → `NavGraph`.** This is the last piece of B1 and it
   joins the schema work to the navmesh work. Sequence:
   - resolve each `Opening`'s parametric `t` to world coordinates on its parent wall
     (`Floor::opening_position` already does this)
   - offset each wall centreline by `thicknessM` into an obstacle ring
     (`cf_geom::offset_polyline_to_ring`)
   - collect points + constraint edges, dedupe coincident points — **`triangulate` errors
     on duplicates by design**, so dedupe is the caller's job
   - **seal every door opening with a virtual constraint edge before classifying**, then
     re-open them as portals afterward. See the Gotchas below; there is a test
     (`region::door_gaps_leak_until_sealed`) that exists to make this unmissable
   - emit `CompileWarning`s: unreachable zone, room with no exit, opening below minimum
     clear width, component with no queue area, disconnected mesh island
   - **Acceptance:** compile `fixtures/unit/hall-two-doors.venue.json` to a NavGraph with
     240 m² walkable and a path that exits through a door.
2. **Mesh refinement** — cap triangle area so the density grid has adequate resolution.
   Ruppert-style. Only needed once analytics land; not on the M1 path.
3. **Codegen: TypeScript types** from `schema/*.json` into `web/src/schema/`
   (`json-schema-to-typescript`), wired into gate G1. ~1 hour, unblocks Track A.
4. **Codegen: Pydantic models** (`datamodel-code-generator`). ~1 hour.

Item 1 is the critical path to M1. Items 3 and 4 are small and good for a short session.

### Open questions

- **Repo visibility.** The repo is currently **public**.
  `docs/07-infrastructure-and-cost.md` §4.4 recommends `engine/` public (free CI, supports
  the papers) but filing provisionals **before** any public push. Public disclosure can
  start or forfeit patent filing windows. **Both of you + VIT's IP office should confirm
  this is intended.** My recommendation: keep it public for the CI and paper benefits, and
  drop patent scope 2 (the SharedArrayBuffer memory architecture) — its novelty is thin
  anyway, as noted in `docs/05-roadmap-and-risks.md` §6.

### Gotchas discovered — don't rediscover these

**Navmesh**

- **Doorways must be sealed before region classification.** A door is a *gap* in a wall
  run, so a hall with doors has an unclosed outline and the exterior fill leaks straight
  in — giving zero walkable area. Seal with virtual constraint edges, classify, then
  re-open as portals. Test: `region::door_gaps_leak_until_sealed`.
- **The funnel algorithm uses an inverted sign convention.** Mononen's `triarea2` is
  `-cross(b-a, c-a)`, the negation of standard CCW orientation. Passing `orient()` straight
  in swaps left/right and the path hugs the *far* side of every corner — a plausible route
  about twice as long as it should be. See the comment on `navmesh::area_sign`.
- **Assert exact taut path length, not a loose bound.** Both funnel bugs above produced
  routes that passed a "shorter than 1.9×" check. `(len - taut).abs() < 1e-9` caught them.
- **An obstacle sharing an edge with the outer wall is not a closed ring.** Modelling a
  wall spur as a separate rectangle whose base sits on the outer wall is incoherent — that
  "interior" is solid wall contiguous with the outdoors, reachable at two nesting parities.
  Trace one simple ring around the spur instead. `classify` reports this as
  `InconsistentNesting`.
- Region classification seeds from the convex hull, and a **constrained hull edge means
  depth 1, not 0** — the true exterior is the unbounded region outside the hull and owns no
  triangles. Seeding everything at 0 classifies an entire rectangular hall as solid.
- `triangulate` **rejects duplicate points** rather than dropping them, because dropping
  would shift every downstream vertex index. Callers dedupe first.

**Schema**

- `#[serde(rename_all)]` on an **enum** renames variants, **not** their fields. Struct
  variants with multi-word fields (e.g. `Lognormal { mu_ln }`) need their own attribute.
  Guarded by `every_schema_property_is_camel_case`.
- `Distribution::sample_icdf(0.0)` on an unbounded Normal used to return `-inf`. Uniform
  PRNGs do emit exactly 0.0, so this would have injected NaN agents. Clamped away from
  both endpoints now (`U_EPS`).

**Build / tooling**

- `cf-geom`'s serde support is a default-on feature so `cf-sim`'s wasm bundle can drop
  serde and schemars. **Field-level** attributes need `cfg_attr` too, not just derives —
  easy to miss, and only breaks the `--no-default-features` build. CI builds that path.
- Clippy rejects methods named `add`/`sub` (shadows `std::ops`). `Vec2` implements the real
  operator traits: use `a + b`, `a - b`, `v * k`.
- Acklam's coefficients in `dist.rs` carry a digit more than f64 holds. The
  `excessive_precision` lint is allowed there deliberately so the constants stay checkable
  against the published algorithm. Don't "fix" it.
- `schemars` is pinned at 0.8 (1.x has a different API). If you upgrade, the manual
  `#[schemars(with = ...)]` on `Vec2` needs revisiting.

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
