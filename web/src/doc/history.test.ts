/**
 * Undo across two documents.
 *
 * A venue and a scenario have separate histories but one user, who made their
 * edits in one order and expects them back in that order. `SessionHistory`
 * exists to reconcile that, and it is worth testing because the failure is
 * silent: a wrong-document undo looks like a UI glitch rather than a bug, and
 * the user learns not to trust undo instead of reporting it.
 *
 * This shipped broken once. Scenario edits were being recorded into the
 * session, but `undo` still read the venue history alone, so undoing straight
 * after changing a population reverted a *wall*.
 */

import { describe, expect, it } from 'vitest';
import { History, SessionHistory, type Command } from './commands';

interface Doc {
  n: number;
}

/**
 * Set `n`, carrying what it replaced so the inverse can restore it.
 *
 * Matches the real contract: `apply` returns a new document rather than
 * mutating, and `invert` returns a command rather than performing one.
 */
function setN(from: number, to: number, key?: string): Command<Doc> {
  return {
    kind: 'setN',
    label: `set ${to}`,
    coalesceKey: key,
    apply: (d: Doc) => ({ ...d, n: to }),
    invert: () => setN(to, from, key),
  };
}

describe('SessionHistory', () => {
  it('undoes edits in the order they were made, across both documents', () => {
    const venue = new History<Doc>({ n: 0 });
    const scenario = new History<Doc>({ n: 0 });
    const session = new SessionHistory({ venue, scenario });

    // venue → scenario → venue
    let d = venue.depth;
    venue.run(setN(venue.document.n, 1));
    session.record('venue', d);

    d = scenario.depth;
    scenario.run(setN(scenario.document.n, 10));
    session.record('scenario', d);

    d = venue.depth;
    venue.run(setN(venue.document.n, 2));
    session.record('venue', d);

    expect(session.undo()).toBe('venue');
    expect(venue.document.n).toBe(1);
    expect(scenario.document.n).toBe(10);

    // The next undo must reach the *scenario*, not the venue again.
    expect(session.undo()).toBe('scenario');
    expect(scenario.document.n).toBe(0);
    expect(venue.document.n).toBe(1);

    expect(session.undo()).toBe('venue');
    expect(venue.document.n).toBe(0);

    expect(session.canUndo).toBe(false);
    expect(session.undo()).toBeNull();
  });

  it('redoes in the same interleaved order', () => {
    const venue = new History<Doc>({ n: 0 });
    const scenario = new History<Doc>({ n: 0 });
    const session = new SessionHistory({ venue, scenario });

    let d = venue.depth;
    venue.run(setN(venue.document.n, 1));
    session.record('venue', d);
    d = scenario.depth;
    scenario.run(setN(scenario.document.n, 10));
    session.record('scenario', d);

    session.undo();
    session.undo();
    expect(venue.document.n).toBe(0);
    expect(scenario.document.n).toBe(0);

    expect(session.redo()).toBe('venue');
    expect(venue.document.n).toBe(1);
    expect(session.redo()).toBe('scenario');
    expect(scenario.document.n).toBe(10);
    expect(session.canRedo).toBe(false);
  });

  it('does not record an edit the history coalesced away', () => {
    // Dragging a slider produces a command per frame. They coalesce into one
    // history entry, and the session must not stack an entry per frame or undo
    // would need fifty presses to reverse one drag.
    const venue = new History<Doc>({ n: 0 });
    const scenario = new History<Doc>({ n: 0 });
    const session = new SessionHistory({ venue, scenario });

    for (const v of [1, 2, 3, 4]) {
      const d = scenario.depth;
      scenario.run(setN(scenario.document.n, v, 'slider'));
      session.record('scenario', d);
    }

    expect(scenario.document.n).toBe(4);
    session.undo();
    expect(scenario.document.n).toBe(0);
    expect(session.canUndo).toBe(false);
  });

  it('a new edit clears the redo stack', () => {
    const venue = new History<Doc>({ n: 0 });
    const scenario = new History<Doc>({ n: 0 });
    const session = new SessionHistory({ venue, scenario });

    let d = venue.depth;
    venue.run(setN(venue.document.n, 1));
    session.record('venue', d);
    session.undo();
    expect(session.canRedo).toBe(true);

    d = scenario.depth;
    scenario.run(setN(scenario.document.n, 9));
    session.record('scenario', d);
    expect(session.canRedo).toBe(false);
  });
});
