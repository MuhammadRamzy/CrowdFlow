# GENERATED from schema/scenario.schema.json — do not edit.
# Regenerate with: make models   (from services/)
# The source of truth is engine/cf-schema (ADR 0001).

from __future__ import annotations

from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field, RootModel


class Floor(BaseModel):
    id: str


class AlarmScope1(BaseModel):
    model_config = ConfigDict(
        extra="forbid",
    )
    floor: Floor


class Zone(BaseModel):
    id: str


class AlarmScope2(BaseModel):
    model_config = ConfigDict(
        extra="forbid",
    )
    zone: Zone


class Point(RootModel[list[float]]):
    root: Annotated[list[float], Field(max_length=2, min_length=2)]


class Distribution1(BaseModel):
    """
    A fixed value. Useful for pinning a parameter during sensitivity analysis.
    """

    dist: Literal["constant"]
    value: float


class Distribution2(BaseModel):
    """
    A sampleable distribution.

    Serialised as an internally-tagged union on the `dist` field, e.g. `{"dist":"normal","mean":1.34,"sd":0.26,"min":0.6,"max":2.2}`.
    """

    dist: Literal["uniform"]
    max: float
    min: float


class Distribution3(BaseModel):
    """
    A sampleable distribution.

    Serialised as an internally-tagged union on the `dist` field, e.g. `{"dist":"normal","mean":1.34,"sd":0.26,"min":0.6,"max":2.2}`.
    """

    dist: Literal["normal"]
    max: float | None = None
    mean: float
    min: float | None = None
    sd: float


class Distribution4(BaseModel):
    """
    Parameterised by the mean and sd **of the underlying normal**, not of the resulting variate. Named `muLn`/`sigmaLn` rather than `mu`/`sigma` so the distinction is impossible to miss at a call site — getting this wrong is the single most common modelling error with lognormals.

    Note: `rename_all` on the enum renames *variants*, not their fields, so multi-word fields need their own attribute.
    """

    dist: Literal["lognormal"]
    max: float | None = None
    min: float | None = None
    muLn: float
    sigmaLn: float


class Distribution5(BaseModel):
    """
    A sampleable distribution.

    Serialised as an internally-tagged union on the `dist` field, e.g. `{"dist":"normal","mean":1.34,"sd":0.26,"min":0.6,"max":2.2}`.
    """

    dist: Literal["exponential"]
    lambda_: Annotated[float, Field(alias="lambda")]
    max: float | None = None


class Distribution6(BaseModel):
    """
    Discrete outcomes with weights. Weights need not sum to 1; they are normalised. Keys are strings so that group sizes (`"1"`, `"2"`, …) and named categories share one representation.
    """

    dist: Literal["categorical"]
    p: dict[str, float]


class Distribution7(BaseModel):
    """
    Empirical distribution over observed samples, sampled by linear interpolation of the order statistics.
    """

    dist: Literal["empirical"]
    samples: list[float]


class EntryWeight(BaseModel):
    opening: str
    weight: float | None = 1.0


class Goal1(BaseModel):
    """
    A destination.
    """

    id: str
    target: Literal["zone"]


class Goal2(BaseModel):
    """
    A destination.
    """

    id: str
    target: Literal["component"]


class Goal3(BaseModel):
    """
    A destination.
    """

    id: str
    target: Literal["waypoint"]


class Goal4(BaseModel):
    """
    A destination.
    """

    id: str
    target: Literal["opening"]


class Goal5(BaseModel):
    """
    Nearest exit by traversable distance, recomputed if blocked.
    """

    target: Literal["nearestExit"]


class ItineraryStep(BaseModel):
    """
    One leg of an agent's plan.
    """

    dwell: (
        Distribution1
        | Distribution2
        | Distribution3
        | Distribution4
        | Distribution5
        | Distribution6
        | Distribution7
        | None
    ) = None
    goal: Goal1 | Goal2 | Goal3 | Goal4 | Goal5
    """
    A destination.
    """
    probability: float | None = 1.0
    """
    Fraction of the population that performs this step. Defaults to all.
    """


class OutputConfig(BaseModel):
    """
    Controls the size and resolution of run artifacts.

    Defaults are chosen so a 100k-agent 30-minute run produces well under 150 MB rather than the ~1.4 GB that storing raw per-tick positions would cost (docs/02-data-model.md §6).
    """

    densityBucketS: float | None = 5.0
    """
    Temporal bucket for density accumulation, in seconds.
    """
    densityGridM: float | None = 0.5
    """
    Density/dwell grid cell size in metres.
    """
    trajectoryHz: float | None = 2.0
    trajectorySampleRate: float | None = 0.02
    """
    Fraction of agents whose full trajectory is recorded, 0.0–1.0.
    """


class TimedEvent1(BaseModel):
    """
    Trigger evacuation behaviour.
    """

    atS: float
    egressPolicy: (
        Literal["nearestAvailable", "assignedExit"] | Literal["familiarRoute"] | None
    ) = "nearestAvailable"
    kind: Literal["alarm"]
    scope: Annotated[
        Literal["all"] | AlarmScope1 | AlarmScope2 | None, Field(validate_default=True)
    ] = "all"


class TimedEvent2(BaseModel):
    """
    A scheduled change to the world mid-run.
    """

    atS: float
    kind: Literal["closeOpening"]
    target: str


class TimedEvent3(BaseModel):
    """
    A scheduled change to the world mid-run.
    """

    atS: float
    kind: Literal["openOpening"]
    target: str


