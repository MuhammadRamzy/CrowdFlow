# STATE — living handoff note

> **Both sessions read this first and update it last.** Keep it short and current.
> If it disagrees with your assumptions, it wins. Stale entries are worse than none —
> delete rather than accumulate.

---

## Right now

**Phase:** B2 — locomotion, essentially complete. Engine and editor work end to
end, the model meets its headline benchmark (82.1 p/m/min through a 1 m door
against the Green Guide's 82), and the whole RiMEA suite runs. One calibration
gap remains; see "Next up".
**Last updated:** 2026-08-06 by Ramzy's session
**Tree status:** green — 250 tests passing, 2 ignored, and one of those two is a
diagnostic tool rather than a test. clippy clean, wasm32 builds, web typecheck
and production build clean.

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
- **Terrain speed** — a zone with a `speed_multiplier` slows the people
  crossing it. Free on a venue that has none: `NavMesh::uniform_speed` is
  settled at build time and short-circuits the whole pass.
- **Scenario authoring** — populations, walking-speed and body-radius
  distributions, entry doors, arrival profile (instant / steady / editable
  curve) and goal. Every control reaches the engine: a run is started from the
  authored scenario, so agents arrive over time rather than all at t=0.
  Anything the document stores that the engine cannot act on is listed verbatim
  under "Not simulated" — a control that edits an ignored field is a lie.
  Undo interleaves venue and scenario edits through `SessionHistory`.

### Next up — pick from the top

The engine is in good shape. **250 tests passing, 2 ignored** — and one of those
two is a diagnostic tool rather than a test. Every RiMEA case the engine
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

RiMEA TC1, TC2, TC3, TC4(curve), TC6, TC7 ×2, TC8 ×2, TC11 ×2, TC12 all pass.

Only two things are ignored, and one is not a test:

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

**The doorway was instrumented and it is correct** — 2.04 persons/m², 82.1
p/m/min. ADR 0007 has the numbers. That reframes the remaining gap: the error
lives only in **uniform streams above ~2 persons/m²**, which are downstream of a
door that already metered the crowd into them, and it is *slow* rather than
fast, which is the conservative side for a life-safety tool.

**Three fixes have now been swept and all three trade off monotonically.** Read
ADR 0006 and ADR 0007 before spending a session here.

- *Anticipatory (time-to-collision) avoidance* — makes the doorway **worse**; it
  removes the jostling that holds flow to a realistic figure.
- *Softening the falloff* — `b_agent` 0.08 → 0.55 degrades the doorway
  monotonically, 82 → 231 p/m/min, and does not improve the diagram at all.
- *Capping the repulsion* — degrades the doorway 82 → 111 while the stream
  barely moves, 0.04 → 0.08.

The measured reason they all fail: **a correct doorway and a stalled dense
stream overlap in separation.** No function of separation alone reaches one
without the other.

Ranked, what is left:

1. **Scenario events.** `cf_schema::scenario::TimedEvent` carries `closeOpening`,
   `openOpening`, `alarm` and `blockLink`, and the engine acts on none of them —
   the authoring panel lists them under "Not simulated". *What if this exit is
   blocked* is the question a fire-safety engineer actually asks, so this is the
   highest-value thing the engine cannot currently answer.

   Closing an opening needs more than dropping it from `Sim::exits`: the edge is
   unconstrained in the triangulation, so agents would still walk through the
   gap and off the mesh. It has to be sealed — put the edge back in
   `tri.constraints`, rebuild adjacency and portals, add the segment to
   `Sim::walls` — and every route through it re-planned. `replan_the_stuck` and
   `reconsider_exits` already exist to absorb the aftermath.

2. **Multi-floor navigation.** Stairs slow people down, but `Sim` holds one flat
   `NavMesh` with no floor identity, so an agent cannot traverse a
   `VerticalLink`. `docs/06-validation.md` §3 needs this.

3. **Flow fields** to replace per-agent A*. Matters more than it did:
   `reconsider_exits` issues a path query per exit per reconsideration.

4. **Floorplan import** — `services/` is an unverified sketch on a branch.

5. Multi-select and marquee.

### A standing caveat: none of the UI has been driven

No browser tool has been attached to any session that built the frontend. The
scenario panel, the report's verification statement and the undo interleaving
have been typechecked and built but **never clicked**. Six of this project's
frontend bugs were found only by driving the real UI, so treat that as unproven
rather than working.

The mitigation so far is `engine/cf-wasm/tests/end_to_end.rs`, which calls
exactly what JS calls — including the scenario path the editor now uses for
every run. If that passes, what is left to break is the JS glue, not the engine.
Anyone with a browser should open the app and exercise: draw a wall, edit a
population, run, open the report.

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
