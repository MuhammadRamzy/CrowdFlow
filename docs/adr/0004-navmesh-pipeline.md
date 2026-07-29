# 0004 — Navmesh pipeline: CDT → regions → portals → A\* + funnel

**Status:** accepted · **Date:** 2026-07-29 · **Session:** Ramzy

## Context

Agents need to path across a floor. The floor arrives as walls, obstacle outlines and
door openings — none of which is directly navigable. Something has to turn that into a
structure supporting "shortest walkable route from A to B" fast enough to serve tens of
thousands of agents.

`docs/04-track-b-simulation-engine.md` §B1 already committed to a triangulation over a
grid: venues have long diagonal and curved walls (stadium bowls, concourses) that a grid
either aliases or needs a punishing cell size to represent. This ADR records the pipeline
built on top of that choice, and the two non-obvious things it forced.

## Decision

Four stages, each independently testable:

1. **Constrained Delaunay triangulation** — walls become constraint edges the mesh must
   contain. Bowyer–Watson for the points, then constraint insertion by carving and
   re-triangulating pseudo-polygons.
2. **Region classification** — count constraint crossings from the exterior inward. Odd
   nesting depth is walkable, even is solid. This is what distinguishes floor from the
   inside of a pillar and from the outdoors.
3. **Portal extraction** — the shared edges between walkable triangles, each carrying its
   length as the clear width at that crossing.
4. **Pathfinding in two stages** — A\* over the triangle dual picks a corridor of
   triangles; the funnel algorithm string-pulls it into the true geometric shortest path
   through that corridor.

Every geometric decision routes through `cf-geom`'s exact predicates.

## Consequences

**Easier:** each stage has its own failure mode and its own tests, so a bad path is
attributable. Portal width is measured at construction rather than reconstructed later,
which matters because it is the figure Green Guide egress capacity uses. Region
classification doubles as a *diagnostic*: an unclosed wall run makes nesting depth
ambiguous, and that ambiguity is reported rather than silently resolved.

**Harder:** four stages is more surface than a grid would need, and `locate()` is
currently a linear scan. `cf-compile` will add a uniform grid index over triangles for the
per-tick lookups the simulation makes; that is an index over this structure, not a change
to it.

**Two things this forced, both discovered by testing rather than by design:**

- **Doorways must be sealed before classification.** A door is a *gap* in a wall run, so a
  hall with doors has an unclosed outline, and the exterior fill leaks straight in —
  yielding zero walkable area. `cf-compile` must insert virtual constraint edges across
  door openings, classify, then re-open them as portals. There is a test documenting this
  (`region::door_gaps_leak_until_sealed`) precisely so the next person does not rediscover
  it against the M1 fixture.

- **The funnel uses an inverted sign convention.** Mononen's `triarea2` is
  `-cross(b-a, c-a)`, the negation of standard CCW orientation. Passing `orient()` straight
  in swaps left and right, and the path then hugs the *far* side of every corner: a
  perfectly plausible route roughly twice as long as it should be. It is called out in the
  code comment on `area_sign`.

**Revisit if:** profiling shows A\* over the dual is too slow at scale. The planned answer
is already in the design docs — flow fields (one Dijkstra per goal, O(1) per agent per
step) rather than per-agent A\*. A\* stays for one-off queries and for building the fields.

## Alternatives considered

- **Uniform grid navmesh.** Rejected in §B1: aliases diagonal walls, and the cell size
  needed to represent a 0.9 m doorway across a 400 m venue is punitive.
- **Single-stage pathfinding (A\* only).** Rejected: yields centroid-to-centroid paths
  that visibly zigzag. The funnel is not optional polish; without it agents wander.
- **Skipping region classification and treating every triangle as walkable.** Rejected:
  the triangulation tiles the convex hull, so this would let agents walk through pillars
  and outside the building.
- **A general polygon-clipping library to compute the walkable region directly.** Would
  work, but the crossing-count classification falls out of the CDT for ~60 lines and keeps
  the dependency surface small — and it produces the leak diagnostic for free.
