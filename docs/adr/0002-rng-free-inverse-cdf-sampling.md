# 0002 — Distributions sample by inverse CDF, without owning an RNG

**Status:** accepted · **Date:** 2026-07-29 · **Session:** Ramzy

## Context

Agent parameters (walking speed, radius, patience, service times, reaction times) are all
drawn from distributions authored in the editor and sampled by the engine. Three
constraints apply at once:

1. **Determinism.** The same seed must produce bit-identical results on x86-64, aarch64
   and wasm32 — this is what makes the in-browser preview and the server report agree,
   and what makes an audit reproducible years later.
2. **One definition per distribution.** The editor draws a histogram preview; the engine
   spawns agents. If they disagree about what `lognormal` means, the preview lies.
3. **Stream stability.** Changing one population's distribution must not shift the random
   numbers every *other* agent receives, or an unrelated edit silently changes the whole
   result.

## Decision

`Distribution::sample_icdf(u: f64) -> f64` maps a uniform `u ∈ [0,1)` to a variate and
takes no RNG. The engine owns the PRNG and decides which stream feeds which sample. The
standard normal uses Acklam's rational approximation (relative error < 1.15e-9).

All transcendental calls go through a private `fmath` module so the crate can be moved
onto a bit-reproducible `libm` in one edit.

## Consequences

**Easier:** the sampling code is pure and trivially testable — the test suite integrates
the quantile function by the midpoint rule and checks it recovers the closed-form mean.
Porting to TypeScript for the editor preview is ~40 lines with guaranteed agreement.
Every distribution consumes **exactly one** uniform, so the PRNG stream stays aligned
regardless of which distribution a population uses.

**Harder:** distributions without a closed-form inverse CDF need a numerical one. None of
the v1 set does.

**Caught a real bug immediately:** `normal_icdf(0.0)` is `-inf`, and uniform PRNGs emit
exactly `0.0`. Unclamped, this would have injected NaN agents into the simulation.
Input is now clamped away from both endpoints.

## Alternatives considered

- **Box–Muller.** Rejected: consumes two uniforms and produces two variates, so the
  number of draws depends on call pattern — breaks stream stability.
- **Passing `&mut Rng` into `sample()`.** Rejected: couples the schema crate to an RNG
  implementation, and makes the editor's preview path drag in the engine's PRNG.
- **Ziggurat.** Faster, but table-driven and harder to guarantee bit-identical across
  targets. Sampling happens at spawn, not in the hot loop, so speed is not the constraint.
