/**
 * Scenario editing.
 *
 * These commands are the only way the panel changes a scenario, and what they
 * emit goes straight to the engine as JSON. So two things are worth pinning:
 * that every edit is reversible, and that the **wire shape** is the one
 * `cf-wasm` deserialises. This repo has twice shipped a field whose serialised
 * name did not match its declared one, and both times every other check passed.
 */

import { describe, expect, it } from 'vitest';
import type { ScenarioDoc } from '../schema/scenario';
import type { VenueDoc } from '../schema/venue';
import { History } from './commands';
import {
  addAlarm,
  addClosure,
  addPopulation,
  defaultScenario,
  newPopulation,
  removeEvent,
  removePopulation,
  setEventTime,
  setPopulationCount,
  setScenarioSeed,
  sortedEvents,
  totalAgents,
} from './scenario';

/** A minimal venue: one floor, one zone, one door. */
function venue(): VenueDoc {
  return {
    schemaVersion: 'cfs.venue/1.0',
    id: 'ven_t',
    name: 'Test',
    units: 'm',
    floors: [
      {
        id: 'f0',
        name: 'Ground',
        level: 0,
        elevationM: 0,
        walls: [],
        openings: [{ id: 'op_a', wall: 'w0', t: 0.5, widthM: 1.2 }],
        zones: [
          {
            id: 'z_hall',
            polygon: [
              [0, 0],
              [20, 0],
              [20, 12],
              [0, 12],
            ],
            kind: 'assemblyConcentrated',
          },
        ],
      },
    ],
    routing: { waypoints: [], edges: [] },
  } as unknown as VenueDoc;
}

function fresh(): { doc: ScenarioDoc; h: History<ScenarioDoc> } {
  const doc = defaultScenario(venue(), 500);
  return { doc, h: new History<ScenarioDoc>(doc) };
}

describe('defaultScenario', () => {
  it('is runnable the moment a venue opens', () => {
    const { doc } = fresh();
    expect(doc.populations).toHaveLength(1);
    expect(totalAgents(doc)).toBe(500);
    // Everyone already inside, everyone leaving: the evacuation case.
    expect(doc.populations[0]!.arrival.kind).toBe('preplaced');
    expect(doc.populations[0]!.itinerary![0]!.goal).toEqual({ target: 'nearestExit' });
  });
});

describe('scenario commands', () => {
  it('every edit is reversible', () => {
    const { h } = fresh();
    const before = structuredClone(h.document);

    h.run(setPopulationCount(h.document, h.document.populations[0]!.id, 42));
    expect(totalAgents(h.document)).toBe(42);

    h.run(addPopulation(h.document, newPopulation(venue(), 'Staff', 20)));
    expect(h.document.populations).toHaveLength(2);
    expect(totalAgents(h.document)).toBe(62);

    h.run(setScenarioSeed(h.document, 99));
    expect(h.document.seed).toBe(99);

    while (h.canUndo) h.undo();
    expect(h.document).toEqual(before);
  });

  it('removing a population removes only that one', () => {
    const { h } = fresh();
    h.run(addPopulation(h.document, newPopulation(venue(), 'Staff', 20)));
    const keep = h.document.populations[0]!.id;
    const drop = h.document.populations[1]!.id;

    h.run(removePopulation(h.document, drop));
    expect(h.document.populations.map((p) => p.id)).toEqual([keep]);

    h.undo();
    expect(h.document.populations).toHaveLength(2);
  });

  it('never mutates the document it was given', () => {
    // The panel renders from the same object it passes in. An in-place edit
    // would change what React is holding without telling it, and the screen
    // would silently disagree with the run.
    const { doc, h } = fresh();
    const snapshot = structuredClone(doc);
    h.run(setPopulationCount(doc, doc.populations[0]!.id, 7));
    expect(doc).toEqual(snapshot);
  });
});

describe('timed events', () => {
  it('emits the wire shape cf-wasm deserialises', () => {
    const { h } = fresh();
    h.run(addAlarm(h.document, 20));
    h.run(addClosure(h.document, 45, 'op_a'));

    // These two literals are what `engine/cf-wasm/tests/end_to_end.rs` asserts
    // the engine acts on. If either side drifts, one of the two fails.
    expect(h.document.events).toEqual([
      { atS: 20, kind: 'alarm' },
      { atS: 45, kind: 'closeOpening', target: 'op_a' },
    ]);
  });

  it('orders by time without disturbing the document', () => {
    const { h } = fresh();
    h.run(addClosure(h.document, 90, 'op_a'));
    h.run(addAlarm(h.document, 10));

    expect(sortedEvents(h.document).map((e) => e.atS)).toEqual([10, 90]);
    // Document order is untouched: the commands address events by index.
    expect(h.document.events!.map((e) => e.atS)).toEqual([90, 10]);
  });

  it('clamps a time outside the run rather than accepting one that never fires', () => {
    const { h } = fresh();
    h.run(addAlarm(h.document, 20));
    const i = 0;

    h.run(setEventTime(h.document, i, -5));
    expect(h.document.events![i]!.atS).toBe(0);

    h.run(setEventTime(h.document, i, h.document.durationS + 1000));
    expect(h.document.events![i]!.atS).toBe(h.document.durationS);

    // NaN from an emptied number input must not corrupt the document.
    h.run(setEventTime(h.document, i, Number.NaN));
    expect(h.document.events![i]!.atS).toBe(0);
  });

  it('removing an event is reversible', () => {
    const { h } = fresh();
    h.run(addAlarm(h.document, 20));
    h.run(addClosure(h.document, 45, 'op_a'));

    h.run(removeEvent(h.document, 0));
    expect(h.document.events).toEqual([{ atS: 45, kind: 'closeOpening', target: 'op_a' }]);

    h.undo();
    expect(h.document.events).toHaveLength(2);
  });
});
