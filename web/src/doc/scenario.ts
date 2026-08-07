/**
 * Scenario authoring: who the crowd is, where they come from, when they arrive.
 *
 * A scenario is a document in its own right (`docs/02-data-model.md` §3) with
 * its own lifecycle — one venue supports many scenarios without duplicating
 * geometry. So it gets its own history and its own commands, and the same rule
 * applies: **never mutate it directly.**
 *
 * # Why these commands hold snapshots when venue commands do not
 *
 * `commands.ts` argues against snapshots, and it is right — about venues. A
 * venue carries tens of thousands of vertices, so a snapshot per edit is
 * expensive to keep and slow to diff. A scenario is a handful of populations:
 * a few hundred bytes. Storing before-and-after outright is smaller than the
 * machinery needed to avoid it, and it cannot get an `invert` wrong. Different
 * document, different trade, stated rather than assumed.
 */

import type {
  AgentProfile,
  TimedEvent,
  Arrival,
  Distribution,
  Goal,
  Population,
  ScenarioDoc,
} from '../schema/scenario';
import type { VenueDoc } from '../schema/venue';
import { newId, type Command } from './commands';

export const SCENARIO_SCHEMA_VERSION = 'cfs.scenario/1.0';

/** Weidmann (1993): mean free walking speed 1.34 m/s, sd 0.26. */
export const DEFAULT_SPEED: Distribution = {
  dist: 'normal',
  mean: 1.34,
  sd: 0.26,
  min: 0.6,
  max: 2.2,
};

/** Shoulder half-width. 0.23 m is the figure the engine's defaults use. */
export const DEFAULT_RADIUS: Distribution = {
  dist: 'normal',
  mean: 0.23,
  sd: 0.02,
  min: 0.18,
  max: 0.3,
};

const DEFAULT_PROFILE: AgentProfile = {
  desiredSpeed: DEFAULT_SPEED,
  radiusM: DEFAULT_RADIUS,
  familiarity: 0.6,
  mobilityImpairedFrac: 0,
};

/**
 * A scenario that reproduces the venue as loaded.
 *
 * One population, everyone already inside, everyone leaving by the nearest
 * exit — the evacuation case, and the same thing the workspace did before
 * scenarios existed. Starting from a defensible default rather than an empty
 * document means the Run button works the moment a venue opens.
 */
export function defaultScenario(venue: VenueDoc, count = 500): ScenarioDoc {
  return {
    schemaVersion: SCENARIO_SCHEMA_VERSION,
    id: newId('scn'),
    name: 'Full evacuation',
    venueVersion: venue.id,
    mode: 'evacuation',
    durationS: 600,
    timestepS: 0.05,
    seed: 20260801,
    populations: [newPopulation(venue, 'General admission', count)],
    events: [],
    output: {
      densityGridM: 0.5,
      densityBucketS: 5,
      trajectorySampleRate: 0.02,
      trajectoryHz: 2,
    },
  };
}

/**
 * A population placed wherever the venue can actually hold one.
 *
 * Preplaced in the largest zone if there is one, because that is the reading a
 * planner expects from "put 500 people in my hall". With no zones there is
 * nothing to name, and the engine scatters them over the floor and says so.
 */
export function newPopulation(venue: VenueDoc, label: string, count: number): Population {
  const zone = largestZone(venue);
  const arrival: Arrival = zone
    ? { kind: 'preplaced', zones: [{ zone, weight: 1 }] }
    : { kind: 'preplaced', zones: [] };
  return {
    id: newId('pop'),
    label,
    count,
    profile: structuredClone(DEFAULT_PROFILE),
    arrival,
    itinerary: [{ goal: { target: 'nearestExit' }, probability: 1 }],
    access: [],
  };
}

/** The zone with the largest polygon area, by id. */
function largestZone(venue: VenueDoc): string | null {
  let best: { id: string; area: number } | null = null;
  for (const floor of venue.floors) {
    for (const z of floor.zones ?? []) {
      if (z.isVoid) continue;
      const area = polygonArea(z.polygon);
      if (!best || area > best.area) best = { id: z.id, area };
    }
  }
  return best?.id ?? null;
}

/** Shoelace area of a closed ring, m². */
export function polygonArea(poly: number[][]): number {
  let acc = 0;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    acc += poly[j]![0]! * poly[i]![1]! - poly[i]![0]! * poly[j]![1]!;
  }
  return Math.abs(acc) / 2;
}

/** Everyone the scenario asks for, across all populations. */
export function totalAgents(doc: ScenarioDoc): number {
  return doc.populations.reduce((n, p) => n + p.count, 0);
}

/** The population's destination, or the implied default when none is set. */
export function goalOf(pop: Population): Goal {
  return pop.itinerary?.[0]?.goal ?? { target: 'nearestExit' };
}

