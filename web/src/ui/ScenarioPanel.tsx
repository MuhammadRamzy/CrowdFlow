/**
 * Scenario authoring: who the crowd is, where it comes from, when it arrives.
 *
 * The venue answers "what is the building". This answers "what happens in it",
 * and it is the half a planner actually varies — the geometry is fixed by the
 * time anyone is asking whether the doors are adequate.
 *
 * # Everything here changes the run
 *
 * No control on this panel is decorative. Population counts, speed and body
 * distributions, entry points and arrival profiles all reach the engine through
 * `ScenarioPlan`, and the figures shown beside them (occupant load, headroom)
 * are computed from the same numbers the compliance dossier uses. A control
 * that looked real and did nothing would be worse than no control, because a
 * reviewer would believe it.
 *
 * # Editing
 *
 * Every mutation goes through a `Command` from `doc/scenario.ts`, so undo and
 * redo work across scenario edits exactly as they do across geometry edits.
 * The panel never writes to the document.
 */

import { useState } from 'react';
import type { Arrival, Distribution, Population, ScenarioDoc } from '../schema/scenario';
import type { VenueDoc } from '../schema/venue';
import type { Command } from '../doc/commands';
import {
  DEFAULT_RADIUS,
  DEFAULT_SPEED,
  addAlarm,
  addClosure,
  addPopulation,
  removeEvent,
  setEventTime,
  sortedEvents,
  defaultCurve,
  goalOf,
  newPopulation,
  removePopulation,
  setArrival,
  setGoal,
  setPopulationCount,
  setPopulationLabel,
  setProfileDistribution,
  setScenarioSeed,
  totalAgents,
} from '../doc/scenario';
import { ArrivalPlot, profileOf } from './ArrivalPlot';
import { IconAgents, IconDelete } from './Icon';
import { occupantLoad, useApp } from '../state/store';

interface Props {
  scenario: ScenarioDoc | null;
  venue: VenueDoc | null;
  onEdit: (c: Command<ScenarioDoc>) => void;
}

export function ScenarioPanel({ scenario, venue, onEdit }: Props) {
  const [openPop, setOpenPop] = useState<string | null>(null);
  const thresholds = useApp((s) => s.thresholds);
  // Read from the store rather than threaded through: this is the compiled
  // venue's area, and the compiler is what put it there.
  const walkableArea = useApp((s) => s.walkableArea);

  if (!scenario || !venue) {
    return (
      <section className="panel">
        <h2 className="panel-title">Scenario</h2>
        <p className="row-note">Load a venue to author a scenario.</p>
      </section>
    );
  }

  const total = totalAgents(scenario);
  const limit = occupantLoad(walkableArea, thresholds.occupantLoadFactorM2);
  // The same arithmetic the dossier prints, so the two can never disagree.
  const over = limit > 0 && total > limit;

  return (
    <section className="panel">
      <h2 className="panel-title">
        Scenario
        <span className="panel-count">{scenario.populations.length}</span>
      </h2>

      <div className="rows">
        <div className="row">
          <span className="row-label">Total agents</span>
          <span className={`row-value${over ? ' is-alarm' : ''}`}>{total.toLocaleString()}</span>
        </div>
        <div className="row">
          <span className="row-label">Occupant load</span>
          <span className="row-value">{limit.toLocaleString()}</span>
        </div>
        {over && (
          <p className="row-note is-alarm">
            {(total - limit).toLocaleString()} over the NFPA 101 load for{' '}
            {walkableArea.toFixed(0)} m² at {thresholds.occupantLoadFactorM2} m²/person.
          </p>
        )}
      </div>

      <div className="field">
        <label className="field-label" htmlFor="scn-seed">
          Seed
        </label>
        <div className="field-group">
          <input
            id="scn-seed"
            className="field-input"
            type="number"
            value={scenario.seed}
            onChange={(e) => {
              const v = Number(e.target.value);
              if (Number.isFinite(v)) onEdit(setScenarioSeed(scenario, Math.trunc(v)));
            }}
          />
        </div>
        <p className="row-note">
          Same seed and same inputs give bit-identical results. Change it to sample a
          different draw from the distributions below.
        </p>
      </div>

      {scenario.populations.map((pop) => (
        <PopulationBlock
          key={pop.id}
          pop={pop}
          scenario={scenario}
          venue={venue}
          open={openPop === pop.id}
          onToggle={() => setOpenPop(openPop === pop.id ? null : pop.id)}
          onEdit={onEdit}
          canRemove={scenario.populations.length > 1}
        />
      ))}

      <button
        type="button"
        className="btn btn-wide"
        onClick={() =>
          onEdit(
            addPopulation(
              scenario,
              newPopulation(venue, `Population ${scenario.populations.length + 1}`, 100),
            ),
          )
        }
      >
        <IconAgents /> Add population
      </button>

      <EventList scenario={scenario} venue={venue} onEdit={onEdit} />
    </section>
  );
}

