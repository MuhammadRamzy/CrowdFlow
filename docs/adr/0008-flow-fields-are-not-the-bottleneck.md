# ADR 0008 — Flow fields are not the bottleneck; measure before optimising

- **Status:** accepted (measurement taken, roadmap reordered)
- **Date:** 2026-08-07
- **Relates to:** `docs/00-overview.md` (the 25k-agent commitment),
  `docs/04-track-b-simulation-engine.md` §B4 (flow fields)

## What was believed

The roadmap has always had flow fields as the answer to scale. `sim.rs` says so
in its own module docs:

> phase B4 replaces it with flow fields, which compute one potential per *goal*
> rather than one path per *agent* — the difference between O(goals × mesh) and
> O(agents × search), and the reason 100k agents is feasible at all.

Congestion-aware rerouting then made per-agent pathfinding more expensive
(a query per exit per reconsideration), and `egressDistribution` runs a whole
scenario ten times. Flow fields moved to second on the list.

Nothing had ever been measured.

## What the measurement says

`engine/cf-sim/tests/scale.rs`, native release build, agents at 1.4 persons/m²
in a hall with two exits:

| agents | ms/step | µs/agent | × real time |
|---|---|---|---|
| 377 | 0.45 | 1.18 | 112 |
| 1,740 | 2.17 | 1.25 | 23.0 |
| 4,656 | 5.79 | 1.24 | 8.6 |
| 9,384 | 11.39 | 1.21 | 4.4 |
| 24,089 | 31.10 | 1.29 | **1.6** |

Two things fall out.

**Cost is linear in agent count** — 1.2 µs per agent across a 64× range. The
spatial grid is doing its job; there is no hidden O(n²) waiting at scale.

**Rerouting is noise.** Disabling it entirely:

| agents | with | without | share of step |
|---|---|---|---|
| 1,740 | 2.22 ms | 2.32 ms | −4.7% (i.e. inside the noise) |
| 9,384 | 12.84 ms | 12.46 ms | 2.9% |

**Flow fields would buy about 3%.** The step is dominated by the per-agent
force and contact work, not by pathfinding, and replacing A* with a potential
field does nothing about that.

## A correction worth recording

The first version of this benchmark measured a step against a 16.7 ms frame and
concluded the engine was 1.7× short of target. That was the wrong budget and it
*understated* the engine threefold.

The physics runs at 20 Hz and rendering interpolates between ticks rather than
driving them — `sim.rs` says exactly this in its own module documentation. A
step therefore has the whole 50 ms tick interval to keep up with real time, not
one display frame. At 60 fps that is one step every three frames.

Measured against the correct budget the engine runs **1.6× real time at 24k
agents**, natively. It meets the commitment rather than missing it.

## Decision

**Flow fields drop down the list.** They remain right eventually — one
potential per goal is the better shape, and it is what makes rerouting free
rather than merely cheap — but they are not what stands between this engine and
its stated numbers, and doing them now would be a large, determinism-sensitive
change for a 3% win.

What would actually buy time, in the order the measurement suggests:

1. **SIMD force kernel** (already planned, B3). The per-agent force loop is the
   step. `simd128` with a scalar fallback that is bit-identical — the
   determinism contract makes that a real constraint, not a footnote.
2. **Threads** (B3, `wasm-bindgen-rayon`). Linear scaling means the work
   partitions cleanly, but R2 forbids unordered reductions, so the partition
   has to be deterministic.
3. Only then flow fields.

## The caveat on these figures

Native release, one machine, one geometry. **Wasm typically runs 1.5–2× slower
than native**, which puts 24k agents at roughly 0.8–1.1× real time in a browser
— at or just under the line, before any of the optimisations above. That is
close enough that the in-browser number should be measured directly rather than
inferred from this table, and it has not been.

Nobody should quote 25k in-browser until someone has run it in a browser.
