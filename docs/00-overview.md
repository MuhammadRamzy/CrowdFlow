# CrowdFlow Studio — Overview, Scope & Assumptions

> Source material: `Project Idea_ CrowdFlow Studio.pdf`, `Crowd Simulation Software Project Plan.pdf`,
> `CrowdFlow_Studio.pdf` (pitch deck). This document set turns those into an executable plan.

---

## 1. What we are building

A browser-based platform where a planner can:

1. **Import** a venue (DWG / DXF / vector PDF / scanned raster) and get topologically clean,
   simulation-ready geometry — or **draw** one from scratch in a CAD-grade canvas editor.
2. **Author** operational reality on top of that geometry: turnstiles, checkpoints, queues,
   one-way corridors, restricted zones, arrival curves, agent demographics.
3. **Simulate** microscopic agent-based crowd movement (Social Force + contact resolution)
   at interactive frame rates, in the browser for design-loop scale and on the server for
   stress-test scale.
4. **Analyse** the result: density heatmaps, flow lines, dwell maps, bottleneck ranking,
   throughput graphs, evacuation time estimates.
5. **Certify** against NFPA 101 / UK Green Guide / NFPA 130 and export a professional PDF
   dossier, plus CSV/BOM for the deployment team.

## 2. The two tracks

The project splits cleanly into two mostly-parallel engineering tracks joined by a single
data contract (the **Venue Document**, see `02-data-model.md`).

| | Track A — Venue Designer + Import | Track B — Simulation Engine |
|---|---|---|
| **Owns** | React/TS canvas editor, component library, scenario authoring, import pipeline (Python/ML), review UX, report rendering | Rust core: geometry compile, navmesh, ECS, locomotion, components, analytics, compliance math. WASM + native hosts |
| **Language** | TypeScript, Python | Rust |
| **Deliverable** | `venue.json` + `scenario.json` | `run/` result artifacts + live frame buffer |
| **Hard problem** | Topology repair from noisy raster; making CAD-grade editing feel fast in a browser | Determinism, cache-efficient 100k-agent stepping, numerically stable dense-crowd contact |
| **Detail doc** | `03-track-a-venue-designer.md` | `04-track-b-simulation-engine.md` |

They meet at four integration milestones (`05-roadmap-and-risks.md`).

## 3. Scope decisions

### In scope for v1

- 2D simulation with multi-floor stacking and vertical links (stairs / ramps / escalators / lifts).
- Deterministic, reproducible runs (same seed + same inputs → bit-identical output).
- Browser simulation up to ~25k agents interactive; server simulation to 100k+.
- NFPA 101 occupant load, NFPA 101/130 egress, UK Green Guide rates of passage.
- Import: DXF, vector PDF, SVG (deterministic) and PNG/JPG/raster PDF (AI-assisted).
- DWG via conversion to DXF (see risk R-07).

### Explicitly out of scope for v1 (deliberately)

- 3D rendering / walkthrough. 2.5D floor-stack view only.
- Real-time IoT digital twin. Architecture leaves room (`01-architecture.md` §9) but no ingest in v1.
- LLM-cognitive agents. Behaviour is physics + rule-based. The `Behavior` trait is designed so
  an LLM deliberation layer can be slotted in later without touching the locomotion core.
- Multi-user real-time co-editing. The command-log design makes it tractable later; v1 is
  single-writer with optimistic locking.
- Fire/smoke coupling (FDS-style). Evacuation mode alters agent urgency, not atmosphere.
- Mobile authoring. Desktop browser only; mobile is view-only for reports.

## 4. Assumptions this plan is built on

State these back to me if any are wrong — several change the calendar materially.

