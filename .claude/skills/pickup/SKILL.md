---
name: pickup
description: Start a work session on this repo by loading context from where the other person left off. Use at the beginning of a session, or when the user says "pickup", "what's next", "where were we", "continue from where we left off", or takes over from their collaborator.
---

# Pickup

Two people alternate on this repo. You are starting cold. Spend ninety seconds loading
real context before touching anything — guessing costs far more than reading.

## 1. Sync and orient

```bash
git pull --rebase origin main
git log --oneline -15
git status
```

If `git status` is not clean, something was left mid-flight. Read the diff before deciding
whether to keep or discard it, and ask the user if it isn't obvious.

## 2. Read the handoff note

Read `docs/STATE.md` in full. It is authoritative — it tells you the phase, what the last
session finished, what to do next, open questions, and gotchas. Read the **Gotchas**
section properly; it exists so you don't re-discover the same traps.

Then skim `docs/TASKS.md` for anything marked `[~]` (someone was mid-task on it).

## 3. Verify the tree

```bash
cargo test
```

Compare against the "Tree status" line in `STATE.md`.
- **Green and STATE says green** — good, proceed.
- **Red but STATE says green** — something broke between sessions. Investigate and tell
  the user before doing anything else. Do not start new work on a broken tree.
- **Red and STATE says red** — fixing it is your first task.

## 4. Pick the work

Take the top item from **Next up** in `STATE.md` unless the user says otherwise. If that
item is ambiguous or turns out to be larger than it looked, say so and propose a smaller
first slice rather than starting something sprawling.

For deeper background on whatever you picked:
- `docs/03-track-a-venue-designer.md` — canvas, components, import, reporting
- `docs/04-track-b-simulation-engine.md` — navmesh, ECS, physics, analytics, compliance
- `docs/02-data-model.md` — the schemas
- `docs/adr/` — why things are the way they are

Read only the section you need. These documents are long; loading all of them wastes
budget that is better spent on the work.

## 5. Confirm before you build

Tell the user in three or four lines: what state you found the repo in, what you're
picking up, and roughly what it involves. Then start.

Keep the working slice small enough to finish, test, and commit within this session —
budget for `/handoff` at the end.
