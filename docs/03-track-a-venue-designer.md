# Track A — Venue Designer + Import

Owns: the canvas editor, the intelligent component library, scenario authoring, the import
pipeline, the review UX, results visualization, and report composition.

Phases A1–A6. A4 (deterministic import) intentionally ships **before** A5 (AI import).

---

## A1 — Canvas core (6 eng-weeks)

The foundation everything else sits on. Get this wrong and every later phase is slow.

### Architecture

```
web/src/canvas/
  viewport.ts       # world↔screen transform, pan/zoom, DPI, fit-to-content
  scene.ts          # retained scene graph over PixiJS containers, layer ordering
  hittest.ts        # spatial index (R-tree) over authored elements; pick under cursor
  snap.ts           # snapping solver: grid, endpoint, midpoint, intersection, extension,
                    # perpendicular, parallel, angle (15° increments)
  tools/            # state machines: Select, Wall, Rect, Polygon, Opening, Measure, Pan
  overlay.ts        # transient chrome: handles, dimension ghosts, snap indicators
```

**Do not put canvas elements in React.** React renders the shell (toolbars, inspector, panels).
The canvas is a PixiJS app driven by an imperative scene that subscribes to the document store.
Mixing them is the standard way these editors end up at 12 fps.

**Snapping is a solver, not an if-chain.** Candidate snap targets are collected within a screen-space
radius, scored by priority × distance, and the best is applied. Adding a new snap type is one
candidate generator. Users can hold a modifier to suppress.

### Document store and commands

```ts
interface Command {
  readonly kind: string;
  apply(doc: Draft<VenueDoc>): void;
  invert(): Command;
  readonly coalesceKey?: string;   // drag ops coalesce into one undo entry
}
```

Every mutation goes through `dispatch(cmd)`. This buys undo/redo, dirty tracking, autosave
deltas, an audit trail, and a future CRDT path — all from one discipline enforced by a lint rule
that forbids direct store writes outside `doc/commands/`.

### Deliverables

- Pan / zoom / fit / zoom-to-selection; 60 fps at 20k primitives.
- Grid with adaptive subdivision, metric + imperial display, configurable origin.
- Wall tool: click-chain polyline, ortho lock, live length/angle readout, close-loop detection.
- Rect/polygon tool for zones and obstacles.
- Selection: click, marquee, shift-add, alt-subtract; multi-select transform (move/rotate/scale).
- Vertex editing: drag, insert, delete, with live constraint feedback.
- Layer panel: visibility, lock, reorder, isolate.
- Measure tool: distance, area, angle.
- Undo/redo (≥200 entries) with coalescing.
- Keyboard-first: every tool has a shortcut; command palette (`⌘K`).

### Acceptance

Draw the `fixtures/unit/hall-two-doors` venue from scratch in under 90 seconds; the resulting
JSON round-trips byte-identically through save/load; undo restores every intermediate state.

---

## A2 — Semantic authoring & component library (6 eng-weeks)

Turning geometry into a *venue*.

### Deliverables

- **Openings**: place on a wall by click; the tool resolves the parametric `t`, previews the
  swing, validates against min clear width, marks fire exits.
- **Zones**: draw or auto-detect from enclosed wall loops ("Detect rooms" runs the same planar
  face extraction the import pipeline uses — shared code path, one place to fix bugs).
- **Zone inspector**: `kind` (drives NFPA OLF, previewed live: "OLF 0.65 m²/p → 1,962 occupants"),
  access tags, speed multiplier, attractors.
- **Component palette**: drag-and-drop the v1 library (`02-data-model.md` §2.3), with per-type
  inspectors surfacing the encoded operational metadata (service-time distribution with a live
  histogram preview, throughput ceiling, lane count, direction).
- **Component ↔ queue-area binding**: draw a queue polygon, bind it to a component; the editor
  shows the implied max queue occupancy.
- **Multi-floor**: floor stack UI, per-floor canvas, "trace floor below" ghost underlay, vertical
  link placement that requires footprints on both floors and validates alignment.
- **Validation panel**: live list of `CompileWarning`s from a debounced background compile
  (WASM, off-thread). Click a warning → canvas pans to and highlights the offending element.

