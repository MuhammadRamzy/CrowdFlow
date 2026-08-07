/**
 * The scenario panel, rendered.
 *
 * No session that built this frontend has had a browser attached, so this
 * panel has never been clicked. That is the gap these close: not how it looks,
 * which still needs a person, but that it mounts without throwing, shows the
 * numbers it claims to, and that pressing a control emits the command it says
 * it does.
 *
 * Six of this project's bugs were found only by driving the real UI. A render
 * test would have caught the kind that throw; it would not have caught the
 * kind where a handler was silently null. Both matter, and only one of them is
 * testable here.
 */

import { cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ScenarioDoc } from '../schema/scenario';
import type { VenueDoc } from '../schema/venue';
import { defaultScenario } from '../doc/scenario';
import { useApp } from '../state/store';
import { ScenarioPanel } from './ScenarioPanel';

function venue(): VenueDoc {
  return {
    schemaVersion: 'cfs.venue/1.0',
    id: 'ven_t',
    name: 'Test hall',
    units: 'm',
    floors: [
      {
        id: 'f0',
        name: 'Ground',
        level: 0,
        elevationM: 0,
        walls: [],
        openings: [
          { id: 'op_w', wall: 'w0', t: 0.25, widthM: 1.2, kind: 'door', isFireExit: true },
          { id: 'op_e', wall: 'w0', t: 0.75, widthM: 2.0, kind: 'doubleDoor' },
        ],
        zones: [
          {
            id: 'z_hall',
            name: 'Main hall',
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

function setup(doc?: ScenarioDoc) {
  const onEdit = vi.fn();
  const scenario = doc ?? defaultScenario(venue(), 500);
  render(<ScenarioPanel scenario={scenario} venue={venue()} onEdit={onEdit} />);
  return { onEdit, scenario };
}

beforeEach(() => {
  // 240 m² at 0.65 m²/person → an occupant load of 369.
  useApp.setState({ walkableArea: 240 });
});

// Vitest is configured without globals, so testing-library's automatic
// teardown does not run. Without this the DOM accumulates across tests and
// every query finds several matches.
afterEach(cleanup);

describe('ScenarioPanel', () => {
  it('renders without a scenario rather than throwing', () => {
    render(<ScenarioPanel scenario={null} venue={null} onEdit={vi.fn()} />);
    expect(screen.getByText(/load a venue/i)).toBeTruthy();
  });

  it('shows the agent count against the occupant load', () => {
    setup();
    // Scoped to their own rows: the population's own count also reads 500, and
    // a bare text query would match either.
    const total = screen.getByText('Total agents').closest('.row')!;
    expect(within(total as HTMLElement).getByText('500')).toBeTruthy();

    const load = screen.getByText('Occupant load').closest('.row')!;
    expect(within(load as HTMLElement).getByText('369')).toBeTruthy();

    // 500 people in a hall licensed for 369 is an alarm, not a note.
    expect((total as HTMLElement).querySelector('.is-alarm')).toBeTruthy();
  });

  it('says how far over the limit a crowd is, in the dossier’s own arithmetic', () => {
    const doc = defaultScenario(venue(), 500);
    doc.populations[0]!.count = 1000;
    setup(doc);
    // 1000 − 369 = 631. A reviewer should not have to do that subtraction.
    expect(screen.getByText(/631 over the NFPA 101 load/i)).toBeTruthy();
  });

  it('does not claim an overcrowding it has not measured', () => {
    // A venue whose area failed to compile has a limit of 0, and every crowd
    // would otherwise read as over it.
    useApp.setState({ walkableArea: 0 });
    setup();
    expect(screen.queryByText(/over the NFPA 101 load/i)).toBeNull();
  });

  it('emits a command when the seed is changed', async () => {
    const user = userEvent.setup();
    const { onEdit } = setup();
    const seed = screen.getByLabelText(/seed/i);
    await user.clear(seed);
    await user.type(seed, '7');
    expect(onEdit).toHaveBeenCalled();
  });

  it('opens a population and edits its count', async () => {
    const user = userEvent.setup();
    const { onEdit } = setup();

    // Collapsed to start: the fields are not in the document yet.
    expect(screen.queryByLabelText(/^count$/i)).toBeNull();

    await user.click(screen.getByRole('button', { name: /general admission/i }));
    const count = screen.getByLabelText(/^count$/i);
    await user.clear(count);
    await user.type(count, '250');
    expect(onEdit).toHaveBeenCalled();
  });

  it('offers only the goals the engine can act on', async () => {
    const user = userEvent.setup();
    setup();
    await user.click(screen.getByRole('button', { name: /general admission/i }));

    const goal = screen.getByLabelText(/^goal$/i) as HTMLSelectElement;
    const options = Array.from(goal.options).map((o) => o.textContent);
    expect(options).toContain('Nearest exit');
    expect(options).toContain('Dwell in Main hall');
  });

  it('names doors by size rather than by opaque id', async () => {
    const user = userEvent.setup();
    const doc = defaultScenario(venue(), 100);
    doc.populations[0]!.arrival = { kind: 'uniform', entries: [{ opening: 'op_e', weight: 1 }] };
    setup(doc);
    await user.click(screen.getByRole('button', { name: /general admission/i }));

    const entry = screen.getByLabelText(/^entry$/i);
    expect(within(entry).getByText(/2\.00 m/)).toBeTruthy();
  });
});

describe('events', () => {
  it('says what an empty timeline means instead of showing nothing', () => {
    setup();
    expect(screen.getByText(/nothing happens partway through/i)).toBeTruthy();
  });

  it('adds an alarm', async () => {
    const user = userEvent.setup();
    const { onEdit } = setup();
    await user.click(screen.getByRole('button', { name: /add alarm/i }));
    expect(onEdit).toHaveBeenCalledTimes(1);
  });

  it('lists an existing event with its time and effect', () => {
    const doc = defaultScenario(venue(), 100);
    doc.events = [
      { atS: 20, kind: 'alarm' },
      { atS: 45, kind: 'closeOpening', target: 'op_e' },
    ];
    setup(doc);

    expect(screen.getByText(/alarm — everyone leaves/i)).toBeTruthy();
    expect(screen.getByText(/shut 2\.00 m/i)).toBeTruthy();
    // Ordered by time down the page, so the list reads as a timeline.
    const times = screen.getAllByLabelText(/event time/i) as HTMLInputElement[];
    expect(times.map((t) => t.value)).toEqual(['20', '45']);
  });

  it('cannot shut a door in a venue with none', () => {
    const bare = { ...venue() };
    bare.floors = [{ ...bare.floors[0]!, openings: [] }];
    render(
      <ScenarioPanel scenario={defaultScenario(bare, 100)} venue={bare} onEdit={vi.fn()} />,
    );
    const shut = screen.getByRole('button', { name: /shut a door/i }) as HTMLButtonElement;
    expect(shut.disabled).toBe(true);
  });
});
