# Track B — Simulation Engine

One Rust core (`cf-sim`), two hosts (`cf-wasm`, `cf-native`), bit-identical results.
Phases B1–B6.

---

## B1 — Geometry, navmesh & the compiler (7 eng-weeks)

### `cf-geom`

Primitives, robust orientation/incircle predicates (adaptive precision — `robust` crate),
segment intersection, polygon boolean ops, offsetting, point-in-polygon, distance queries.
No `f64` where `f32` suffices in the hot path, but **all geometry construction is `f64`** and
only the runtime arrays are `f32`. Constructing a navmesh in `f32` produces degenerate
triangles on large venues.

### `cf-navmesh`

**Constrained Delaunay Triangulation** of each floor's walkable region:

- Input: floor boundary polygon − obstacle polygons, with openings as *non-constraining* gaps.
- CDT via a sweep-line/incremental implementation (`spade` crate as a starting point, likely
  forked for determinism control).
- Refinement: Ruppert-style, with a maximum triangle area tied to expected agent density so
  density accumulation has adequate resolution.
- Output: triangles, adjacency, **portals** (shared edges with clear width), per-triangle zone id.

**Why CDT and not a grid:** venues have long diagonal and curved walls (stadium bowls,
concourses). A grid either aliases them or needs a punishing cell size. CDT gives exact
boundaries with ~100× fewer cells, and the funnel algorithm gives geometrically optimal paths.

**Pathfinding: flow fields, not per-agent A\*.**

For N agents and G goals, per-agent A* is O(N · search). Flow fields are O(G · mesh) once,
then O(1) per agent per step. With N = 100,000 and G ≈ 20 this is the difference between
feasible and not.

```
For each goal g:
  1. Dijkstra over the triangle dual graph from g's triangles, cost = centroid distance
     × zone speed multiplier × (1 + congestion_penalty).
  2. Store per-triangle potential φ_g.
  3. Gradient ∇φ_g per triangle → the agent's desired direction.
  4. Within a triangle, refine with the funnel algorithm against the next 2–3 portals so
     agents cut corners naturally instead of walking centroid-to-centroid.
```

Congestion-aware rerouting = recompute a subset of flow fields every ~2 s with a density term
in the cost, and give agents a `familiarity`-gated probability of switching to the updated field.
This produces the adaptive rerouting behaviour from the source plan without per-agent replanning.

**Multi-floor:** each floor is an independent mesh; `LinkPortal`s join them with a traversal
cost, a speed multiplier, and a capacity (Green Guide flow rate). The Dijkstra runs across the
joined graph, so a flow field naturally routes agents down the right stair.

### `cf-compile`

`VenueDoc → NavGraph`, deterministic and pure. Resolves parametric openings to world
coordinates, unions wall thickness into obstacles, cuts opening gaps, triangulates, builds the
BVH and uniform grid, compiles components into `CompiledComponent` service nodes, enumerates
goals, and emits `CompileWarning`s.

**The warning set is a Track A feature.** Budget real time on the messages: unreachable zone,
room with no exit, opening below minimum clear width, component with no bound queue area,
stair footprints misaligned across floors, zone overlapping an obstacle, disconnected mesh island.

### Acceptance

Compile `fixtures/real/convention-hall` (2 floors, ~50k m², ~400 walls) to a valid NavGraph in
< 800 ms, with zero unreachable walkable area, verified by flood fill.

---

## B2 — ECS core & locomotion (7 eng-weeks)

### Memory layout

Structure-of-Arrays, 32-byte aligned for SIMD, no per-agent allocation:

