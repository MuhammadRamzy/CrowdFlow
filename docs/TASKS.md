# TASKS — shared backlog

Tick items as you complete them, in the same commit as the work. This is the one place
both sessions can see what is and isn't done. Detail for each phase lives in
`docs/03-track-a-venue-designer.md` and `docs/04-track-b-simulation-engine.md`.

Legend: `[ ]` todo · `[~]` in progress (put your name) · `[x]` done

---

## P0 — Foundations (W1–W3)

- [x] Repo, `.gitignore`, Cargo workspace
- [x] `cf-schema`: geometry primitives (`Vec2`, `Polyline`, `Polygon`, `Aabb`, `Transform`)
- [x] `cf-schema`: distributions with inverse-CDF sampling
- [x] `cf-schema`: typed ids
- [x] `cf-schema`: Venue document
- [x] `cf-schema`: Scenario document
- [x] `cf-schema`: structural + referential validation
- [x] `gen-schema` binary → `schema/*.json`
- [x] First shared fixture (`hall-two-doors`)
- [x] `CLAUDE.md`, `docs/STATE.md`, `/handoff` skill
- [x] CI: fmt, clippy, test, schema-drift gate
- [ ] Codegen → TypeScript types (`web/src/schema/`)
- [ ] Codegen → Pydantic models (`services/api/`)
- [ ] Cross-language round-trip test (same fixture, three languages, identical structure)
- [ ] `licence-check` CI job (`cargo-deny`, `pip-licenses`, `license-checker`)
- [ ] Apply for GitHub Student Pack / Azure for Students / GCP Research Credits
- [ ] Provision Oracle Always Free ARM VM + `infra/compose.yml`
- [ ] Cloudflare Pages + R2 + verify `crossOriginIsolated === true`
- [ ] Decide repo visibility with VIT IP office (see `STATE.md` open questions)

## Track B — Simulation Engine

### B1 — Geometry, navmesh, compiler (W3–W9) · critical path
- [x] `cf-geom`: robust orientation/incircle predicates (Shewchuk via `robust`)
- [x] `cf-geom`: segment intersection, distance queries, point-in-polygon
- [x] `cf-geom`: polygon validity (winding, convexity, self-intersection, defects)
- [x] `cf-geom`: polygon offsetting (wall thickness → obstacle), miter limit + bevel
- [x] `cf-navmesh`: Delaunay triangulation (Bowyer-Watson, exact predicates)
- [ ] `cf-navmesh`: constraint edge insertion (makes it *constrained* Delaunay)
- [ ] `cf-navmesh`: refinement, adjacency, portals with clear width
- [ ] `cf-navmesh`: funnel algorithm for corner-cutting paths
- [ ] `cf-navmesh`: flow fields (Dijkstra on triangle dual + gradient)
- [ ] `cf-navmesh`: multi-floor link portals
- [ ] `cf-compile`: `VenueDoc` → `NavGraph`
- [ ] `cf-compile`: `CompileWarning` set (unreachable zone, no exit, narrow opening, …)
- [ ] Acceptance: compile `convention-hall` fixture < 800 ms, zero unreachable area

### B2 — ECS + locomotion (W8–W14)
- [ ] SoA `World` with hot/warm/cold array split
- [ ] Spatial hash with counting-sort rebuild
- [ ] Fixed-timestep step loop
- [ ] Social Force Model: drive, agent repulsion (anisotropic), wall repulsion
- [ ] PBD contact resolution
- [ ] Speed distributions + Weidmann density-speed reduction
- [ ] Seeded PRNG with per-system streams
- [ ] RiMEA component tests (walking speed, stairs, corner, speed distribution, door flow)
- [ ] Fundamental diagram harness vs Weidmann envelope
- [ ] Determinism gate G2 in CI (x86-64 / aarch64 / wasm32)

### B3 — WASM host (W12–W17)
- [ ] `cf-wasm` bindings + worker protocol
- [ ] SharedArrayBuffer double-buffered layout
- [ ] `wasm-bindgen-rayon` thread pool
- [ ] `simd128` force kernel + scalar fallback (bit-identical)
- [ ] Single-threaded non-isolated fallback path
- [ ] NavGraph cache in IndexedDB

### B4 — Components, modes, behaviour (W15–W22)
- [ ] `ServiceNode` trait + turnstile, checkpoint, desk, seating, vertical link
- [ ] Queue lanes + slot assignment (parallel / serpentine / single)
- [ ] Groups: cohesion, speed matching, split/rejoin
- [ ] Patience + reneging
- [ ] Wayfinding: familiarity-gated flow field vs signage
- [ ] Adaptive rerouting with congestion-weighted fields
- [ ] Operational modes: event flow, peak load, evacuation
- [ ] Runtime event injection + incremental flow-field invalidation

