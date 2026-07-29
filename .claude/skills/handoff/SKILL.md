---
name: handoff
description: End the current work session cleanly so the other person's Claude can pick up. Runs checks, updates docs/STATE.md and docs/TASKS.md, commits, and pushes. ONLY use when the user explicitly asks for it — they trigger this themselves when their token budget nears its limit. Never invoke it on your own judgement that a session is ending.
---

# Handoff

Two people alternate on this repo from separate machines and separate Claude accounts.
Whatever is not pushed does not exist for the next session. Your job is to leave the repo
in a state where someone with **zero context** can run `cargo test`, read `docs/STATE.md`,
and be productive within two minutes.

The user invokes this deliberately when their token budget is nearly spent. Do not invoke
it yourself, and do not pre-emptively wind work down in anticipation of it.

Work through these in order. Do not skip step 1 or step 5.

## 1. Get the tree green

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

If anything fails:
- **Small fix?** Fix it now. That is the whole point of handing off green.
- **Big fix?** Do **not** push broken code to `main`. Commit to a branch
  (`wip/<short-description>`), push that, and say so loudly in `STATE.md` under
  "Tree status" and "Next up".

If you changed anything in `engine/cf-schema`, regenerate and include the result:

```bash
cargo run -p cf-schema --bin gen-schema
```

## 2. Update `docs/STATE.md`

This is the file that carries the context across the gap. Rewrite, don't append.

- **Phase** — which phase from `docs/05-roadmap-and-risks.md`.
- **Last updated** — today's date and whose session.
- **Tree status** — `green — N tests passing`, or exactly what is broken and where.
- **Just finished** — what *this session* did. Not cumulative history; `git log` has that.
- **Next up** — a ranked short list. Each item must be actionable with no context.
  Bad: "continue the engine". Good: "add `cf-geom::segment_intersect` with the
  degenerate-case tests listed in `docs/04-track-b-simulation-engine.md` §B1".
  If you were mid-task, say precisely where you stopped and what you were about to do.
- **Open questions** — anything the other person should decide. Include your
  recommendation so they can just agree and move on.
- **Gotchas** — anything you learned the hard way. This section saves the most time.

## 3. Update `docs/TASKS.md`

Tick `[x]` what you finished. Mark `[~]` with your name for anything genuinely mid-flight.
Add any new tasks you discovered. Keep it honest — an over-ticked list is worse than none.

## 4. Write an ADR if you made a real decision

If you made a call that would be expensive to reverse (a library choice, an algorithm, a
schema shape), add `docs/adr/NNNN-short-title.md`:

```markdown
# NNNN — <title>

**Status:** accepted · **Date:** YYYY-MM-DD · **Session:** <whose>

## Context
What forced a decision.

## Decision
What we're doing.

## Consequences
What this makes easy, what it makes hard, and what would make us revisit.

## Alternatives considered
What was rejected and why.
```

## 5. Commit and push

Stage everything relevant, commit with a conventional-commit message, push to `main`.

```bash
git add -A
git status                       # look at it — don't commit junk
git commit -m "feat(scope): summary

Longer body if the change needs explaining.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
git push origin main
```

If the push is rejected, the other person pushed first. Do **not** force. Run
`git pull --rebase origin main`, resolve, re-run the tests, then push. If `docs/STATE.md`
conflicts, merge both sessions' notes rather than picking one.

## 6. Report back

Tell the user, in a few lines:
- what got done and what the tree status is
- the commit hash and that it's pushed
- the single most useful thing for the next session to start on

Then stop. Do not begin new work after a handoff.
