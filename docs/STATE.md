# STATE — living handoff note

> **Both sessions read this first and update it last.** Keep it short and current.
> If it disagrees with your assumptions, it wins. Stale entries are worse than none —
> delete rather than accumulate.

---

## Right now

**Phase:** B2 — locomotion. Engine and editor both work end to end; the model is
**not calibrated**, which is the top open item.
**Last updated:** 2026-07-30 by Ramzy's session
**Tree status:** green — 219 tests passing, 3 deliberately ignored (see below),
clippy clean, wasm32 builds, web typecheck and production build clean.

### How to run it

```bash
cd web && pnpm install     # once
pnpm engine                # once, and after any Rust change — builds the wasm
pnpm dev                   # → http://localhost:5173
```

`web/src/engine/` is gitignored, so a fresh clone **must** run `pnpm engine`
first or the app shows "Engine failed to start". Full instructions in `README.md`.

### Just finished

The frontend is real and drives the real engine — nothing on screen is mocked.

- **`cf-wasm`** bindings; positions cross as flat typed arrays, documents as JSON.
- **Workspace**: PixiJS canvas outside React, pan/zoom, live validation panel,
  transport controls, density heatmap with banded legend.
- **Authoring**: wall / zone / door tools, selection, property editing, undo/redo
  with coalescing, grid + vertex snapping. Every mutation is a `Command`.
- **Life-safety status bar** — the signature. Latches peak occupancy, so an
  overcrowding event that has cleared is still reported.
- **Compliance dossier** — findings against NFPA 101 and the Green Guide with the
  arithmetic shown, recommendations with computed shortfalls, venue plan and
  peak-density figures, verification statement. Prints to PDF via the browser.
- **Drawn SVG icon set** replacing Unicode glyphs (which fell back to
  missing-glyph boxes on Linux).
- **`cf_sim::calibration`** — doorway-flow and speed–density harnesses.

### Next up — pick from the top

The locomotion model **now meets its headline benchmark.** Three real bugs came
out of the calibration work; the numbers below are current.

    cargo test -p cf-sim calibration -- --ignored --nocapture
    cargo test -p cf-sim --test rimea -- --ignored --nocapture

| Measurement | Model | Reference | Error |
|---|---|---|---|
| 1.0 m doorway flow | 74.8 p/m/min | 82 (Green Guide) | −9% |
| Speed at ρ = 0.5 /m² | 1.34 m/s | 1.30 (Weidmann) | +3% |
| Speed at ρ = 1.0 /m² | 1.23 m/s | 1.06 (Weidmann) | +17% |
| Speed at ρ = 2.0 /m² | 0.42 m/s | 0.61 (Weidmann) | −31% |
| Speed at ρ = 3.0 /m² | 0.04 m/s | 0.33 (Weidmann) | −89% |
| 0.9 m doorway flow | 47.3 p/m/min | 82 | −42% |
| 1.8 m doorway flow | 92.7 p/m/min | 82 | +11% |

What was wrong, all three found by measurement rather than by reading code:

- **A doorway was a capsule, not a door.** Exit despawned anyone within 0.6 m of
  the span, reaching back into the room, so no queue ever formed. 281 p/m/min.
- **`a_agent` was Helbing's 2000 *newtons* used as an acceleration**, a factor of
  body mass out. At ρ = 2 the crowd gridlocked and boiled sideways at 1.18 m/s.
- **The fundamental diagram averaged |v|**, so that boiling was reported as
  *speed*: "+253% too fast" was really 0.00 m/s of transport. It now reports the
  component along the flow, with lateral motion and overlap beside it.

Ranked, what is left:

1. **Corner and merge deadlock.** RiMEA TC3, TC6 and TC12 are ignored because
   agents pile up at corners and never clear them (13 of 20, 3 of 10, 32 of 40).
   This is the biggest remaining correctness problem — a venue with a corner is
   every venue. Suspect the funnel path hugging the inside corner so tightly that
   bodies cannot fit, or wall repulsion at a reflex vertex.

2. **Density above ρ ≈ 2 is far too slow** (−89% at ρ = 3). The coupling
   over-suppresses once the sensed disc saturates. Probably wants the sensed
   density clamping, or the coupling applying to a *target* rather than a
   ceiling.

3. **Specific flow is not width-independent** — 1.96× between 0.9 m and 1.8 m.
   Narrow doors are under-served, which is the safe direction but still wrong,
   and 0.9 m is the common case in a real venue.

4. **Scenario authoring UI** — populations, arrival curves, entries, goals.
   Foundation exists on a branch, see below.

5. Flow fields to replace per-agent A*; import pipeline; multi-select.

### Parallel work parked on branches

Three agents ran in worktrees and were interrupted before finishing. Each was
committed and pushed as-is rather than merged, so `main` stays green. All three
branched from `dba0367` and predate the calibration fixes.