### The validation panel is the product's quality bar

Most competing tools let you build a nonsense venue and only fail at simulation time. Running
the real compiler continuously in a worker, and rendering its warnings as an actionable
checklist, is a small amount of work (the compiler already exists for Track B) with a
disproportionate effect on perceived quality. Budget 1 of the 6 weeks here.

### Acceptance

Author `fixtures/real/convention-hall` — 40 zones, 12 components, 2 floors — with zero
compile errors, in under 30 minutes.

---

## A3 — Pathflow & scenario authoring (5 eng-weeks)

Core Capability 3. Where the planner's operational intent gets expressed.

### Deliverables

- **Routing graph editor**: drop waypoints, connect with edges, set directionality and cost
  multipliers. Rendered on its own toggleable layer so it doesn't clutter the plan.
- **Flow constraints**: paint a zone as one-way with a heading vector and a compliance strength
  (0.85 = 15% of agents ignore it, which is what actually happens).
- **Restricted zones**: access-tag matrix (general / staff / VIP / accredited / contractor).
- **Arrival curve editor**: draggable cumulative-arrival spline with presets (surge, steady,
  bimodal, doors-open), live "agents per minute" derivative plot underneath, and an entry-point
  weight distributor.
- **Population builder**: count, demographic profile (speed/radius/group-size/patience
  distributions with visual pickers), mobility-impaired fraction, familiarity.
- **Itinerary builder**: an ordered list of goals with probabilities and dwell distributions —
  "enter → security → hall (dwell) → 42% food court → exit".
- **Event timeline**: place alarms, gate closures, link blockages on a time ruler.
- **Scenario manager**: duplicate, rename, tag, compare. A venue's scenario list is the
  "contingency plans" feature from Core Capability 8.

### Design note

Every distribution editor writes the same tagged-union JSON the engine samples from
(`02-data-model.md` §3). Generate the editor UI from the distribution schema rather than
hand-writing eight forms.

---

## A4 — Deterministic import: DXF / vector PDF / SVG (6 eng-weeks)

**Ship this before the AI path.** Reasons: a large fraction of real professional venue files are
already vector; it validates the whole ingest→repair→review pipeline without ML risk; and it
gives the ML phase a working harness and a ground-truth comparison baseline.

### Pipeline

```
services/import-worker/
  ingest/dxf.py          # ezdxf → entities; handle blocks/xrefs/inserts, explode nested
  ingest/pdf_vector.py   # pypdfium2 → path ops; filter by stroke width & colour
  ingest/svg.py
  vectorize/normalize.py # arcs/splines → polylines at tolerance; units from header
  topology/repair.py     # the hard part, below
  emit/venue.py          # → draft venue.json + confidence
```

### Layer mapping UI

DXF layers are the semantic signal. Show the user a table of detected layers with entity counts
and a sample thumbnail, and let them map each to `structural wall | partition | door | window |
furniture | dimension | text | ignore`. Seed the mapping with heuristics (name regex:
`A-WALL`, `WALL*`, `MUR*`, `*-DOOR-*`; AIA/ISO 13567 layer conventions; stroke weight; colour).
Persist mappings per-org so the second file from the same architect is one click.

### Topology repair (shared with A5 — this is the crown jewel)

Sequence, all classical computational geometry, no ML:

1. **Dedupe & clean** — remove zero-length, exact-duplicate, and fully-contained collinear segments.
2. **Dominant direction estimation** — histogram of segment angles → principal axes (rarely
   axis-aligned in scans). Rotate into that frame for the orthogonal steps, rotate back after.
3. **Endpoint clustering** — kd-tree, cluster endpoints within `ε = max(2 px, 0.05 m)`, snap to
   cluster centroid.
4. **Collinear merge** — merge segments whose angle differs < 2° and perpendicular offset < ε.
5. **Junction closing** — for each dangling endpoint, find the nearest segment within `k·ε`;
   extend to the intersection (L-junction) or project onto it and split (T-junction). Formulated
   as a min-cost assignment so we don't create crossing artifacts greedily.
