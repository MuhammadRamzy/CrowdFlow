# CLAUDE.md — read this first

CrowdFlow Studio: browser-based crowd simulation and venue modelling.
Rust/WASM engine, React canvas editor, Python import pipeline.

**Two people work on this repo, alternating.** Ramzy and his collaborator each drive a
separate Claude session from a separate machine. When one runs out of tokens, the other
takes over. You may be either one, mid-stream, with no memory of what came before.

---

## 1. Before you do anything

1. Read **`docs/STATE.md`** — the living handoff note. It says what phase we're in, what
   the previous session just finished, and what to do next. Trust it over your assumptions.
2. Run `git log --oneline -10` to see recent work.
3. Run `cargo test` to confirm the tree is green before you change anything. If it is
   already red, fix that first and say so — do not build on a broken tree.

## 2. Before you stop

Run `/handoff` (see `.claude/skills/handoff/`). It updates `docs/STATE.md`, runs the
checks, commits, and pushes. **A session that ends without a push is work the other person
cannot continue.** If tokens are running low, handoff early rather than risk losing it.

---

## 3. Where things live

| Path | What |
|---|---|
| `docs/STATE.md` | **Living handoff note.** Current phase, last done, next up, open questions. |
| `docs/TASKS.md` | Shared backlog with checkboxes. Tick items as you finish them. |
| `docs/00-overview.md` … `07-*.md` | The plan. Scope, architecture, data model, both track breakdowns, roadmap, V&V, infra. |
| `docs/adr/` | Architecture Decision Records. Add one when you make a call that would be expensive to reverse. |
| `schema/` | **Generated** JSON Schema. Never hand-edit — regenerate. |
| `engine/cf-schema/` | Source of truth for all document types. |
| `fixtures/` | Shared test venues, used by both tracks. |

Full document map: `README.md`.

## 4. The two rules that matter most

**R1 — `engine/cf-schema` is the single source of truth for the data contract.**
JSON Schema, TypeScript types and Pydantic models are all *generated* from it. If you
change a type there, run `cargo run -p cf-schema --bin gen-schema` and commit the
regenerated `schema/` files in the same commit. Never hand-edit `schema/*.json`.

**R2 — The simulation must be deterministic.** Same seed + same inputs → bit-identical
results on x86-64, aarch64 and wasm32. In `cf-sim` and anything it depends on:
- no `HashMap` iteration, no unordered `rayon` reductions
- all transcendentals (`ln`, `exp`, `sin`, …) go through the crate's `fmath` module
- seeded PRNG only; never `rand::thread_rng`
- see `docs/04-track-b-simulation-engine.md` §5 for the full hazard list

## 5. Conventions

So that code written in two different sessions reads like one person wrote it.

**Rust**
- `cargo fmt` before committing. `cargo clippy -- -D warnings` must pass.
- Doc comments (`///`) on every public item. Explain *why*, not what the code already says.
- Errors: `thiserror` for libraries. No `unwrap()` outside tests and `main`.
- Tests live next to the code (`#[cfg(test)] mod tests`) for units; `tests/` for anything
  crossing crate or fixture boundaries.
- Wire format is **uniformly camelCase**. Remember `rename_all` on an enum renames
  *variants*, not their fields — struct variants need their own attribute.

**TypeScript** (once `web/` exists)
- Strict mode, no `any`. Types for documents come from `schema/`, never hand-written.
- Canvas rendering stays out of React. React owns chrome; PixiJS owns the scene.
- Every document mutation goes through a `Command` — never write to the store directly.

**Python** (once `services/` exists)
- `ruff` + `mypy --strict`. Pydantic v2 models generated from `schema/`.

**Commits**
- Conventional commits: `feat(cf-schema): …`, `fix(canvas): …`, `docs: …`, `chore: …`.
- Small and green. Never commit a red tree.
- Co-author trailer is on by default; leave it.

## 6. Commands

```bash
cargo test                                   # everything
cargo test -p cf-schema                      # one crate
cargo run -p cf-schema --bin gen-schema      # regenerate schema/ after type changes
cargo fmt && cargo clippy -- -D warnings     # before every commit
```

## 7. Working style for this project

- **Finish what you start.** Prefer one complete, tested, committed slice over three
  half-finished ones. The next session cannot infer your intent from unfinished code.
- **Write the test first when the behaviour is checkable.** Several real bugs in this repo
  were caught by tests written before the fix.
- **When you hit a decision that isn't in the docs**, make the call, write an ADR, and note
  it in `STATE.md` under "Open questions" if the other person should weigh in.
- **Don't silently expand scope.** `docs/00-overview.md` §3 lists what's deliberately out
  of v1. If you think something needs to move in, say so rather than just building it.
- **Licensing:** no AGPL/GPL/non-commercial dependencies anywhere in the product path.
  Ultralytics YOLO and the original SegFormer weights are both ruled out —
  `docs/07-infrastructure-and-cost.md` §5.
