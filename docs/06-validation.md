# Verification & Validation Strategy

A crowd simulator that produces plausible-looking animations is a toy. One that can be shown
to reproduce measured human behaviour is an engineering tool. This document is the difference,
and it is also the strongest academic contribution the project can make.

**Verification** — are we solving the equations correctly?
**Validation** — are they the right equations?

Governing reference: **ISO 20414:2020** *Fire safety engineering — Verification and validation
protocol for building fire evacuation models*. We follow its structure so the evidence pack maps
onto something an authority having jurisdiction (AHJ) already recognises.

---

## 1. The evidence pack

Everything below produces artifacts committed to `engine/cf-testkit/` and rendered into a public
**Verification & Validation Report** that ships with the product. Every generated compliance
dossier cites its version.

| Level | Question | Method | Gate |
|---|---|---|---|
| **V0 Analytical** | Do the primitives compute what they claim? | Unit + property tests vs closed-form results | Every PR |
| **V1 Component** | Does each behaviour reproduce its specified value in isolation? | RiMEA component tests (§2) | Every PR |
| **V2 Functional** | Do agents make sensible decisions? | RiMEA functional tests (§3) | Nightly |
| **V3 Quantitative** | Do emergent statistics match measured human data? | Fundamental diagram, specific flow (§4) | Nightly + release |
| **V4 End-to-end** | Does a full building evacuation match a real recorded one? | Published evacuation datasets (§5) | Release |
| **V5 Cross-model** | Do we agree with established simulators? | Comparison vs published Pathfinder/buildingEXODUS results (§6) | Release |
| **V6 Determinism** | Do repeat runs and different hosts agree? | CI gate G2 (`04` §5) | Every PR |
| **V7 Sensitivity** | Which parameters actually drive the answer? | Monte Carlo + Sobol indices (§7) | Release |

---

## 2. V1 — RiMEA component verification

RiMEA (*Richtlinie für Mikroskopische Entfluchtungsanalysen*) defines the de-facto acceptance
suite for microscopic evacuation models. We implement all of it as fixtures in
`fixtures/rimea/`, each an automated test with a numeric tolerance.

> Implementation note: numbering and exact tolerances must be taken from the current published
> RiMEA guideline document at implementation time — do not rely on secondary summaries. The
> thematic grouping below is what the suite covers.

**Component tests — each isolates one behaviour:**

| Theme | What is verified | Pass criterion |
|---|---|---|
| Walking speed in a corridor | An agent traverses a 2×40 m corridor at its assigned free speed | Time within ±2% of `length / desired_speed` |
| Walking speed on stairs | Assigned speed reduction applied going up and down | Matches the specified stair speed multiplier, ±5% |
| Movement around a corner | Agents round a 90° corner without passing through the wall or stalling | Zero wall penetration; no deadlock; smooth trajectory |
| Distribution of walking speeds | A population's sampled speeds reproduce the specified demographic distribution | KS test vs target distribution, p > 0.05 |
| Specific flow through a door | Steady-state flow through a 1 m opening under saturated demand | Within the empirically accepted band (≈1.2–1.4 persons/m·s) |
| Pre-movement / reaction time | Agents begin moving per their assigned reaction-time distribution | Sampled distribution matches spec |
| Movement in a group | Group members remain within the leash distance and match the slowest member | ≥95% of ticks within tolerance |
| Congestion formation | Agents queue and arch realistically at a constriction rather than overlapping | No overlap; arch geometry forms; no explosion |
| Reduced-mobility agents | Impaired agents move at reduced speed and are correctly overtaken | Speed matches spec; no clipping through |
| Counterflow in a corridor | Bidirectional flow produces lane formation | Lanes emerge; throughput degradation within measured range |

These run on **every PR**. R-01 (dense-crowd instability) is caught here or not at all — which is
why these land in phase **B2**, not phase B5.

---

## 3. V2 — Functional verification

Qualitative behaviours checked by assertion rather than eyeball:

- **Exit selection** — with two exits of unequal distance, the split matches the expected
  familiarity-weighted allocation; unfamiliar agents follow signage.
- **Route choice under congestion** — when the primary route saturates, the reroute fraction
  rises and is bounded by the population's `familiarity`.
- **Blocked exit response** — a `close_opening` event mid-evacuation causes affected agents to
  re-target without deadlock, and total egress time increases monotonically.
- **Vertical circulation** — multi-floor evacuation uses stairs with the correct flow-rate clamp;
  merging flows at stair landings queue rather than interpenetrate.
- **Population conservation** — `spawned == in_venue + exited` at every tick. (Also a property
  test; stated here because silently losing agents flatters evacuation times.)

---

## 4. V3 — Quantitative validation against measured human data

This is what makes the model defensible, and it is the part most competing mid-market tools
skip entirely.

### 4.1 Fundamental diagram

The relationship between density and walking speed is the single most-measured property of
pedestrian crowds. Our simulated speed–density curve must fall inside the envelope of published
measurements.