```rust
pub struct World {
    // hot: touched every tick, in this order
    pos_x: Vec<f32>,  pos_y: Vec<f32>,
    vel_x: Vec<f32>,  vel_y: Vec<f32>,
    des_x: Vec<f32>,  des_y: Vec<f32>,      // desired velocity from flow field
    radius: Vec<f32>, desired_speed: Vec<f32>,
    tri: Vec<u32>,                           // current triangle, for O(1) mesh queries
    floor: Vec<u8>,
    state: Vec<AgentState>,                  // #[repr(u8)]

    // warm: touched by behaviour systems at lower frequency
    goal: Vec<u16>, itinerary_step: Vec<u8>, group: Vec<u32>,
    patience_left: Vec<f32>, familiarity: Vec<f32>, population: Vec<u16>,

    // cold: rarely touched, separate allocation
    cold: Vec<AgentCold>,                    // spawn time, full itinerary, stats

    grid: SpatialHash,
    rng: Pcg64,
}
```

The split into hot/warm/cold arrays is the actual source of the ECS speedup — a tick that only
touches the hot arrays streams ~40 bytes/agent instead of ~200.

### Neighbour search

Uniform spatial hash, cell size = 2 × max interaction radius (≈ 2.0 m). Rebuilt every tick by
**counting sort** — O(N), allocation-free, produces agents ordered by cell, which makes the
subsequent neighbour loop nearly linear in memory access. Rebuilding is cheaper than
maintaining incrementally at this agent count and is trivially parallel.

### The step loop

Fixed `dt = 0.05 s` (20 Hz). Order matters:

```
1. rebuild_spatial_hash()                     // counting sort, parallel
2. update_desired_velocity()                  // flow field gradient + funnel refinement
3. compute_social_forces()                    // ★ dominant cost; parallel over agents
     f = f_drive + Σ f_agent + Σ f_wall + f_group + f_flow_constraint
4. integrate()                                // semi-implicit Euler, speed clamp
5. resolve_contacts()                         // ★ PBD position projection, 2–4 iterations
6. update_triangle_membership()               // walk portals, cheap since motion < cell size
7. tick_components()                          // turnstile/checkpoint service, queue admission
8. tick_behaviours()                          // goal advance, dwell, patience, reroute, groups
9. accumulate_analytics()                     // density grid, dwell, events (every k ticks)
```

### Locomotion model: SFM + PBD, not SFM alone

Pure Social Force Model is well-validated at low-to-moderate density but becomes stiff at
high density: preventing overlap needs large repulsion constants, which needs a tiny `dt`,
which kills performance — and it can still produce non-physical overlap and "explosions" at
choke points, exactly the regime we care most about.

**The hybrid:**

