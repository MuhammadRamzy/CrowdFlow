# STATE — living handoff note

> **Both sessions read this first and update it last.** Keep it short and current.
> If it disagrees with your assumptions, it wins. Stale entries are worse than none —
> delete rather than accumulate.

---

## Right now

**Phase:** P0 — Foundations (target: W1–W3)
**Last updated:** 2026-07-29 by Ramzy's session
**Tree status:** green — 21 tests passing

### Just finished

- Full planning doc set in `docs/` (8 documents, architecture through V&V and infra).
- `engine/cf-schema` crate: the data contract. Venue + Scenario documents, geometry
  primitives, distributions with RNG-free inverse-CDF sampling, structural/referential
  validation. 21 tests green.
- Generated JSON Schema committed to `schema/`.
- First shared fixture: `fixtures/unit/hall-two-doors.{venue,scenario}.json` — this is the
  M1 target venue (draw a hall with two doors, 500 agents walk out).
- Repo, `CLAUDE.md`, handoff tooling, CI.

### Next up — pick from the top

1. **Codegen: TypeScript types** from `schema/*.json` into `web/src/schema/`.
   Use `json-schema-to-typescript`. Add an npm script and wire it into CI gate G1
   (regenerate → `git diff --exit-code`).
2. **Codegen: Pydantic models** from `schema/*.json` into `services/api/`.
   Use `datamodel-code-generator`.
3. **`cf-geom` crate** — start of Track B / phase B1. Robust predicates, segment
   intersection, polygon ops. This is the critical path to M1.
4. **Round-trip test across languages** — same fixture parsed by Rust, TS and Python,
   asserting identical structure. This is what makes gate G1 real rather than aspirational.

Items 1 and 2 are small and unblock the other track. Item 3 is the critical path.
If you only have budget for one thing, do item 3.

### Open questions

*(Nothing blocking. Add anything the other person should weigh in on, with your
recommendation so they can just agree.)*

- Repo visibility: `docs/07-infrastructure-and-cost.md` §4.4 recommends making `engine/`
  public (free CI, supports the papers) but filing provisionals **before** any public push.
  **Needs a decision from both of you + VIT's IP office before the repo goes public.**
  Until then, keep it private.

### Gotchas discovered — don't rediscover these

- `#[serde(rename_all)]` on an **enum** renames variants, **not** their fields. Struct
  variants with multi-word fields (e.g. `Lognormal { mu_ln }`) need their own
  `#[serde(rename_all = "camelCase")]`. The test `every_schema_property_is_camel_case`
  now guards this.
- `Distribution::sample_icdf(0.0)` on an unbounded Normal used to return `-inf`. Uniform
  PRNGs do emit exactly 0.0, so this would have injected NaN agents. Input is now clamped
  away from **both** endpoints (`U_EPS`).
- `schemars` is pinned at 0.8 (1.x has a different API). If you upgrade, the manual
  `#[schemars(with = ...)]` on `Vec2` needs revisiting.

---

## How to update this file

At the end of a session, `/handoff` will prompt you through it. By hand:

1. Set **Phase**, **Last updated**, **Tree status**.
2. Replace **Just finished** with what *this* session did (not cumulative history — that's
   what `git log` is for).
3. Rewrite **Next up** as a short ranked list. Be specific enough that someone with zero
   context can start. "Continue the engine" is useless; "add `cf-geom::segment_intersect`
   with the degenerate-case tests from B1" is useful.
4. Add anything learned the hard way to **Gotchas**.
5. Commit and push.
