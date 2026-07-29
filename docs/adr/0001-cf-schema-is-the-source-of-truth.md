# 0001 — `cf-schema` is the single source of truth for the data contract

**Status:** accepted · **Date:** 2026-07-29 · **Session:** Ramzy

## Context

Three languages consume the Venue and Scenario documents: Rust (engine), TypeScript
(editor) and Python (import/API). All three need types that agree exactly. Any drift
between them shows up as a runtime parse failure in someone else's session, usually far
from the change that caused it — the worst possible failure mode for a two-person project
where sessions alternate.

Two directions were possible: hand-write JSON Schema and generate all three languages
from it, or pick one language as canonical and generate the rest.

## Decision

The Rust crate `engine/cf-schema` is canonical. `schemars` derives JSON Schema from the
Rust types; `schema/*.json` is generated and committed; TypeScript and Pydantic types are
generated from those JSON Schema files.

CI gate G1 regenerates the schema and fails if the working tree changes, so a type change
without a regenerated schema cannot merge.

## Consequences

**Easier:** one place to change a type. serde attributes give exact control over the wire
format. The engine — the most correctness-critical consumer — works with native types
rather than generated ones. Validation logic lives next to the types it validates and is
shared by every Rust consumer.

**Harder:** the editor and API cannot add a field without touching Rust. That friction is
deliberate; the contract belongs to the project, not to whoever needs a field today.

**Revisit if:** the frontend starts needing document extensions the engine genuinely does
not care about. The likely answer then is a separate `ui-state` document rather than
loosening this one.

## Alternatives considered

- **Hand-written JSON Schema as canonical.** Rejected: JSON-Schema-to-Rust tooling
  produces poor Rust, and the engine is where type quality matters most.
- **Protobuf / FlatBuffers.** Rejected for the *authored* document: it is human-edited in
  fixtures and reviewed in pull requests, so it wants to be readable JSON. Binary formats
  are still the right answer for the compiled `NavGraph` and run artifacts, which are
  machine-only.