/**
 * What happens partway through the run.
 *
 * Only the two the engine acts on are offered — sounding the alarm and shutting
 * a doorway. `openOpening` and `blockLink` are in the schema and round-trip,
 * but nothing reads them, and a control that edits an ignored field is worse
 * than no control because a reviewer will believe it.
 *
 * Shutting a door mid-evacuation is the question a static occupant-load
 * calculation cannot answer, so it is the one thing here worth demonstrating.
 */
function EventList({
  scenario,
  venue,
  onEdit,
}: {
  scenario: ScenarioDoc;
  venue: VenueDoc;
  onEdit: (c: Command<ScenarioDoc>) => void;
}) {
  const events = sortedEvents(scenario);
  const doors = venue.floors.flatMap((f) => f.openings ?? []);
  // Index into the *document* order, which is what the commands address.
  const indexOf = (e: (typeof events)[number]) => (scenario.events ?? []).indexOf(e);

  return (
    <>
      <h2 className="panel-title">
        Events
        <span className="panel-count">{events.length}</span>
      </h2>

      {events.length === 0 && (
        <p className="row-note">
          Nothing happens partway through. Add an alarm to hold the crowd until it sounds, or
          shut a doorway to test whether the remaining exits cope.
        </p>
      )}

      <div className="rows">
        {events.map((e) => {
          const i = indexOf(e);
          return (
            <div className="row event" key={`${i}-${e.kind}`}>
              <input
                className="field-input event-time"
                type="number"
                min={0}
                max={scenario.durationS}
                aria-label="Event time, seconds"
                value={e.atS}
                onChange={(ev) => onEdit(setEventTime(scenario, i, Number(ev.target.value)))}
              />
              <span className="field-unit">s</span>
              <span className="event-what">
                {e.kind === 'alarm'
                  ? 'Alarm — everyone leaves'
                  : `Shut ${labelOfDoor(doors, 'target' in e ? e.target : '')}`}
              </span>
              <button
                type="button"
                className="btn-icon"
                aria-label="Remove event"
                onClick={() => onEdit(removeEvent(scenario, i))}
              >
                <IconDelete />
              </button>
            </div>
          );
        })}
      </div>

      <div className="field-pair">
        <button
          type="button"
          className="btn"
          onClick={() => onEdit(addAlarm(scenario, Math.round(scenario.durationS * 0.05)))}
        >
          Add alarm
        </button>
        <button
          type="button"
          className="btn"
          disabled={doors.length === 0}
          onClick={() => {
            const d = doors[0];
            if (d) onEdit(addClosure(scenario, Math.round(scenario.durationS * 0.1), d.id));
          }}
        >
          Shut a door
        </button>
      </div>
    </>
  );
}

/** A door's name for the event list, or its id if it is not on this floor. */
function labelOfDoor(
  doors: { id: string; kind?: string; widthM: number }[],
  id: string,
): string {
  const d = doors.find((x) => x.id === id);
  return d ? `${d.widthM.toFixed(2)} m ${d.kind ?? 'opening'}` : id;
}

interface PopProps {
  pop: Population;
  scenario: ScenarioDoc;
  venue: VenueDoc;
  open: boolean;
  canRemove: boolean;
  onToggle: () => void;
  onEdit: (c: Command<ScenarioDoc>) => void;
}

