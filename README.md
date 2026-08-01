# CrowdFlow Studio

Browser-based crowd simulation and venue modelling. Import or draw a venue, author operational
rules, simulate tens of thousands of agents client-side via Rust/WebAssembly, and export a
regulator-ready compliance dossier.

**Status:** the engine and a working editor exist. You can draw a venue, compile it,
simulate a few hundred agents, and export a compliance dossier. The locomotion model is
**not yet calibrated** — see the caution in any generated report — and the RiMEA
verification suite is outstanding, so no figure it produces is fit for a statutory
submission today.

---

## Running it

```bash
# once — install frontend dependencies
cd web && pnpm install

# once, and again after any Rust change — compile the engine to WebAssembly
pnpm engine

# start the dev server
pnpm dev            # → http://localhost:5173
```

`pnpm engine` runs `wasm-pack` over `engine/cf-wasm` and writes the bundle into
`web/src/engine/`, which is gitignored — so a fresh clone must run it before
`pnpm dev`, or the app will show "Engine failed to start".

> Nothing hand-written belongs in `web/src/engine/`. `wasm-pack` writes a
> `.gitignore` containing `*` there, so anything you put in it is invisible to
> git and will not survive a clone. The typed bridge lives at
> `web/src/engine.ts` for exactly this reason.

### What to try

1. The **hall with two doors** loads on start. Drag to pan, scroll to zoom.
2. **Place agents** in the inspector, then press play in the timeline. Raise the
   speed to 16× to watch a full evacuation in a few seconds.
3. Click a wall or a doorway to select it. Widen a doorway and watch its Green
   Guide rate of passage move.
4. Delete a wall and watch the venue go unsimulable — the status bar goes to
   ALARM and the validation panel says why. `⌘Z` puts it back.
5. **Compliance report** in the tool rail produces the dossier; print it or save
   it as PDF from the browser's print dialog.

### Other commands

```bash
cargo test                                # the whole engine, 218 tests
cargo run -p cf-schema --bin gen-schema   # regenerate schema/ after type changes
pnpm --dir web schema                     # regenerate TypeScript types from schema/
pnpm --dir web build                      # production build
```

---

## Start here

| Doc | Read it for |
|---|---|
| [00-overview.md](docs/00-overview.md) | Scope, non-goals, assumptions, and three corrections to the source proposal |
| [01-architecture.md](docs/01-architecture.md) | System design, tech stack, repo layout, API surface, deployment |
| [02-data-model.md](docs/02-data-model.md) | Venue / Scenario schemas, NavGraph, run artifacts, versioning, DB |
| [03-track-a-venue-designer.md](docs/03-track-a-venue-designer.md) | **Track A** — canvas editor, component library, import pipeline, reporting |
| [04-track-b-simulation-engine.md](docs/04-track-b-simulation-engine.md) | **Track B** — navmesh, ECS, SFM+PBD, analytics, compliance, determinism |
| [05-roadmap-and-risks.md](docs/05-roadmap-and-risks.md) | Phases, dated milestones, Gantt, risk register, academic outputs |
| [06-validation.md](docs/06-validation.md) | RiMEA / ISO 20414 verification and validation strategy |
| [07-infrastructure-and-cost.md](docs/07-infrastructure-and-cost.md) | Free-tier stack, licensing audit, cost model, week-1 checklist |

---

## The shape of it

```
Venue Document ──compile──► NavGraph ──step──► Run artifacts
  editable                  immutable          immutable
  Track A owns              the contract       Track B owns
```

Two tracks, one data contract, four integration milestones.

| | Track A — Designer + Import | Track B — Simulation Engine |
|---|---|---|
| Stack | TypeScript / React / PixiJS, Python / ONNX | Rust → WASM + native |
| Ships | `venue.json`, `scenario.json` | `run/` artifacts, live frame buffer |
| Phases | A1 canvas · A2 semantics · A3 pathflow · A4 vector import · A5 AI import · A6 analysis + reports | B1 navmesh · B2 ECS + locomotion · B3 WASM host · B4 components + modes · B5 analytics + compliance · B6 native scale |

## Milestones

| | When | Demo |
|---|---|---|
| **M1** Vertical slice | 2026-10-02 | Draw a hall with two doors → 500 agents walk out |
| **M2** Design loop closed | 2026-11-27 | Import DXF → add turnstiles → 10k agents @ 60 fps with live heatmap |
| **M3** Full fidelity | 2027-01-29 | Raster import → 2 floors → evacuation mode → NFPA + Green Guide verdict |
| **M4** v1 Beta | 2027-03-26 | Pilot user exports a dossier they'd hand to a licensing authority |

~112 engineer-weeks, ~34 calendar weeks at 4 engineers.

## Three decisions worth knowing up front

1. **Author-time and simulation-time geometry are separate artifacts** joined by an explicit
   compile step. Everything in the architecture follows from this.
2. **One Rust core, two hosts**, with bit-identical results across x86-64, aarch64 and wasm32
   enforced in CI. This is what makes the browser preview and the server report agree.
3. **The simulation runs on the user's machine.** That's a performance decision, a
   $0-infrastructure decision, and a structural margin advantage all at once.

## Source material

`Project Idea_ CrowdFlow Studio.pdf` · `Crowd Simulation Software Project Plan.pdf` ·
`CrowdFlow_Studio.pdf`. Where this plan departs from them, it says so
([00-overview.md §5](docs/00-overview.md)).
