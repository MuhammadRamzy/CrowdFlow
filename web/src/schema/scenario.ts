/* eslint-disable */
/**
 * GENERATED from schema/scenario.schema.json — do not edit.
 * Regenerate with: pnpm schema
 */

export type OccupancyBasis = "simulated" | "declared";
/**
 * A scheduled change to the world mid-run.
 */
export type TimedEvent = {
  atS: number;
} & TimedEvent1;
export type TimedEvent1 =
  | {
      egressPolicy?: EgressPolicy & string;
      kind: "alarm";
      scope?: AlarmScope & string;
    }
  | {
      kind: "closeOpening";
      target: string;
    }
  | {
      kind: "openOpening";
      target: string;
    }
  | {
      kind: "blockLink";
      target: string;
    }
  | {
      kind: "unblockLink";
      target: string;
    }
  | {
      kind: "closeComponent";
      target: string;
    }
  | {
      kind: "openComponent";
      target: string;
    };
export type EgressPolicy = ("nearestAvailable" | "assignedExit") | "familiarRoute";
export type AlarmScope =
  | "all"
  | {
      floor: {
        id: string;
      };
    }
  | {
      zone: {
        id: string;
      };
    };
export type SimMode = "eventFlow" | "peakLoad" | "evacuation";
/**
 * How agents enter the simulation.
 */
export type Arrival =
  | {
      entries: EntryWeight[];
      kind: "curve";
      points: [number, number][];
    }
  | {
      entries: EntryWeight[];
      kind: "uniform";
    }
  | {
      kind: "preplaced";
      zones: ZoneWeight[];
    };
/**
 * A sampleable distribution.
 *
 * Serialised as an internally-tagged union on the `dist` field, e.g. `{"dist":"normal","mean":1.34,"sd":0.26,"min":0.6,"max":2.2}`.
 */
export type Distribution =
  | {
      dist: "constant";
      value: number;
    }
  | {
      dist: "uniform";
      max: number;
      min: number;
    }
  | {
      dist: "normal";
      max?: number | null;
      mean: number;
      min?: number | null;
      sd: number;
    }
  | {
      dist: "lognormal";
      max?: number | null;
      min?: number | null;
      muLn: number;
      sigmaLn: number;
    }
  | {
      dist: "exponential";
      lambda: number;
      max?: number | null;
    }
  | {
      dist: "categorical";
      p: {
        [k: string]: number;
      };
    }
  | {
      dist: "empirical";
      samples: number[];
    };
/**
 * A destination.
 */
export type Goal =
  | {
      id: string;
      target: "zone";
    }
  | {
      id: string;
      target: "component";
    }
  | {
      id: string;
      target: "waypoint";
    }
  | {
      id: string;
      target: "opening";
    }
  | {
      target: "nearestExit";
    };

export interface ScenarioDoc {
  compliance?: ComplianceConfig | null;
  durationS: number;
  events?: TimedEvent[];
  id: string;
  mode?: SimMode & string;
  name: string;
  output?: OutputConfig;
  populations: Population[];
  schemaVersion: string;
  /**
   * Master PRNG seed. Same seed + same inputs must produce a bit-identical run on every target — see docs/04-track-b-simulation-engine.md §5.
   */
  seed: number;
  timestepS?: number;
  /**
   * The exact venue version this scenario was authored against.
   */
  venueVersion: string;
}
export interface ComplianceConfig {
  /**
   * Rule packs to evaluate, e.g. `["NFPA101", "GreenGuide"]`.
   */
  codes: string[];
  occupancyBasis?: OccupancyBasis & string;
  targetEgressS?: number | null;
}
/**
 * Controls the size and resolution of run artifacts.
 *
 * Defaults are chosen so a 100k-agent 30-minute run produces well under 150 MB rather than the ~1.4 GB that storing raw per-tick positions would cost (docs/02-data-model.md §6).
 */
export interface OutputConfig {
  /**
   * Temporal bucket for density accumulation, in seconds.
   */
  densityBucketS?: number;
  /**
   * Density/dwell grid cell size in metres.
   */
  densityGridM?: number;
  trajectoryHz?: number;
  /**
   * Fraction of agents whose full trajectory is recorded, 0.0–1.0.
   */
  trajectorySampleRate?: number;
}
export interface Population {
  /**
   * Access tags these agents carry, matched against zone and component rules.
   */
  access?: string[];
  arrival: Arrival;
  count: number;
  id: string;
  itinerary?: ItineraryStep[];
  label: string;
  profile: AgentProfile;
}
export interface EntryWeight {
  opening: string;
  weight?: number;
}
export interface ZoneWeight {
  weight?: number;
  zone: string;
}
/**
 * One leg of an agent's plan.
 */
export interface ItineraryStep {
  dwell?: Distribution | null;
  goal: Goal;
  /**
   * Fraction of the population that performs this step. Defaults to all.
   */
  probability?: number;
}
/**
 * Per-agent parameter distributions.
 *
 * Defaults are drawn from the pedestrian dynamics literature (Weidmann's 1.34 m/s mean free walking speed, ~0.23 m shoulder radius) so that an unconfigured population is already defensible rather than arbitrary.
 */
export interface AgentProfile {
  desiredSpeed?: Distribution;
  /**
   * 0.0 = follows signage only, 1.0 = knows every route in the venue.
   */
  familiarity?: number;
  groupSize?: Distribution | null;
  massKg?: Distribution | null;
  mobilityImpairedFrac?: number;
  /**
   * Seconds an agent will queue before considering an alternative.
   */
  patienceS?: Distribution | null;
  radiusM?: Distribution;
  /**
   * Pre-movement delay after an alarm. Evacuation mode only.
   */
  reactionTimeS?: Distribution | null;
}
