# 0005 — The fundamental diagram is applied, not emerged

**Status:** accepted · **Date:** 2026-07-30 · **Session:** Ramzy

## Context

The Social Force Model is supposed to slow a crowd emergently: repulsion between
bodies makes dense crowds move slowly without anyone telling them to. Measured
against Weidmann's 1993 speed–density relation, ours did not do nearly enough.
At 2 persons/m² the model walked at 1.29 m/s where measurement says 0.61.

Doorway flow showed the same bias: ~118 persons/m/min against the Green Guide's
82, ~45% fast. Both errors point the same way — **the model was optimistic**,
and an optimistic evacuation time is one a venue could be approved on and then
fail to achieve.

Two ways to fix it:

1. **Tune the force constants** until the curve fits.
2. **Apply the published relation directly** as a multiplier on desired speed.

## Decision

Option 2. Local density is measured per agent over a 1 m radius disc, and
desired speed is scaled by Weidmann's `v(ρ)/v₀` before forces are computed.

The forces keep their real job — collision avoidance, lane formation, arching at
exits, all the emergent behaviour SFM is validated for. The empirical curve sets
the pace.

## Consequences

**Easier:** the fundamental diagram matches by construction rather than by
fitting. At 0.5 and 1.0 persons/m² the model now tracks Weidmann within 3%. The
coupling is one interpretable parameter (`density_speed_coupling`, 0 to 1)
rather than a pair of opaque force constants whose effect is only visible after
a full run.

**Harder:** the model is no longer purely first-principles. That is a real
philosophical cost and worth stating plainly — but the fundamental diagram *is*
measured human behaviour, not something a force model should be expected to
re-derive. Every serious evacuation model calibrates against it one way or
another; doing it explicitly is more honest than tuning constants until the same
curve appears.

**Rejected because:** cranking `A` and `B` until the curve fits makes the system
stiff, needs a smaller timestep — which is unaffordable at 25,000 agents — and
buys a fit at one density by distorting behaviour at every other.

**Not yet resolved.** At 2 persons/m² the model still reads far too fast, and
doorway flow has swung from +45% to −49%. The coupling is right in principle and
the low-density end now matches; the parameters are not calibrated. The
measurements live in `cf_sim::calibration` and their assertions are `#[ignore]`d
with the reason, so the harness states the target rather than being weakened
until it passes.

## What the harness found on the way

Writing the measurement was worth more than the change it motivated.

**A contact solver that made dense crowds faster.** Velocity was being folded
into each contact correction as it was applied — `correction / dt` per pair per
iteration. With three iterations and many neighbours that injects several
impulses per step, and at 2 persons/m² the crowd walked at 2.14 m/s: above free
walking speed, pinned exactly at the speed cap. Position-based dynamics does not
work that way. Constraints move positions; velocity is derived once from the net
displacement. A projection cannot add energy — which is the property that made
PBD worth choosing in the first place, and which the implementation was
violating.

**Two harnesses that measured the wrong thing.** The first packed a closed room
and aimed everyone at a goal outside the mesh; pathfinding failed, agents walked
into a wall, and the "speed" recorded was how fast a stationary pile jitters —
producing the impossible result of walking speed *rising* with density. The
second used a long corridor and let the crowd disperse into it, so density fell
from the target the moment the run began and every reading came back near free
speed. A fundamental diagram needs a periodic domain, which is why the real
experiments use one.

Each of those looked like a model defect and was a measurement defect. The
lesson worth carrying: when a harness reports something physically impossible,
suspect the harness first.