// ---------------------------------------------------------------------------
// Arrival curves
// ---------------------------------------------------------------------------

/**
 * A gentle default curve: a slow start, a rush, then a tail.
 *
 * Points are `(t seconds, cumulative fraction)`. Chosen to look like a real
 * doors-open profile rather than a straight line, so the first thing a user
 * sees in the plot is a shape they can recognise and drag.
 */
export function defaultCurve(durationS: number): [number, number][] {
  const d = Math.max(1, durationS);
  return [
    [0, 0],
    [d * 0.25, 0.15],
    [d * 0.55, 0.7],
    [d, 1],
  ];
}

/**
 * Put a curve back into a shape the engine can invert.
 *
 * The schema requires points non-decreasing in *both* axes and ending at 1.0.
 * A drag can violate either, so this is applied on every edit rather than
 * trusted to the pointer maths: an invalid curve does not throw, it silently
 * produces a crowd that arrives at the wrong time.
 */
export function normaliseCurve(points: [number, number][], durationS: number): [number, number][] {
  if (points.length === 0) return defaultCurve(durationS);
  const d = Math.max(1, durationS);
  const sorted = points
    .map(([t, f]): [number, number] => [clamp(t, 0, d), clamp(f, 0, 1)])
    .sort((a, b) => a[0] - b[0]);

  const out: [number, number][] = [];
  let highest = 0;
  for (const [t, f] of sorted) {
    highest = Math.max(highest, f);
    out.push([t, highest]);
  }
  // Anchor both ends. Without the last point at 1.0 a fraction of the
  // population has no arrival time at all and would never enter.
  if (out[0]![0] > 0) out.unshift([0, 0]);
  const last = out[out.length - 1]!;
  if (last[1] < 1) {
    if (last[0] >= d) last[1] = 1;
    else out.push([d, 1]);
  }
  return out;
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/**
 * A whole-document scenario edit.
 *
 * `apply` ignores the document handed to it and returns its own `after`
 * snapshot. That is sound because history is linear — a command is only ever
 * applied to the state it was built against, or to the state its inverse just
 * produced.
 */
class EditScenario implements Command<ScenarioDoc> {
  constructor(
    readonly kind: string,
    readonly label: string,
    private readonly before: ScenarioDoc,
    private readonly after: ScenarioDoc,
    readonly coalesceKey?: string,
  ) {}

  apply(): ScenarioDoc {
    return structuredClone(this.after);
  }

  invert(): Command<ScenarioDoc> {
    return new EditScenario(this.kind, this.label, this.after, this.before, this.coalesceKey);
  }
}

/** Build a command from a mutation applied to a copy of the document. */
export function editScenario(
  doc: ScenarioDoc,
  kind: string,
  label: string,
  mutate: (draft: ScenarioDoc) => void,
  coalesceKey?: string,
): Command<ScenarioDoc> {
  const after = structuredClone(doc);
  mutate(after);
  return new EditScenario(kind, label, structuredClone(doc), after, coalesceKey);
}

function withPopulation(
  doc: ScenarioDoc,
  popId: string,
  kind: string,
  label: string,
  mutate: (p: Population) => void,
  coalesceKey?: string,
): Command<ScenarioDoc> {
  return editScenario(
    doc,
    kind,
    label,
    (draft) => {
      const p = draft.populations.find((x) => x.id === popId);
      if (p) mutate(p);
    },
    coalesceKey,
  );
}

export function addPopulation(doc: ScenarioDoc, pop: Population): Command<ScenarioDoc> {
  return editScenario(doc, 'scenario.population.add', 'Add population', (d) => {
    d.populations.push(pop);
  });
}

export function removePopulation(doc: ScenarioDoc, popId: string): Command<ScenarioDoc> {
  return editScenario(doc, 'scenario.population.remove', 'Delete population', (d) => {
    d.populations = d.populations.filter((p) => p.id !== popId);
  });
}

export function setPopulationLabel(
  doc: ScenarioDoc,
  popId: string,
  label: string,
): Command<ScenarioDoc> {
  return withPopulation(
    doc,
    popId,
    'scenario.population.label',
    'Rename population',
    (p) => {
      p.label = label;
    },
    `pop:${popId}:label`,
  );
}

export function setPopulationCount(
  doc: ScenarioDoc,
  popId: string,
  count: number,
): Command<ScenarioDoc> {
  return withPopulation(
    doc,
    popId,
    'scenario.population.count',
    'Change population size',
    (p) => {
      p.count = Math.max(0, Math.round(count));
    },
    `pop:${popId}:count`,
  );
}

/** Replace one of the two per-agent distributions the engine actually reads. */
export function setProfileDistribution(
  doc: ScenarioDoc,
  popId: string,
  field: 'desiredSpeed' | 'radiusM',
  dist: Distribution,
): Command<ScenarioDoc> {
  const what = field === 'desiredSpeed' ? 'walking speed' : 'body radius';
  return withPopulation(
    doc,
    popId,
    `scenario.population.${field}`,
    `Change ${what}`,
    (p) => {
      p.profile = { ...p.profile, [field]: dist };
    },
    `pop:${popId}:${field}`,
  );
}

export function setArrival(
  doc: ScenarioDoc,
  popId: string,
  arrival: Arrival,
): Command<ScenarioDoc> {
  return withPopulation(
    doc,
    popId,
    'scenario.population.arrival',
    'Change arrival',
    (p) => {
      p.arrival = arrival;
    },
    // Dragging a curve point emits an edit per pointermove; one undo should
    // reverse the whole drag.
    `pop:${popId}:arrival`,
  );
}

export function setGoal(doc: ScenarioDoc, popId: string, goal: Goal): Command<ScenarioDoc> {
  return withPopulation(doc, popId, 'scenario.population.goal', 'Change goal', (p) => {
    const rest = (p.itinerary ?? []).slice(1);
    p.itinerary = [{ goal, probability: 1 }, ...rest];
  });
}

export function setScenarioName(doc: ScenarioDoc, name: string): Command<ScenarioDoc> {
  return editScenario(
    doc,
    'scenario.name',
    'Rename scenario',
    (d) => {
      d.name = name;
    },
    'scenario:name',
  );
}

export function setScenarioSeed(doc: ScenarioDoc, seed: number): Command<ScenarioDoc> {
  return editScenario(
    doc,
    'scenario.seed',
    'Change seed',
    (d) => {
      d.seed = Math.max(0, Math.round(seed));
    },
    'scenario:seed',
  );
}

/**
 * Change the run length.
 *
 * Curves are rescaled with it: their points are times in seconds, so halving
 * the duration without touching them would leave half the crowd arriving after
 * the run has ended.
 */
export function setScenarioDuration(doc: ScenarioDoc, durationS: number): Command<ScenarioDoc> {
  const next = Math.max(1, Math.round(durationS));
  const previous = Math.max(1, doc.durationS);
  const scale = next / previous;
  return editScenario(
    doc,
    'scenario.duration',
    'Change duration',
    (d) => {
      d.durationS = next;
      for (const p of d.populations) {
        if (p.arrival.kind !== 'curve') continue;
        p.arrival.points = normaliseCurve(
          p.arrival.points.map(([t, f]): [number, number] => [t * scale, f]),
          next,
        );
      }
    },
    'scenario:duration',
  );
}

// ---------------------------------------------------------------------------
// Timed events
//
// The engine acts on `closeOpening` and `alarm`; the rest round-trip and are
// reported under "Not simulated". Only the two that do something are offered
// here — a control that edits a field nothing reads is worse than no control,
// because a reviewer will believe it.
// ---------------------------------------------------------------------------

/** Events in the order they fire. Ties keep document order, as the engine does. */
export function sortedEvents(doc: ScenarioDoc): TimedEvent[] {
  return [...(doc.events ?? [])].sort((a, b) => a.atS - b.atS);
}

/** Sound the alarm at `atS`: everyone heads for the nearest exit. */
export function addAlarm(doc: ScenarioDoc, atS: number): Command<ScenarioDoc> {
  return editScenario(doc, 'scenario.events', 'Add alarm', (d) => {
    d.events = [...(d.events ?? []), { atS: clampTime(atS, d), kind: 'alarm' }];
  });
}

/** Shut a doorway at `atS`. The engine seals it, not merely un-lists it. */
export function addClosure(
  doc: ScenarioDoc,
  atS: number,
  opening: string,
): Command<ScenarioDoc> {
  return editScenario(doc, 'scenario.events', 'Close a doorway', (d) => {
    d.events = [
      ...(d.events ?? []),
      { atS: clampTime(atS, d), kind: 'closeOpening', target: opening },
    ];
  });
}

export function removeEvent(doc: ScenarioDoc, index: number): Command<ScenarioDoc> {
  return editScenario(doc, 'scenario.events', 'Remove event', (d) => {
    d.events = (d.events ?? []).filter((_, i) => i !== index);
  });
}

export function setEventTime(
  doc: ScenarioDoc,
  index: number,
  atS: number,
): Command<ScenarioDoc> {
  return editScenario(
    doc,
    'scenario.events',
    'Move event',
    (d) => {
      const e = (d.events ?? [])[index];
      if (e) e.atS = clampTime(atS, d);
    },
    `scenario:event:${index}:time`,
  );
}

/**
 * An event after the run ends never fires, and nothing on screen would say so.
 * Clamping is the honest reading of "put it at the end".
 */
function clampTime(atS: number, doc: ScenarioDoc): number {
  if (!Number.isFinite(atS)) return 0;
  return Math.min(Math.max(0, Math.round(atS)), doc.durationS);
}
