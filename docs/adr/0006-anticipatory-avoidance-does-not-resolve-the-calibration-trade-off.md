# ADR 0006 — Anticipatory avoidance does not resolve the calibration trade-off

- **Status:** rejected (experiment run, result negative, code reverted)
- **Date:** 2026-08-06
- **Supersedes / relates to:** ADR 0005 (empirical speed–density coupling)

## The problem this was meant to solve

A single agent-repulsion constant cannot satisfy both benchmarks at once.

| `a_agent` | v(ρ=3) | 1.0 m doorway |
|---|---|---|
| 25 (current default) | 0.04 m/s | **82.1 p/m/min** |
| 6 | 0.30 | 140.5 |
| 3 | **0.34** | 156.5 |

Reference: Weidmann gives 0.33 m/s at 3 persons/m²; the Green Guide plans at
82 persons/m/min.

The mechanism is straightforward. At 3 persons/m² a crowd sits at 0.117 m
surface gaps. With `a_agent = 25` the anisotropic net force there is about
4.35 m/s² backwards against a driving force of 0.58 m/s², so the equilibrium
speed is negative — the crowd cannot move at all, and measurement agrees
(0.04 m/s). For it to flow at Weidmann's 0.33 m/s the repulsion constant would
have to be about 0.35, which is essentially no social repulsion. But repulsion
is doing real work at a doorway: the arching that holds flow to 82 p/m/min
against a geometric maximum near 350 is genuine jostling, and without it agents
pack tighter than people do.

## The hypothesis

Helbing's repulsion is a function of separation alone, so it cannot distinguish
a neighbour walking away at one's own speed from a neighbour standing still. In
a uniform stream that is fatal — everyone is close, nobody is a threat, and the
model stops the crowd anyway. At a doorway, where paths genuinely converge, the
same force is doing the right thing.

Time-to-collision separates the two cases. Two people in a stream moving alike
will never collide however close they are; two people converging on a doorway
will. So: add an anticipatory term with energy `E(τ) ∝ exp(−τ/τ₀)/τ²` — the
power law Karamouzas et al. (2014) fit across several real crowd datasets — and
then lower `a_agent` until the fundamental diagram comes right, expecting
anticipation to hold the doorway.

## What actually happened

The term was implemented, and one real bug was found on the way: `1/τ²`
diverges as `τ → 0`, and without a floor on τ a dense stream received kicks of
order 10⁶ m/s². That stalled the crowd outright and left bodies overlapping by
up to 0.10 m where the contact solve had previously held overlap at exactly
zero. An imminent collision belongs to the contact solver, not to anticipation;
flooring τ at 0.5 s fixed it and the term became well behaved.

With it behaving, sweeping the anticipation constant at `a_agent = 3`:

| `k_anticipate` | v(ρ=3) | 1.0 m doorway | worst overlap |
|---|---|---|---|
| 0.6 | 0.34 | 130.2 | 0.004 |
| 1.5 | 0.36 | 141.5 | 0.011 |
| 3.0 | 0.33 | 159.3 | 0.058 |
| 6.0 | 0.12 | 173.5 | 0.109 |

**More anticipation makes doorway flow worse, not better.** The reasoning was
backwards: anticipation helps agents step around one another *earlier*, so they
interfere less and stream through an opening more efficiently. It reduces
jostling, and jostling is the thing that was holding flow down to a realistic
figure.

At the shipped defaults (`a_agent = 25`, `k = 0.3`) it also made the headline
benchmark slightly worse — 88.5 p/m/min against 82.1 without — while leaving
the fundamental diagram unchanged.

## Decision

Reverted. The term costs work per neighbour pair per tick and improved no
benchmark. Keeping unmeasured code because it is theoretically attractive is
how a model becomes impossible to reason about.

The trade-off stands, and the model remains tuned to the doorway. Doorway flow
sets evacuation time directly, and being fast there produces a figure a venue
is approved on and then fails to achieve; being slow at 3 persons/m² is the
conservative error.

## What to try instead

Anticipation was the wrong lever because it addresses *how* agents avoid each
other, not *how tightly they will stand*. The quantity that differs between a
doorway and a uniform stream is willingness to be close to a stranger, so the
next thing to try is separating the two radii that are currently one:

- a **body** radius, hard, enforced by the contact solve — anatomy;
- a **personal-space** radius, soft, which the repulsion acts on and which can
  legitimately be larger.

Doorway flow is then governed by personal space, which people surrender under
pressure, while the fundamental diagram is governed by body size, which they
cannot. That is also closer to how the pedestrian literature treats the two.

Do not repeat this experiment without reading the table above.