| Branch | State |
|---|---|
| `worktree-agent-ab40b1df98d31158e` | RiMEA suite — **already merged into main**, nothing left on the branch |
| `worktree-agent-af192909cfdd38588` | Scenario authoring: `cf-wasm/src/scenario.rs` (1053 lines, 10 tests, compiles), `web/src/doc/scenario.ts`, `ArrivalPlot.tsx`. **No panel, not wired into `App.tsx`.** Finish by adding `ScenarioPanel.tsx` and mounting it. |
| `worktree-agent-aedbe6affb1b5f598` | Floorplan importer under `services/`. Unverified — never compiled or linted. Treat as a sketch. |

Before merging either, re-run the suite: they were written against the old exit
semantics, which is exactly what caught the deadlock regression.

2. **Flow fields** (`cf-navmesh`) — replace per-agent A\* with one Dijkstra per
   goal over the triangle dual. This is what makes 25k agents feasible; the
   current path is O(agents × search).

3. **Scenario authoring** — arrival curves and populations exist in the schema
   (`cf-schema::scenario`) but the editor cannot build one yet.

4. **Multi-select and marquee** in the canvas.

### Open questions

- **Repo visibility.** Still public. `docs/07-infrastructure-and-cost.md` §4.4
  recommends filing provisionals before any public push. Needs a decision from
  both of you plus VIT's IP office.
- **Is the empirical speed–density coupling acceptable?** ADR 0005 argues yes and
  explains why tuning force constants instead is worse. If your collaborator
  disagrees, that is the decision to revisit before calibrating further.

### Gotchas discovered — don't rediscover these

**Calibration and physics**

- **When a harness reports something physically impossible, suspect the harness.**
  Two of my speed–density harnesses measured the wrong thing and both looked like
  model defects: one measured a pile of agents jittering against a wall (speed
  appeared to *rise* with density), the other measured a dispersing crowd (every
  density read near free speed). A fundamental diagram needs a **periodic domain**.
- **PBD derives velocity once, from net displacement.** Folding each contact
  correction into velocity as it is applied injects energy — a dense crowd walked
  *faster* than an empty one. `derive_velocity_from_positions` now runs after all
  constraints. Do not reintroduce per-correction velocity updates.
- Contact corrections are capped at one body radius per iteration; without that,
  many simultaneous neighbours can teleport a body past a wall corner.
- Walls are a **hard** constraint applied before *and* after the agent solve.
  Soft repulsion alone loses agents through corners under load.

**Build and repo hygiene**

- **Never put authored source in `web/src/engine/`.** `wasm-pack` writes its own
  `.gitignore` containing `*` into its output directory, so anything placed there
  is invisible to git — `git status` stays clean and `git add -A` silently skips
  it. The typed bridge lived there for several sessions, was never committed, and
  **the pushed repo could not be built from a fresh clone**. Nothing caught it
  because every local check passed; it surfaced only when the directory was
  deleted to test the fresh-clone path. The bridge now lives at `web/src/engine.ts`.
- The general rule: **generated output and authored source never share a
  directory.** If a tool owns a directory, assume it owns the ignore rules too.
- Worth repeating occasionally: `git ls-files web/src` shows what a collaborator
  would actually receive. That is the check, not `git status`.

**Frontend**

- **Effects that attach to the renderer must depend on `rendererReady`.** The
  renderer mounts asynchronously; an effect running on first render sees `null`,
  returns early, and never re-runs. Canvas input worked only after switching
  tools until this was found.
- A diagnostic that calls `handler?.(...)` **hides a null handler** and looks like
  "events arrive but the handler is broken". Cost two rounds of misdirection.
- Click selects, drag beyond 4 px pans. Select mode still needs pointer events —
  forwarding them only while a drawing tool is active makes selection impossible.

**Schema**

- `#[schemars(with = ...)]` at *container* level is silently ignored by the
  derive. `Vec2` declared an object `{x, y}` while serde wrote `[x, y]`; every
  Rust test passed and only the generated TypeScript exposed it. There is now a
  test asserting declared shapes match what `Serialize` emits.
- `rename_all` on an **enum** renames variants, not their fields.
- `sample_icdf(0.0)` on an unbounded normal returns `-inf`; input is clamped away
  from both endpoints.

**Build**

- `cf-geom`'s serde is a default-on feature so `cf-sim` can drop it from the wasm
  bundle. **Field-level** attributes need `cfg_attr` too, not just derives.
- Clippy rejects methods named `add`/`sub`. `Vec2` implements the real operator
  traits.

---

## How to update this file

At the end of a session, `/handoff` will prompt you through it. By hand:

1. Set **Phase**, **Last updated**, **Tree status**.
2. Replace **Just finished** with what *this* session did — not cumulative
   history, which `git log` already has.
3. Rewrite **Next up** as a short ranked list, specific enough that someone with
   zero context can start.
4. Add anything learned the hard way to **Gotchas**.
5. Commit and push.
