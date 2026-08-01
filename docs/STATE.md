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

1. **Calibrate the locomotion model.** This is the highest-value work in the
   project: every figure the tool reports inherits it, and the dossier currently
   carries a caution saying so.

   Run the harness: `cargo test -p cf-sim calibration -- --ignored --nocapture`

   Current readings against the references:

   | Measurement | Model | Reference | Error |
   |---|---|---|---|
   | Speed at ρ = 0.5 /m² | 1.33 m/s | 1.30 (Weidmann) | +3% |
   | Speed at ρ = 1.0 /m² | 1.03 m/s | 1.06 (Weidmann) | −3% |
   | Speed at ρ = 2.0 /m² | **2.14 m/s** | 0.61 (Weidmann) | **+253%** |
   | 1.0 m doorway flow | **41.7 p/m/min** | 82 (Green Guide) | **−49%** |
   | 0.9 m doorway flow | 16.7 p/m/min | 82 | −80% |
   | 1.8 m doorway flow | 90.8 p/m/min | 82 | +11% |

   Low density now tracks Weidmann well. Two things are still wrong:
   - **ρ = 2.0 reads above free walking speed**, which is impossible. Suspect the
     periodic harness first (it has been wrong twice — see ADR 0005), then the
     density sensing radius, then the speed cap interacting with the contact solve.
   - **Narrow doorways are far too slow, wide ones about right.** A 0.9 m door
     barely passes anyone. Likely the wall repulsion constant is too large
     relative to a body — an agent cannot get close enough to a 0.9 m opening to
     use it. Try reducing `a_wall`, or making wall repulsion fall off from the
     *surface* rather than the centre.

   Tunables are all in `LocomotionParams`. Change one at a time and re-run the
   harness; the numbers above are the baseline to beat.

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