function PopulationBlock({
  pop,
  scenario,
  venue,
  open,
  canRemove,
  onToggle,
  onEdit,
}: PopProps) {
  const goal = goalOf(pop);
  // Anything a person can walk through is a candidate entry. `isFireExit`
  // marks the subset that also counts toward code-required egress, which is a
  // compliance question rather than an authoring one.
  const doors = venue.floors.flatMap((f) => f.openings ?? []);
  const zones = venue.floors.flatMap((f) => f.zones ?? []).filter((z) => !z.isVoid);

  return (
    <div className={`pop${open ? ' is-open' : ''}`}>
      <button type="button" className="pop-head" onClick={onToggle} aria-expanded={open}>
        <span className="pop-caret" aria-hidden="true" />
        <span className="pop-label">{pop.label}</span>
        <span className="pop-count">{pop.count.toLocaleString()}</span>
      </button>

      {open && (
        <div className="pop-body">
          <div className="field">
            <label className="field-label" htmlFor={`lbl-${pop.id}`}>
              Label
            </label>
            <input
              id={`lbl-${pop.id}`}
              className="field-input"
              value={pop.label}
              onChange={(e) => onEdit(setPopulationLabel(scenario, pop.id, e.target.value))}
            />
          </div>

          <div className="field">
            <label className="field-label" htmlFor={`cnt-${pop.id}`}>
              Count
            </label>
            <div className="field-group">
              <input
                id={`cnt-${pop.id}`}
                className="field-input"
                type="number"
                min={0}
                value={pop.count}
                onChange={(e) => {
                  const v = Number(e.target.value);
                  if (Number.isFinite(v) && v >= 0) {
                    onEdit(setPopulationCount(scenario, pop.id, Math.trunc(v)));
                  }
                }}
              />
              <span className="field-unit">people</span>
            </div>
          </div>

          <DistributionField
            label="Walking speed"
            unit="m/s"
            dist={pop.profile.desiredSpeed ?? DEFAULT_SPEED}
            onChange={(d) => onEdit(setProfileDistribution(scenario, pop.id, 'desiredSpeed', d))}
            note="Weidmann (1993) gives a mean of 1.34 m/s, sd 0.26, for level walking."
          />

          <DistributionField
            label="Body radius"
            unit="m"
            dist={pop.profile.radiusM ?? DEFAULT_RADIUS}
            onChange={(d) => onEdit(setProfileDistribution(scenario, pop.id, 'radiusM', d))}
            note="Shoulder half-width. Sets how tightly the crowd can pack at a door."
          />

          <ArrivalField
            pop={pop}
            scenario={scenario}
            doors={doors.map((d) => ({ id: d.id, label: labelOfOpening(d) }))}
            zones={zones.map((z) => ({ id: z.id, label: z.name ?? z.id }))}
            onEdit={onEdit}
          />

          <div className="field">
            <label className="field-label" htmlFor={`goal-${pop.id}`}>
              Goal
            </label>
            <select
              id={`goal-${pop.id}`}
              className="field-select"
              value={goal.target === 'nearestExit' ? 'nearestExit' : `zone:${goal.id}`}
              onChange={(e) => {
                const v = e.target.value;
                onEdit(
                  setGoal(
                    scenario,
                    pop.id,
                    v === 'nearestExit'
                      ? { target: 'nearestExit' }
                      : { target: 'zone', id: v.slice(5) },
                  ),
                );
              }}
            >
              <option value="nearestExit">Nearest exit</option>
              {zones.map((z) => (
                <option key={z.id} value={`zone:${z.id}`}>
                  Dwell in {z.name ?? z.id}
                </option>
              ))}
            </select>
          </div>

          {canRemove && (
            <button
              type="button"
              className="btn btn-wide btn-danger"
              onClick={() => onEdit(removePopulation(scenario, pop.id))}
            >
              <IconDelete /> Remove population
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * A normal distribution, edited as mean and spread.
 *
 * Only the normal case is editable here. The schema supports more, but a
 * planner reasons in "average and spread", and offering a lognormal shape
 * parameter on this panel would be offering a control nobody on site can
 * answer. Other kinds are shown read-only rather than silently rewritten.
 */
function DistributionField({
  label,
  unit,
  dist,
  note,
  onChange,
}: {
  label: string;
  unit: string;
  dist: Distribution;
  note: string;
  onChange: (d: Distribution) => void;
}) {
  if (dist.dist !== 'normal') {
    return (
      <div className="field">
        <span className="field-label">{label}</span>
        <p className="row-note">
          {dist.dist} distribution — edit as JSON; this panel edits normal distributions.
        </p>
      </div>
    );
  }

  return (
    <div className="field">
      <span className="field-label">{label}</span>
      <div className="field-pair">
        <div className="field-group">
          <input
            className="field-input"
            type="number"
            step="0.01"
            aria-label={`${label} mean`}
            value={dist.mean}
            onChange={(e) => {
              const v = Number(e.target.value);
              if (Number.isFinite(v)) onChange({ ...dist, mean: v });
            }}
          />
          <span className="field-unit">{unit}</span>
        </div>
        <div className="field-group">
          <input
            className="field-input"
            type="number"
            step="0.01"
            min={0}
            aria-label={`${label} standard deviation`}
            value={dist.sd}
            onChange={(e) => {
              const v = Number(e.target.value);
              if (Number.isFinite(v) && v >= 0) onChange({ ...dist, sd: v });
            }}
          />
          <span className="field-unit">sd</span>
        </div>
      </div>
      <p className="row-note">{note}</p>
    </div>
  );
}

const ARRIVAL_KINDS: { id: Arrival['kind']; label: string }[] = [
  { id: 'preplaced', label: 'Already inside' },
  { id: 'uniform', label: 'Steady' },
  { id: 'curve', label: 'Curve' },
];

function ArrivalField({
  pop,
  scenario,
  doors,
  zones,
  onEdit,
}: {
  pop: Population;
  scenario: ScenarioDoc;
  doors: { id: string; label: string }[];
  zones: { id: string; label: string }[];
  onEdit: (c: Command<ScenarioDoc>) => void;
}) {
  const arrival = pop.arrival;
  const duration = scenario.durationS;

  const change = (kind: Arrival['kind']) => {
    if (kind === arrival.kind) return;
    const entries =
      'entries' in arrival && arrival.entries.length
        ? arrival.entries
        : doors.slice(0, 1).map((d) => ({ opening: d.id, weight: 1 }));
    const next: Arrival =
      kind === 'preplaced'
        ? { kind: 'preplaced', zones: zones.slice(0, 1).map((z) => ({ zone: z.id, weight: 1 })) }
        : kind === 'uniform'
          ? { kind: 'uniform', entries }
          : { kind: 'curve', entries, points: defaultCurve(duration) };
    onEdit(setArrival(scenario, pop.id, next));
  };

  return (
    <div className="field">
      <span className="field-label">Arrival</span>
      <div className="seg" role="group" aria-label="Arrival profile">
        {ARRIVAL_KINDS.map((k) => (
          <button
            key={k.id}
            type="button"
            className={`seg-btn${arrival.kind === k.id ? ' is-on' : ''}`}
            aria-pressed={arrival.kind === k.id}
            onClick={() => change(k.id)}
          >
            {k.label}
          </button>
        ))}
      </div>

      <ArrivalPlot
        points={profileOf(arrival, duration)}
        durationS={duration}
        onChange={
          arrival.kind === 'curve'
            ? (points) => onEdit(setArrival(scenario, pop.id, { ...arrival, points }))
            : undefined
        }
      />

      {arrival.kind === 'preplaced' ? (
        <p className="row-note">
          {arrival.zones.length
            ? 'Placed in the named zone before the first tick.'
            : 'No zones defined — scattered across the walkable floor.'}
        </p>
      ) : (
        <>
          <label className="field-label" htmlFor={`entry-${pop.id}`}>
            Entry
          </label>
          <select
            id={`entry-${pop.id}`}
            className="field-select"
            value={arrival.entries[0]?.opening ?? ''}
            onChange={(e) =>
              onEdit(
                setArrival(scenario, pop.id, {
                  ...arrival,
                  entries: [{ opening: e.target.value, weight: 1 }],
                }),
              )
            }
          >
            {doors.length === 0 && <option value="">No exits defined</option>}
            {doors.map((d) => (
              <option key={d.id} value={d.id}>
                {d.label}
              </option>
            ))}
          </select>
          <p className="row-note">
            {arrival.kind === 'curve'
              ? 'Drag a handle to reshape. Arrow keys work too.'
              : `Everyone enters at a constant rate over ${Math.round(duration / 60)} min.`}
          </p>
        </>
      )}
    </div>
  );
}

/** A door's human-readable name: its kind and clear width. */
function labelOfOpening(o: { id: string; kind?: string; widthM: number; isFireExit?: boolean }) {
  const kind = o.kind === 'doubleDoor' ? 'Double door' : o.kind ? o.kind : 'Opening';
  const name = kind.charAt(0).toUpperCase() + kind.slice(1);
  return `${name} · ${o.widthM.toFixed(2)} m${o.isFireExit ? ' · fire exit' : ''}`;
}