| # | Assumption | Impact if wrong |
|---|---|---|
| A1 | Team of **4 engineers**: 1 Rust/systems, 1 frontend/graphics, 1 ML/CV, 1 full-stack/backend. Plus part-time PM/design. | Calendar scales roughly linearly; effort is quoted in engineer-weeks so you can re-cut it. |
| A2 | This is an **academic project with intended industry entry**. Papers and a defensible product are both deliverables, so compliance correctness and V&V matter from the start. | If purely academic with no commercial intent, drop B6 and §5–6 of `06-validation.md` (~10 weeks). |
| A3 | Infrastructure must be **free-tier / zero recurring cost** through v1 beta. No GPU budget for serving. | Drives real architectural choices — CPU-first ONNX inference, browser-side simulation, self-hosting on one always-free VM. Fully specified in `07-infrastructure-and-cost.md`. |
| A4 | We can deploy with **cross-origin isolation** (COOP/COEP headers). Cloudflare Pages supports this via `_headers`. | Without it, no `SharedArrayBuffer` → no WASM threads → ~4× lower agent ceiling in-browser. Verify in P0 week 1. |
| A5 | Initial target regulations: NFPA 101, UK Green Guide, NFPA 130, NBC India Part 4. | Each additional jurisdiction is ~1–2 engineer-weeks of rule authoring + review. |
| A6 | Start date ≈ **2026-08-03**; v1 beta ≈ **2027-03**. | See `05-roadmap-and-risks.md` for the dated milestone table. |
| A7 | Commercial intent means **no AGPL / GPL / non-commercial dependencies** in the product path. | Rules out Ultralytics YOLO and the original SegFormer weights. Audit + CI gate in `07-infrastructure-and-cost.md` §5. |

## 5. Honest corrections to the source documents

Three things in the source material need adjusting before they become engineering commitments.

**C1 — VLMs should not produce coordinates.**
The deck proposes a VLM emitting "precise global topological coordinates". Current VLMs are
unreliable at sub-pixel spatial regression; they hallucinate plausible-looking coordinates.
The plan instead uses **CV/segmentation for geometry** (precision) and the **VLM for semantics,
scale inference, and validation** (judgement). This preserves the differentiator while making the
accuracy target achievable. See `03-track-a-venue-designer.md` §A5.

**C2 — "100,000 agents in the browser" needs qualifying.**
Achievable with WASM SIMD + threads + level-of-detail, but not at 60 fps with per-agent
rendering and full SFM neighbour queries on a mid-range laptop. Public targets in this plan:
**25k agents @ 60 fps in browser (stretch 50k), 250k @ ≥5× realtime on a server worker.**
100k in-browser is a stretch goal behind an explicit LOD mode. Better to ship a number we hit.

**C3 — Liability posture.**
This is life-safety software. From day one: every report carries a verification statement,
engine version, and an explicit "decision support, not a substitute for a competent fire
engineer's assessment" clause. `06-validation.md` defines the RiMEA/IMO evidence pack that
makes that claim defensible rather than decorative.

## 6. Glossary

| Term | Meaning |
|---|---|
| **Venue Document** | Authored, editable, human-facing geometry + semantics. The source of truth. `venue.json` |
| **Scenario** | Populations, arrival curves, itineraries, events, mode. Separate from geometry so one venue → many scenarios. |
| **Compile** | Deterministic transform of Venue Document → NavGraph. The boundary between Track A and Track B. |
| **NavGraph** | Compiled simulation geometry: triangulation, portals, obstacle BVH, cost fields. Machine-facing, disposable, cached. |
| **Run** | One execution of (venue version, scenario, seed, engine version). Immutable artifact set. |
| **Component** | An intelligent placed object (turnstile, checkpoint, stair) with encoded operational metadata. |
| **OLF** | Occupant Load Factor — NFPA 101 net area per person. |
| **Rate of passage** | Green Guide egress throughput: 82 persons/m/min level, 66 persons/m/min stepped. |
| **SFM** | Social Force Model (Helbing & Molnár 1995). |
| **ECS** | Entity-Component-System; Structure-of-Arrays memory layout. |

## 7. Document map

| File | Contents |
|---|---|
| `00-overview.md` | This file. Scope, assumptions, corrections. |
| `01-architecture.md` | System architecture, services, tech stack, repo layout, API surface, deployment. |
| `02-data-model.md` | Venue/Scenario schemas, compiled artifacts, result format, versioning model, DB schema. |
| `03-track-a-venue-designer.md` | Track A phases A1–A6 in detail. |
| `04-track-b-simulation-engine.md` | Track B phases B1–B6 in detail, algorithms, performance budget. |
| `05-roadmap-and-risks.md` | Phase table, dated milestones, dependency graph, risk register, team plan, academic outputs. |
| `06-validation.md` | Verification & validation strategy, RiMEA suite, compliance evidence pack. |
| `07-infrastructure-and-cost.md` | Free-tier service selection, deployment topology, licensing audit, cost model, bootstrap checklist. |