6. **Sliver removal** — drop faces with area < `A_min` or aspect ratio > 50.
7. **Planar arrangement** — build the arrangement of all segments, extract bounded faces.
   Faces = candidate rooms. Unbounded face = exterior.
8. **Opening inference** — gaps in wall runs of width ∈ [0.7 m, 3.0 m] become candidate openings;
   detected door symbols (A5) override with higher confidence.
9. **Wall thickness inference** — pair parallel segments 0.08–0.5 m apart into single centerline
   walls with thickness.
10. **Validate** — every candidate room reachable from exterior? Report as `CompileWarning`s.

Each step emits per-element confidence and a provenance record. Steps 3–6 have tolerance
parameters exposed in the review UI as a single "aggressiveness" slider with live re-preview —
because the right tolerance genuinely differs between a crisp CAD export and a fax-quality scan.

### Scale calibration

Order of preference, always shown to the user for confirmation:
1. DXF `$INSUNITS` header + explicit units.
2. OCR'd dimension strings cross-checked against the geometry they annotate (A5).
3. Door-width prior: modal detected door width ≈ 0.9 m.
4. Manual two-point calibration ("click two points, type the real distance").

Never proceed on an unconfirmed scale. A wrong scale silently invalidates every downstream
compliance number, so this gets a blocking modal, not a toast.

### Review UI

The import result renders on a `proposal` layer in a distinct colour. A side panel lists element
groups by confidence band with counts. Actions: accept all / accept group / accept element /
reject / edit-then-accept. Low-confidence elements are visually flagged. Only on
"Commit import" does it become a venue version, with provenance recorded.

---

## A5 — AI raster import (12 eng-weeks, ML-heavy)

For scanned plans, photographs of plans, and raster PDFs. Reuses everything from step
"vectorize/normalize" onward in A4 — **the AI stages only replace the front end of the pipeline**.

### Corrected approach (see `00-overview.md` C1)

| Task | Method | Why |
|---|---|---|
| Wall geometry | Semantic segmentation — encoder + U-Net decoder, tiled 512² with 64 px overlap. **Permissively-licensed backbone only** (DINOv2 / ConvNeXt / timm), *not* the original SegFormer weights — see `07` §5 | Pixel-precise. VLMs cannot do this reliably. |
| Doors / windows / stairs / columns / lifts | **RT-DETR or YOLOX (Apache-2.0)** object detection. **Not Ultralytics YOLO — AGPL-3.0 is incompatible with hosted SaaS** | Symbol detection is a solved detection problem; the licence choice is the only real decision here. |
| Room labels, dimension strings, scale bars | PaddleOCR / TrOCR, with a dimension-string grammar parser | Text is text. |
| Room semantics, disambiguation, scale sanity-check, "is this even a floor plan?" | **VLM (Claude)** over the image + the extracted structured candidates | Judgement, not regression. Ask it to *label and validate*, never to emit coordinates. |
| Sheet segmentation (title block, legend, multiple plans per sheet, north arrow) | VLM + layout heuristics | Genuinely multimodal reasoning task. |

The VLM sees the image plus a rendering of our extracted geometry and answers structured
questions: *which of these detected regions are circulation vs occupiable? does this scale bar
agree with the door widths? which sub-region of this sheet is the primary plan? is this room
label "STORE" a storage room or a retail store?* Structured output via tool-use schema, with a
confidence field, and every answer is a *proposal* the user can override.

### Data

| Source | Use |
|---|---|
| **CubiCasa5K** (5,000 annotated plans) | Primary training set for wall segmentation + room types |
| **ResPlan** (17k residential vector-graph plans) | Additional supervision + topology priors |
| **Synthetic generator** (ours) | Procedurally generate venue-like plans (halls, stadiums, concourses) → render with augmentations (scan noise, JPEG artifacts, skew, coffee stains, blueprint blue, hand annotations). CubiCasa is residential-biased; our target is assembly venues. **This is the highest-leverage item in A5.** |
| **Our labelled set** | 300–500 real assembly-venue plans, labelled in-house, held out for eval |
| **Active learning loop** | Every user correction in the review UI is a labelled example. Instrument from day one. |