- **Weidmann (1993)** unidirectional speed–density relation — primary reference curve.
- **Fruin (1971)** Level of Service bands A–F — used both for validation and as a visualization
  colormap in A6.
- **Predtechenskii & Milinskii** flow-density data — secondary check.
- SFPE Handbook egress hydraulics — the Nelson/Mowrer correlations.

Harness: a periodic corridor of fixed width, seeded at increasing global densities from 0.5 to
6.0 persons/m²; measure steady-state mean speed and specific flow; plot against the reference
envelope. **Committed as a chart in the V&V report and regenerated on every release.**

If the curve sits outside the envelope, the SFM/PBD parameters are recalibrated — the model is
fit to human data, not to whatever looked good in the demo.

### 4.2 Specific flow through openings

Measured saturated flow through doorways of varying width, compared against:
- Green Guide rates of passage: **82 persons/m/min level, 66 persons/m/min stepped**.
- NFPA 101 egress capacity factors.
- Published bottleneck experiments (Hamburg/Jülich doorway series and similar).

Note the model must reproduce these *emergently* from the physics, and separately the component
throughput clamp must enforce them. Both are tested; a discrepancy between emergent and clamped
flow is itself a reported diagnostic.

### 4.3 Faster-is-slower and arching

At high desired speeds through a constriction, throughput must *decrease* — the well-documented
faster-is-slower effect. A model that doesn't reproduce it will systematically under-predict
evacuation times under panic, which is precisely the dangerous failure mode.

---

## 5. V4 — End-to-end validation against real evacuations

Compare full-building egress curves (cumulative exits vs time) against published, documented
evacuation trials. Candidate sources:

- RiMEA's own hotel evacuation reference case.
- Published university/office building evacuation drill datasets with instrumented timing.
- Stadium/arena egress studies where bowl clearance times are published.
- The Jülich open pedestrian dynamics data archive (experimental trajectory datasets).

Acceptance: simulated total egress time within **±15%** of measured, and the shape of the egress
curve (not just its endpoint) qualitatively matching. Report both; a right answer from a wrong
curve is luck.

**Data sourcing is a real task, not an afterthought** — budget 1 engineer-week in B5 for
acquiring and digitising reference datasets, and record provenance for each.

---

## 6. V5 — Cross-model comparison

Where published results exist for standard geometries (Pathfinder, buildingEXODUS, STEPS,
MassMotion on the same RiMEA or SFPE test cases), run the identical geometry and compare.

Purpose is not to claim superiority — it's to demonstrate we sit inside the spread of
established tools. A result that agrees with four commercial simulators is far easier to defend
to an AHJ than one that's merely internally consistent.

---

## 7. V7 — Sensitivity and uncertainty

A single stochastic run is not evidence. Every reported number in a compliance dossier is
distributional:

- **Monte Carlo**: ≥30 seeds per reported scenario (B6 supports this natively). Report
  mean, p5, p95, and the max observed.
- **Sobol sensitivity indices** over the parameters planners actually guess at — desired speed
  distribution, reaction time, service times, arrival curve shape, familiarity. Output: a ranked
  "what actually drives your egress time" chart in the report appendix.

This has direct product value beyond rigour: it tells a planner *"your egress time is dominated
by checkpoint service time, not exit width — spend money there."* That is the insight the tool
is ultimately selling.

---

## 8. Calibration workflow

Parameters are not hand-tuned by feel. `cf-testkit` includes a calibration harness:

1. Define the objective: weighted error across the fundamental diagram, specific flow, and
   RiMEA component targets.
2. Optimise the SFM/PBD constants (`A`, `B`, `λ` anisotropy, contact stiffness, PBD iterations)
   with CMA-ES or Nelder–Mead over the harness.
3. Freeze the resulting parameter set as `cf-sim/params/default.ron`, versioned.
4. Any change to default parameters requires a re-run of V1–V3 and a diff in the V&V report.

Users may override parameters per-scenario; doing so stamps the report
**"non-default calibration"** with the diff, so a reviewer knows.

---

## 9. What ships to the user

Every exported dossier (`03` A6) carries:

- Engine version, compiler version, parameter-set version, `determinism_hash`.
- Seed(s) and the number of Monte Carlo replicates.
- A link to the V&V report for that engine version.
- The verification statement:

> *This analysis was produced by CrowdFlow Studio v{X}, an agent-based evacuation and crowd-flow
> model verified against the RiMEA test suite and validated against published pedestrian flow
> measurements per ISO 20414. Results are decision support for a competent person and are not a
> substitute for assessment by a qualified fire safety engineer or the approval of the authority
> having jurisdiction. Model assumptions and input parameters are listed in Appendix A.*

That paragraph is not legal boilerplate to be added at the end — it is only truthful if
sections 2–7 of this document have actually been executed. Building the evidence is what earns
the right to print it.