### B5 — Analytics + compliance (W21–W28)
- [ ] Density grid (Gaussian-smoothed), velocity field, dwell map
- [ ] Cohort trajectory sampling
- [ ] Event log
- [ ] Bottleneck detection + ranking
- [ ] Throughput per component/opening
- [ ] Egress metrics (50/90/99/100% cleared)
- [ ] `cf-compliance` rule evaluator + data-driven rule format
- [ ] NFPA 101 rule pack + hand-worked fixtures
- [ ] Green Guide rule pack + hand-worked fixtures
- [ ] NFPA 130, NBC India rule packs
- [ ] **External review of every rule by someone with fire-engineering knowledge**

### B6 — Native worker + scale (W25–W32)
- [ ] `cf-native` CLI
- [ ] Redis worker mode + progress streaming
- [ ] Scale to 250k agents
- [ ] Monte Carlo seed sweeps
- [ ] Parameter sweeps + comparison matrix

## Track A — Venue Designer + Import

### A1 — Canvas core (W3–W8)
- [ ] Vite + React + TS scaffold
- [ ] PixiJS viewport: pan, zoom, fit, DPI
- [ ] Scene graph + R-tree hit testing
- [ ] Snapping solver (grid, endpoint, midpoint, intersection, perpendicular, angle)
- [ ] Document store + `Command` log + undo/redo with coalescing
- [ ] Wall tool, rect/polygon tool
- [ ] Selection + multi-select transform
- [ ] Vertex editing
- [ ] Layer panel
- [ ] Measure tool
- [ ] Command palette
- [ ] Acceptance: draw `hall-two-doors` in < 90 s, round-trips byte-identically

### A2 — Semantic authoring + components (W7–W12)
- [ ] Opening placement on walls (parametric `t`)
- [ ] Zone drawing + "detect rooms" from wall loops
- [ ] Zone inspector with live NFPA occupant-load preview
- [ ] Component palette + per-type inspectors
- [ ] Component ↔ queue-area binding
- [ ] Multi-floor stack + trace-below underlay
- [ ] Vertical link placement with cross-floor validation
- [ ] Validation panel driven by background compile

### A3 — Pathflow + scenario (W11–W16)
- [ ] Routing graph editor
- [ ] Flow constraints (one-way painting)
- [ ] Access-tag matrix
- [ ] Arrival curve editor with derivative plot
- [ ] Population builder (distribution editors generated from schema)
- [ ] Itinerary builder
- [ ] Event timeline
- [ ] Scenario manager (duplicate, tag, compare)

### A4 — Deterministic import (W9–W14)
- [ ] DXF ingest (`ezdxf`), blocks/xrefs/inserts
- [ ] Vector PDF ingest (`pypdfium2`)
- [ ] SVG ingest
- [ ] Layer mapping UI + heuristic seeding + per-org persistence
- [ ] Topology repair: dedupe, dominant direction, endpoint clustering, collinear merge
- [ ] Topology repair: L/T junction closing as min-cost assignment
- [ ] Topology repair: sliver removal, planar arrangement, face → rooms
- [ ] Opening inference from wall gaps
- [ ] Wall thickness inference from parallel pairs
- [ ] Scale calibration (header / OCR / door prior / manual two-point)
- [ ] Review UI: proposal layer, confidence bands, accept/reject/edit diff

### A5 — AI raster import (W15–W26)
- [ ] Data pipeline + CubiCasa5K adapter + eval harness
- [ ] **Synthetic venue generator** (highest-leverage item — schedule first)
- [ ] Wall segmentation model (permissive backbone only)
- [ ] Symbol detection (RT-DETR / YOLOX — **not** Ultralytics, AGPL)
- [ ] OCR + dimension-string grammar
- [ ] VLM semantic/validation pass behind `VlmProvider`
- [ ] ONNX INT8 export for CPU serving
- [ ] Active-learning capture from user corrections
- [ ] Metric: median human correction time < 10 min/sheet

### A6 — Results viz + reporting (W21–W28)
- [ ] Instanced agent rendering from SharedArrayBuffer
- [ ] LOD / density-splat above 30k on screen
- [ ] Transport controls + timeline scrub
- [ ] Density heatmap shader with 2/4/6 p/m² banding
- [ ] Flow lines, dwell map, bottleneck markers
- [ ] Throughput charts, Fruin LOS bands
- [ ] Report composer (Typst) + section toggles
- [ ] Auto-generated recommendations from failed compliance rules
- [ ] CSV exports + Bill of Materials
- [ ] Verification statement on every export

## P5 — Hardening + beta (W29–W34)

- [ ] Performance pass against the §10 non-functional targets
- [ ] Accessibility pass
- [ ] Onboarding + docs
- [ ] Pilot users
- [ ] V&V report published for the shipping engine version