- **SFM produces the desired velocity** — driving force to goal, anisotropic repulsion from
  agents (Helbing's exponential with a field-of-view weight), wall repulsion via nearest-surface
  query against the BVH, group cohesion, flow-constraint bias.
- **PBD resolves the constraint** — after integration, run 2–4 Gauss-Seidel-style iterations
  projecting overlapping agent pairs apart along their centre line, mass-weighted, plus
  agent-wall non-penetration. Velocity is corrected from the position delta.

This is unconditionally stable at 20 Hz, guarantees no overlap, and preserves the emergent
behaviours SFM is validated for (lane formation, arching at exits, faster-is-slower). It is also
what modern high-density crowd work has converged on. Both models' parameters are exposed for
calibration in `06-validation.md`.

### Additional behavioural layers

- **Speed distributions** — sampled per agent from the population profile; Weidmann-style
  density-dependent speed reduction applied as a multiplier on desired speed, giving a realistic
  fundamental diagram without hand-tuning SFM constants.
- **Groups** — a cohesion force toward the group centroid plus a speed match to the slowest
  member, with a leash distance beyond which trailing members prioritise rejoining. Groups can
  split under extreme density and attempt to rejoin (recorded as a `group_split` event).
- **Queueing** — an agent entering a bound queue area is assigned a slot in a queue lane
  (serpentine/single/parallel by discipline); slots advance as service completes; agents target
  their slot rather than the server, which is what produces realistic queue geometry rather
  than a mob.
- **Patience & reneging** — `patience_left` decrements while queuing; on expiry, the agent
  re-evaluates alternatives (a different open component, a different route) — the deck's
  "psychological frustration" behaviour, implemented as a concrete threshold rather than a mood.
- **Wayfinding** — `familiarity` interpolates between "use the global optimal flow field"
  (familiar) and "follow the nearest visible sign's flow field, else nearest exit by line of
  sight" (unfamiliar). Matters enormously for evacuation realism and is a common weakness in
  competing tools.

### Performance budget (target: 25k agents @ 60 fps in browser)

At 60 fps rendering with 20 Hz physics, one physics tick per ~3 frames → **16.6 ms budget per tick**
worst case, target ≤ 8 ms.

| Stage | Budget @ 25k agents | Notes |
|---|---|---|
| Spatial hash rebuild | 0.4 ms | counting sort, parallel |
| Desired velocity | 0.6 ms | table lookup + gradient |
| Social forces | **4.0 ms** | ~15 neighbours avg; SIMD 4-wide f32; the thing to optimise |
| Integrate | 0.2 ms | |
| PBD contacts (3 iter) | 1.6 ms | |
| Triangle membership | 0.3 ms | |
| Components + behaviours | 0.5 ms | most agents idle in these systems |
| Analytics | 0.4 ms | every 4th tick only |
| **Total** | **~8.0 ms** | 2× headroom |

---

## B3 — WASM host & browser integration (5 eng-weeks)

### Worker protocol

The sim lives in a dedicated Worker. Main thread never blocks.

```
main ──postMessage──► worker:  { Init, venue, scenario }
                               { Control, Play|Pause|Step|Speed(f32)|Seek(t) }
                               { Patch, command }        // live edits during a paused sim
worker ──postMessage──► main:  { Ready, sab_handles, meta }
                               { Tick, sim_time, frame_index, stats }
                               { Warning, CompileWarning[] }
                               { Done, metrics }
```

### Shared memory layout

Double-buffered so the renderer never reads a torn frame:

```
SharedArrayBuffer (agents):
  [0]                header: { write_index: u32, frame: u32, count: u32, sim_time: f32 }
  [64 .. 64+8N)      buffer A: pos_x f32[N], pos_y f32[N]
  [.. +2N)           buffer A: state u8[N], population u8[N]
  [.. ]              buffer B: same
SharedArrayBuffer (density): u8[floors][H][W], single-buffered, updated every 5 sim-seconds
```

The renderer reads `header.write_index` with `Atomics.load`, reads the other buffer, and uploads
`pos_x`/`pos_y` straight into a GPU instance buffer. **Zero copies, zero serialization, per frame.**

### Threading

`wasm-bindgen-rayon` over `SharedArrayBuffer`, thread count = `navigator.hardwareConcurrency - 1`
capped at 8. Parallel over: spatial hash rebuild, social forces, PBD iterations (with graph
colouring or Jacobi-style batching to avoid write conflicts), analytics accumulation.

`simd128` enabled; the force kernel is written with `core::arch::wasm32::v128` intrinsics behind
a portable fallback (`cfg` on target feature) so native builds use AVX2 via the same abstraction.

**Fallback path is mandatory.** If `crossOriginIsolated === false`: single-threaded, no SAB
(fall back to transferable `ArrayBuffer` per frame), agent cap reduced, UI banner explaining why.
Ship and test this from day one; it's the difference between "degraded" and "broken" for users
behind proxies that strip headers.

### NavGraph caching

Compile in the worker; cache the serialized NavGraph in IndexedDB keyed by content hash. Editing
a scenario (not geometry) never recompiles. Editing geometry recompiles incrementally where
possible — v1 recompiles the affected floor only.

---

## B4 — Components, modes & advanced behaviour (7 eng-weeks)

### Components as service nodes

Each `CompiledComponent` is a discrete-event service node embedded in continuous space:

```rust
trait ServiceNode {
    fn capacity(&self) -> u32;                                  // concurrent servers
    fn admit(&mut self, agent: AgentId, now: f32) -> Option<ServerSlot>;
    fn tick(&mut self, w: &mut World, dt: f32) -> SmallVec<[Completion; 8]>;
    fn queue_geometry(&self) -> &QueueLanes;
    fn throughput_ceiling(&self) -> f32;                        // hard pph clamp
}
```

Implementations: `Turnstile`, `SecurityCheckpoint` (two-stage with probabilistic secondary
screening), `ServiceDesk` (configurable queue discipline), `SeatingBlock` (seat assignment +
row micro-navigation), `VerticalLink` (capacity-limited with flow-rate clamp and directional
speed multipliers).

The `maxThroughputPph` on a component acts as a **hard clamp** independent of the physics —
so a turnstile rated 660 pax/hr cannot pass more even if the SFM would let bodies through.
This is what makes the results defensible against the manufacturer's published spec.

### Operational modes

| Mode | Behavioural changes |
|---|---|
| `event_flow` | Baseline. Itineraries, dwell, normal patience, arrival curve. |
| `peak_load` | Population scaled to theoretical max occupancy; compressed arrival curve; patience reduced. |
| `evacuation` | Alarm event triggers per-agent reaction-time delay (lognormal), itinerary abandoned, goal → `exit:nearest_available`, desired speed ×1.3, personal space reduced, patience → ∞ (people queue at exits rather than reneging), signage compliance up for unfamiliar agents. |

Evacuation mode is where the model must be most defensible; it is the primary target of the
RiMEA verification suite in `06-validation.md`.

### Event injection

Scenario events (`close_opening`, `block_link`, `alarm`, `open_component`) mutate the NavGraph's
traversability at runtime and invalidate affected flow fields, which are recomputed
incrementally. Agents whose current field was invalidated re-target on their next behaviour tick
with a per-agent stagger so the whole crowd doesn't turn on the same frame.

---

## B5 — Analytics & compliance (8 eng-weeks)

### `cf-analytics`

Accumulated during the run, not post-processed from trajectories (which would require storing
them):

- **Density grid** — 0.5 m cells, 5 s buckets. Accumulate a *smoothed* density (Gaussian kernel
  over agent positions, σ ≈ 0.7 m) rather than raw bin counts; raw binning produces noisy
  heatmaps that under-report peaks at cell boundaries. Also track per-cell running max.
- **Velocity field** — mean velocity per cell per bucket, for flow lines and to detect
  counterflow.
- **Dwell map** — cumulative seconds with speed below 0.2 m/s per cell.
- **Cohort trajectories** — a deterministic 2% sample (by agent id hash, so it's stable across
  runs) at 2 Hz, quantized.
- **Event log** — spawn, enqueue, service_start, service_end, exit, reroute, blocked,
  group_split, and `contact_pressure_exceeded`.
- **Bottleneck detection** — a bottleneck is scored by (upstream queue length × duration ×
  density gradient across the constriction). Ranked, deduplicated spatially, each with onset
  time, peak time, and the constriction's geometry. This is the single most commercially
  valuable output; it deserves a real algorithm, not a density threshold.
- **Throughput** — per component and per opening, per minute: arrivals, served, mean/p95 wait,
  max queue.
- **Egress metrics** — time to 50% / 90% / 99% / 100% cleared, per floor and overall; per-exit
  utilization; the classic egress curve.

### `cf-compliance`

A rule evaluator over `(VenueDoc, NavGraph, RunMetrics)`. Rules are data:

```ron
Rule(
  id: "nfpa101.2024.occupant_load.assembly_concentrated",
  code: "NFPA 101", edition: "2024", clause: "Table 7.3.1.2",
  applies_to: ZoneKind("assembly_concentrated"),
  compute: OccupantLoad(olf_m2: 0.65),
  assert: Lte(field: "simulated_peak_occupancy", target: "computed_occupant_load"),
  severity: Violation,
  remediation: "Reduce permitted occupancy to {computed} or increase net floor area.",
  citation: "https://www.nfpa.org/codes-and-standards/nfpa-101-standard-development/101",
)
```

v1 rule packs: NFPA 101 (occupant load, egress capacity, exit count, travel distance,
jam-point density), UK Green Guide (rates of passage, 8-minute egress, minimum widths),
NFPA 130 (platform 4 min / station 6 min), NBC India 2016 Part 4 (travel distance, exit counts),
and crowd-science density thresholds.

Output: `compliance.json` with per-rule `{status, computed, target, evidence_refs, clause}`.
Failures carry a templated remediation with the computed shortfall filled in — this is what
becomes the report's recommendations section.

**Every rule needs a fixture with a hand-worked expected value, reviewed by someone with fire
engineering knowledge.** A compliance engine that is confidently wrong is worse than no
compliance engine. Budget review time in this phase explicitly.

---

## B6 — Native worker & scale (6 eng-weeks)

- `cf-native` binary: `cf-sim run --venue v.json --scenario s.json --seed N --out ./run/`.
- Worker mode: pull from Redis, stream progress, upload artifacts to S3.
- Scale to 250k agents: memory-map the density grids, chunk analytics flushes, `rayon` over
  physical cores, `jemalloc`/`mimalloc`.
- **Monte Carlo sweeps**: `--seeds 1..50` → distributional outputs (mean/p5/p95 egress time),
  because a single stochastic run is not evidence. Reports quote a distribution.
- **Parameter sweeps**: `--sweep components.cmp_001.lanes=2,4,6,8` → a comparison matrix. This
  turns the tool from "test my design" into "find my design", which is the higher-value product.
- Profiling harness, flamegraphs in CI on the reference scenes.

---

## 5. Determinism — how (CI gate G2)

Bit-identical results across x86-64, aarch64, and wasm32 is a hard requirement (it's what makes
the browser preview and the server report agree, and what makes an audit reproducible).

| Hazard | Mitigation |
|---|---|
| Transcendental functions differ between libms | Use the `libm` crate (pure-Rust, spec-following) for **all** `sin/cos/exp/ln/pow/atan2`. Ban `std` float math in `cf-sim` via a clippy lint. |
| FMA contraction differs by target | Compile `cf-sim` with contraction off; never rely on `mul_add` unless explicit. |
| Parallel reduction order | All reductions are **deterministic**: fixed-size chunks reduced in index order, never `rayon::sum` over an unordered iterator. |
| `HashMap` iteration order | Ban `std::collections::HashMap` iteration in `cf-sim`; use `IndexMap` or sorted `Vec`. |
| RNG | One seeded `Pcg64` per *system*, streams derived by `seed ⊕ system_id ⊕ agent_id` — so a parallel force pass and a serial one draw the same numbers. |
| Agent ordering | Agents are processed in stable id order after the spatial sort; the sort is a counting sort (stable), not a comparison sort. |
| SIMD vs scalar paths | Both must be bit-identical: the SIMD kernel uses the same operation order; a CI test runs both and diffs. |
| `f32` vs `f64` drift | Fixed per-field precision in the schema; no implicit widening in the hot loop. |

CI runs every fixture on all three targets and compares `determinism_hash`. A mismatch fails
the build with the first diverging tick and agent index reported.

---

## 6. Testing strategy

| Level | What |
|---|---|
| Unit | Geometry predicates against known-degenerate inputs; distribution samplers against analytic moments; CDT invariants (no crossing constraints, Delaunay property, full coverage). |
| Property | `proptest` — arbitrary polygons compile to meshes whose total area equals the input walkable area within ε; agents never end a tick inside an obstacle; agents never exceed max speed; population is conserved (spawned = in-venue + exited). |
| Golden | Full fixture runs with committed `determinism_hash` + metrics snapshot. |
| Verification | RiMEA test cases 1–15 (`06-validation.md`). |
| Validation | Fundamental diagram vs Weidmann/Fruin; exit flow vs published measurements. |
| Performance | Benchmark suite with regression gate (G3). |
| Fuzz | `cargo-fuzz` on the venue deserializer and the compiler — untrusted user JSON must never panic in WASM (a panic there kills the user's session). |
