# ADR 0007 — The doorway is correct; the remaining error is bounded

- **Status:** accepted (measurement taken, scope of the known error established)
- **Date:** 2026-08-07
- **Relates to:** ADR 0005 (empirical speed–density coupling), ADR 0006 (two
  rejected fixes)

## Why this was measured

ADR 0006 rejected two attempts at the last calibration gap — anticipatory
avoidance and a softer repulsion falloff — and both had been chosen from theory
rather than from data. The obvious next move was a third theory-led attempt.
Instead the doorway was instrumented, because nobody had ever looked at what an
agent standing in one actually believes.

The hypothesis on record was that the **directional density sensor** reads
artificially low at an opening. An agent there has genuinely clear floor ahead,
so the `(1 + cos θ)/2` weighting discounts most of its neighbours, and it would
then walk faster than the fundamental diagram allows at its true local density.

## What the measurement says

`cf_sim::locomotion::sense_density` was extracted from the force loop so it can
be queried, and `measure_doorway_flow` now samples every body within a metre of
the opening:

```
1.0 m doorway: 82.1 persons/m/min, +0% vs Green Guide 82
  at the opening: sensed 1.70 p/m², actually 2.04 p/m², walking 0.31 m/s
```

**The hypothesis is wrong.** The sensor under-reads by 17%, not by the factor
that would be needed to explain anything. The directional weighting is not the
problem at a doorway.

What it shows instead is more useful:

- The doorway settles at **2.04 persons/m²**, which is what doorway experiments
  report and what the hydraulic calculation assumes.
- It passes **82.1 persons/m/min against a Green Guide figure of 82**.

The doorway is not approximately right. It is right, for the right reason, at
the right density.

> The 0.31 m/s figure is a mixed sample — the one-metre band takes in queueing
> bodies as well as passing ones, so it reads low and is not comparable to
> Weidmann directly. The densities are the reliable part of this measurement.

## Why the remaining gap resists every fix

The force balance at each regime, using the measured densities:

| | surface gap | repulsion | driving force | ratio |
|---|---|---|---|---|
| doorway, ρ = 2.04 | 0.24 m | 1.24 m/s² | 1.18 m/s² | 1.1 : 1 |
| uniform stream, ρ = 3 | 0.117 m | 5.8 m/s² | 0.66 m/s² | 8.8 : 1 |

One constant, two regimes, and only the denser one is broken. That invites
capping the repulsion — legitimate in principle, because non-overlap is
guaranteed by the position-based contact solve and the social force is only a
preference. It was tried:

| ceiling, m/s² | v(ρ=3) | 1.0 m doorway |
|---|---|---|
| 2.5 | 0.08 | 110.7 |
| 5 | 0.04 | 102.3 |
| 10 | 0.04 | 92.8 |
| 20 | 0.04 | **82.1** |
| none | 0.04 | **82.1** |

Monotone, and a dead end. The doorway degrades steadily while the stream barely
moves. Halving the peak force only takes v(3.0) from 0.04 to 0.08 — still four
times short — because at 3 persons/m² the *desired* speed is already down to
0.269 m/s, so the driving term is 0.54 m/s² and even a capped force outweighs
it.

The general result, now measured rather than suspected:

**A doorway at the correct flow and a stalled dense stream overlap in
separation.** The doorway's tight pairs sit at gaps where the force is large;
the stream sits at gaps only slightly smaller. No function of separation alone —
lowering `a_agent`, lengthening `b_agent`, or capping the magnitude — reaches
one without the other. All three have now been swept and all three trade off
monotonically.

## Decision

**Ship as is, and treat the remaining error as bounded rather than open.**

The error lives in uniform streams above roughly 2 persons/m². It does not live
at doorways, and doorways are what set evacuation time — a corridor at 3
persons/m² is downstream of a door that metered the crowd into it. That the
error is *slow* rather than fast keeps it on the conservative side: the reported
egress time is longer than reality, which is the direction a life-safety tool
should err.

RiMEA TC4 stays ignored, with the measured value in its ignore reason.

## What would actually fix it

Not another separation-based term. The two regimes have to be told apart by
something that differs between them, and the measurement says separation is not
it. Candidates, in the order they look promising:

1. **A repulsion that scales with the driving force** rather than being absolute.
   The failure is a *ratio* — 1.1 : 1 works, 8.8 : 1 stalls. A force expressed
   as a fraction of what the agent is currently pushing with would hold that
   ratio at every density by construction. This is speculative and has not been
   tried.
2. **Relative velocity.** A stream is everyone moving alike; a doorway is paths
   converging. ADR 0006 tried time-to-collision and it made the doorway worse —
   but it was tuned to *replace* repulsion, not to modulate it.
3. Accept it, and document the model's validated envelope as ρ ≤ 2 persons/m²,
   which is where every compliance figure this product reports is computed.

Option 3 is close to free and is what the dossier should say today.

Do not attempt 1 or 2 without reading ADR 0006 and the tables above.