class TimedEvent4(BaseModel):
    """
    A scheduled change to the world mid-run.
    """

    atS: float
    kind: Literal["blockLink"]
    target: str


class TimedEvent5(BaseModel):
    """
    A scheduled change to the world mid-run.
    """

    atS: float
    kind: Literal["unblockLink"]
    target: str


class TimedEvent6(BaseModel):
    """
    A scheduled change to the world mid-run.
    """

    atS: float
    kind: Literal["closeComponent"]
    target: str


class TimedEvent7(BaseModel):
    """
    A scheduled change to the world mid-run.
    """

    atS: float
    kind: Literal["openComponent"]
    target: str


class ZoneWeight(BaseModel):
    weight: float | None = 1.0
    zone: str


class AgentProfile(BaseModel):
    """
    Per-agent parameter distributions.

    Defaults are drawn from the pedestrian dynamics literature (Weidmann's 1.34 m/s mean free walking speed, ~0.23 m shoulder radius) so that an unconfigured population is already defensible rather than arbitrary.
    """

    desiredSpeed: Annotated[
        Distribution1
        | Distribution2
        | Distribution3
        | Distribution4
        | Distribution5
        | Distribution6
        | Distribution7
        | None,
        Field(validate_default=True),
    ] = {"dist": "normal", "mean": 1.34, "sd": 0.26, "min": 0.6, "max": 2.2}
    """
    A sampleable distribution.

    Serialised as an internally-tagged union on the `dist` field, e.g. `{"dist":"normal","mean":1.34,"sd":0.26,"min":0.6,"max":2.2}`.
    """
    familiarity: float | None = 0.6
    """
    0.0 = follows signage only, 1.0 = knows every route in the venue.
    """
    groupSize: (
        Distribution1
        | Distribution2
        | Distribution3
        | Distribution4
        | Distribution5
        | Distribution6
        | Distribution7
        | None
    ) = None
    massKg: (
        Distribution1
        | Distribution2
        | Distribution3
        | Distribution4
        | Distribution5
        | Distribution6
        | Distribution7
        | None
    ) = None
    mobilityImpairedFrac: float | None = 0.0
    patienceS: (
        Distribution1
        | Distribution2
        | Distribution3
        | Distribution4
        | Distribution5
        | Distribution6
        | Distribution7
        | None
    ) = None
    """
    Seconds an agent will queue before considering an alternative.
    """
    radiusM: Annotated[
        Distribution1
        | Distribution2
        | Distribution3
        | Distribution4
        | Distribution5
        | Distribution6
        | Distribution7
        | None,
        Field(validate_default=True),
    ] = {"dist": "normal", "mean": 0.23, "sd": 0.02, "min": 0.18, "max": 0.3}
    """
    A sampleable distribution.

    Serialised as an internally-tagged union on the `dist` field, e.g. `{"dist":"normal","mean":1.34,"sd":0.26,"min":0.6,"max":2.2}`.
    """
    reactionTimeS: (
        Distribution1
        | Distribution2
        | Distribution3
        | Distribution4
        | Distribution5
        | Distribution6
        | Distribution7
        | None
    ) = None
    """
    Pre-movement delay after an alarm. Evacuation mode only.
    """


class Arrival1(BaseModel):
    """
    Cumulative arrival curve: `points` are `(t_seconds, cumulative_fraction)`, monotonically non-decreasing in both, ending at fraction 1.0.
    """

    entries: list[EntryWeight]
    kind: Literal["curve"]
    points: list[Point]


class Arrival2(BaseModel):
    """
    Constant rate over the whole duration.
    """

    entries: list[EntryWeight]
    kind: Literal["uniform"]


class Arrival3(BaseModel):
    """
    Everyone present at t=0, distributed across zones. The starting condition for an evacuation study.
    """

    kind: Literal["preplaced"]
    zones: list[ZoneWeight]


class ComplianceConfig(BaseModel):
    codes: list[str]
    """
    Rule packs to evaluate, e.g. `["NFPA101", "GreenGuide"]`.
    """
    occupancyBasis: Literal["simulated"] | Literal["declared"] | None = "simulated"
    targetEgressS: float | None = None


class Population(BaseModel):
    access: list[str] | None = None
    """
    Access tags these agents carry, matched against zone and component rules.
    """
    arrival: Arrival1 | Arrival2 | Arrival3
    """
    How agents enter the simulation.
    """
    count: Annotated[int, Field(ge=0)]
    id: str
    itinerary: list[ItineraryStep] | None = None
    label: str
    profile: AgentProfile


class ScenarioDoc(BaseModel):
    compliance: ComplianceConfig | None = None
    durationS: float
    events: (
        list[
            TimedEvent1
            | TimedEvent2
            | TimedEvent3
            | TimedEvent4
            | TimedEvent5
            | TimedEvent6
            | TimedEvent7
        ]
        | None
    ) = None
    id: str
    mode: Literal["eventFlow"] | Literal["peakLoad"] | Literal["evacuation"] | None = (
        "eventFlow"
    )
    name: str
    output: Annotated[OutputConfig | None, Field(validate_default=True)] = {
        "densityGridM": 0.5,
        "densityBucketS": 5.0,
        "trajectorySampleRate": 0.02,
        "trajectoryHz": 2.0,
    }
    populations: list[Population]
    schemaVersion: str
    seed: Annotated[int, Field(ge=0)]
    """
    Master PRNG seed. Same seed + same inputs must produce a bit-identical run on every target — see docs/04-track-b-simulation-engine.md §5.
    """
    timestepS: float | None = 0.05
    venueVersion: str
    """
    The exact venue version this scenario was authored against.
    """