### Metrics (tracked in `ml/evals/`)

- Wall mask IoU ≥ 0.88 on held-out real plans.
- Junction F1 (post-repair, within 0.15 m) ≥ 0.85.
- Room count accuracy ±1 on ≥ 90% of plans.
- Door detection mAP@0.5 ≥ 0.90.
- **End-to-end: median human correction time < 10 minutes per sheet** vs. the 4-hour manual
  baseline cited in the source plan. This is the number that matters commercially; the others
  are diagnostics.

### Phasing within A5

| Sub-phase | Weeks | Content |
|---|---|---|
| A5.1 | 2 | Data pipeline, CubiCasa adapter, eval harness, baseline U-Net |
| A5.2 | 3 | Synthetic venue generator + augmentation stack |
| A5.3 | 3 | Wall segmentation model, RT-DETR symbol model, training infra (free GPU: Kaggle ≈30 hr/wk), ONNX INT8 export for CPU serving |
| A5.4 | 2 | OCR + dimension-string parsing + scale inference |
| A5.5 | 2 | VLM semantic/validation pass, structured output, prompt evals |
| A5.6 | (folded into A4 UI) | Confidence surfacing, active-learning capture |

---

## A6 — Results visualization & reporting (7 eng-weeks)

Where the simulation becomes a deliverable.

### Live simulation view

- Agent rendering: instanced quads, one draw call, colour by state (walking / queuing / dwelling /
  blocked / evacuating) or by population. Position data read directly from the `SharedArrayBuffer`
  the WASM worker writes — no serialization per frame.
- LOD: past ~30k on screen, switch to a point-sprite/density-splat representation.
- Transport controls: play / pause / step / speed (0.25×–32×) / scrub. Scrubbing backwards
  requires either re-simulation from a keyframe or reading a completed run's artifacts — v1
  scrubs freely over completed runs, and forward-only during live simulation.
- Live counters: population in venue, per-component queue lengths and wait times, max density.

### Analysis overlays

| Overlay | Implementation |
|---|---|
| **Density heatmap** | Upload the u8 density grid to a texture; colormap in a fragment shader with the 2/4/6 persons/m² banding. Time-windowed or peak-over-time. |
| **Flow lines** | Cohort trajectories → simplify (RDP) → render as tapered polylines with alpha by cumulative traffic. Reveals desire lines vs designed routes. |
| **Dwell map** | Same texture path as density, different source grid. Drives concession placement. |
| **Bottleneck markers** | Ranked list from `cf-analytics`, each pinned to a location with severity, upstream queue length, and time-of-onset. Clicking one jumps the timeline there. |
| **Throughput charts** | Per-component minute-by-minute served/queued/waited. Recharts or similar in the side panel. |
| **Level-of-service bands** | Fruin LOS A–F shading as an alternate colormap. |

Design the visualization system against the `dataviz` skill's palette and accessibility rules —
these charts go into documents shown to regulators, so colour-blind-safe and print-legible are
requirements, not polish.

### Report composer

- Template-driven (Typst), sections toggleable: cover, venue drawing, scenario parameters,
  population summary, heatmaps at chosen timestamps, flow maps, bottleneck table, throughput
  charts, compliance results with clause citations, recommendations, appendix (engine version,
  determinism hash, verification statement).
- **Auto-generated recommendations** come from `cf-compliance`: each failed rule maps to a
  remediation template with the computed delta — *"North exit clear width 2.4 m provides
  197 persons/min; 8-minute egress of 3,100 occupants requires 388 persons/min. Widen to 4.8 m
  or add a second exit of ≥ 2.4 m."*
- CSV exports: per-component metrics, per-zone occupancy, agent event summary.
- **Bill of Materials**: every placed component aggregated by type with quantities and optional
  unit costs → procurement-ready CSV/PDF.
- Every report carries the liability statement from `00-overview.md` C3.

### Acceptance

Generate a 25-page dossier for `fixtures/real/convention-hall` in < 20 s, containing at least
one correctly-cited NFPA 101 finding and one Green Guide egress calculation that a fire engineer
reviewing it agrees is correctly computed.
