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
- **Scenario authoring** — populations, walking-speed and body-radius
  distributions, entry doors, arrival profile (instant / steady / editable
  curve) and goal. Every control reaches the engine: a run is started from the
  authored scenario, so agents arrive over time rather than all at t=0.
  Anything the document stores that the engine cannot act on is listed verbatim
  under "Not simulated" — a control that edits an ignored field is a lie.
  Undo interleaves venue and scenario edits through `SessionHistory`.

### Next up — pick from the top

The engine is in good shape. **233 tests passing, 3 ignored** — and of those
three, one is a diagnostic tool rather than a test. Every RiMEA case the engine
is capable of satisfying, satisfies.

    cargo test -p cf-sim calibration -- --ignored --nocapture
    cargo test -p cf-sim --test rimea -- --include-ignored

| Measurement | Model | Reference | Error |
|---|---|---|---|
| 1.0 m doorway flow | 82.1 p/m/min | 82 (Green Guide) | **+0%** |
| Speed at ρ = 0.5 /m² | 1.34 m/s | 1.30 (Weidmann) | +3% |
| Speed at ρ = 1.0 /m² | 1.28 m/s | 1.06 (Weidmann) | +21% |
| Speed at ρ = 2.0 /m² | 0.47 m/s | 0.61 (Weidmann) | −23% |
| Speed at ρ = 3.0 /m² | 0.04 m/s | 0.33 (Weidmann) | −89% |

RiMEA TC1, TC3, TC4(curve), TC6, TC7 ×2, TC8 ×2, TC11 ×2, TC12 all pass.

Still ignored, all three honestly:

- **TC2 stairs** — an unimplemented feature, see item 2 below.
- **TC4 fundamental diagram** — the trade-off below.
- **`sweep_agent_repulsion`** — a tool for re-tuning, not an assertion.

### The one trade-off you need to know about

**A single `a_agent` cannot satisfy both doorway flow and the fundamental
diagram.** Run `cargo test -p cf-sim sweep_agent_repulsion -- --ignored
--nocapture` and see for yourself:

| `a_agent` | v(3.0) | 1.0 m door |
|---|---|---|
| 25 (current) | 0.04 | **82.1** |
| 6 | 0.30 | 140.5 |
| 3 | **0.34** | 156.5 |

Weak repulsion lets the density law govern and the diagram comes right, but
agents then pack into a doorway far tighter than people do and flow runs up to
+91%. The model is tuned to the door, because doorway flow sets evacuation time
directly and running *fast* there produces a number a venue gets approved on
and then fails to achieve. Slow at 3 p/m² is the conservative error.

Fixing it properly means separating the two mechanisms — repulsion for
avoidance only, the density law carrying the bulk slowing — rather than one
constant doing both jobs badly at one end.

Ranked, what is left:

1. **Stairs** (RiMEA TC2). `cf_schema::venue::VerticalLink` already carries
   `speed_multiplier_up/_down`, `riser_m` and `going_m`, and `Zone` carries
   `speed_multiplier` — none of it reaches `cf-sim`, which holds one flat
   `NavMesh` with no zone identity. A per-triangle speed multiplier applied in
   `Sim::steer` unlocks TC2 on its own, without multi-floor navigation.

2. **The repulsion/density split above.**

3. **Flow fields** to replace per-agent A*. This matters more than it did:
   `reconsider_exits` issues a path query per exit per reconsideration.

4. **Floorplan import** — `services/` exists only as an unverified sketch on a
   branch. Nothing has ever compiled or linted it.

5. Multi-select and marquee; scenario events (a door closing mid-run) — the
   schema has `events` and the engine ignores them.

### Parallel work parked on branches

Three agents ran in worktrees and were interrupted before finishing. Each was
committed and pushed as-is rather than merged, so `main` stays green. All three
branched from `dba0367` and predate the calibration fixes.

| Branch | State |
|---|---|
| `worktree-agent-ab40b1df98d31158e` | RiMEA suite — **merged**, nothing left on the branch |
| `worktree-agent-af192909cfdd38588` | Scenario authoring — **merged and wired**, nothing left |
| `worktree-agent-aedbe6affb1b5f598` | Floorplan importer under `services/`. **Unverified — never compiled or linted.** Treat as a sketch. |

The importer was written against the old data contract and has never been run.
Re-run the full suite after touching it.

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
