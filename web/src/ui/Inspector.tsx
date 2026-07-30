/**
 * Venue and run properties.
 *
 * Every value shown is measured by the engine. Occupant load is derived from
 * NFPA 101 Table 7.3.1.2 and the walkable area the compiler computed — not an
 * estimate, and not a placeholder.
 */

import { useRef } from 'react';
import type { Selection } from '../canvas/tools';
import { polylineLength } from '../canvas/tools';
import type { VenueDoc, ZoneKind } from '../schema/venue';
import type { Command } from '../doc/commands';
import {
  setOpeningFireExit,
  setOpeningWidth,
  setWallThickness,
  setZoneKind,
} from '../doc/commands';
import { occupantLoad, useApp } from '../state/store';

interface Props {
  venueTitle: string;
  /** How many agents were actually placed, which may be fewer than requested. */
  placedAgents: number;
  selection: Selection | null;
  document: VenueDoc | null;
  onDeleteSelection: () => void;
  onEdit: (cmd: Command) => void;
  onLoadFile: (f: File) => void;
  onReset: () => void;
}

export function Inspector({
  venueTitle,
  placedAgents,
  selection,
  document: doc,
  onDeleteSelection,
  onEdit,
  onLoadFile,
  onReset,
}: Props) {
  const fileRef = useRef<HTMLInputElement | null>(null);
  const {
    walkableArea,
    simulable,
    stats,
    requestedAgents,
    setRequestedAgents,
    thresholds,
    engineVersion,
    showHeatmap,
    setShowHeatmap,
    heatmapPeak,
    setHeatmapPeak,
    peakDensity,
    criticalArea,
  } = useApp();

  const limit = occupantLoad(walkableArea, thresholds.occupantLoadFactorM2);
  const density = stats && walkableArea > 0 ? stats.active / walkableArea : 0;

  return (
    <section className="panel">
      {selection && doc && (
        <SelectionPanel
          selection={selection}
          doc={doc}
          onDelete={onDeleteSelection}
          onEdit={onEdit}
        />
      )}

      <h2 className="panel-title">Venue</h2>

      <div className="rows">
        <Row label="Name" value={venueTitle} />
        <Row label="Walkable" value={`${walkableArea.toFixed(1)} m²`} />
        <Row
          label="Occupant load"
          value={limit > 0 ? limit.toLocaleString() : '—'}
          note={`NFPA 101 · ${thresholds.occupantLoadFactorM2} m²/p`}
        />
        <Row label="Simulable" value={simulable ? 'yes' : 'no'} />
      </div>

      <div className="actions">
        <input
          ref={fileRef}
          type="file"
          accept=".json"
          hidden
          onChange={(e) => {
            const f = e.target.files?.[0];
            if (f) onLoadFile(f);
            e.target.value = '';
          }}
        />
        <button type="button" className="btn" onClick={() => fileRef.current?.click()}>
          Open venue…
        </button>
      </div>

      <h2 className="panel-title">Simulation</h2>

      <label className="field">
        <span className="field-label">Agents</span>
        <input
          className="field-input"
          type="number"
          min={1}
          max={20000}
          step={100}
          value={requestedAgents}
          onChange={(e) => setRequestedAgents(Math.max(1, Number(e.target.value) || 1))}
        />
      </label>

      <div className="actions">
        <button type="button" className="btn btn-primary" onClick={onReset} disabled={!simulable}>
          Place agents
        </button>
      </div>

      {placedAgents > 0 && placedAgents < requestedAgents && (
        <p className="hint">
          Placed {placedAgents.toLocaleString()} of {requestedAgents.toLocaleString()}. The rest
          would have overlapped a body already on the floor.
        </p>
      )}

      <h2 className="panel-title">Density</h2>

      <label className="check">
        <input
          type="checkbox"
          checked={showHeatmap}
          onChange={(e) => setShowHeatmap(e.target.checked)}
        />
        <span>Show heatmap</span>
      </label>

      <label className="check">
        <input
          type="checkbox"
          checked={heatmapPeak}
          onChange={(e) => setHeatmapPeak(e.target.checked)}
          disabled={!showHeatmap}
        />
        <span>Worst reached, not current</span>
      </label>

      <Legend />

      {peakDensity > 0 && (
        <div className="rows">
          <Row
            label="Peak density"
            value={`${peakDensity.toFixed(2)} p/m²`}
            note={peakDensity >= thresholds.criticalDensity ? 'above crush threshold' : undefined}
          />
          <Row
            label="Critical area"
            value={`${criticalArea.toFixed(1)} m²`}
            note={`at or above ${thresholds.criticalDensity} p/m²`}
          />
        </div>
      )}

      {stats && (
        <div className="rows">
          <Row label="In venue" value={stats.active.toLocaleString()} />
          <Row label="Cleared" value={stats.exited.toLocaleString()} />
          <Row label="Blocked" value={stats.blocked.toLocaleString()} />
          <Row
            label="Mean density"
            value={`${density.toFixed(2)} p/m²`}
            note={`critical ${thresholds.criticalDensity}`}
          />
          {stats.escaped > 0 && (
            <Row label="Recovered" value={String(stats.escaped)} note="physics leak" />
          )}
        </div>
      )}

      <p className="build">{engineVersion}</p>
    </section>
  );
}

/**
 * Properties of the selected element.
 *
 * Reads as a spec sheet rather than a form: these are measured facts about a
 * piece of geometry, and the units carry as much meaning as the numbers. Wall
 * length and door width are the figures egress capacity is computed from, so
 * they are stated plainly rather than buried in an editor.
 */
function SelectionPanel({
  selection,
  doc,
  onDelete,
  onEdit,
}: {
  selection: Selection;
  doc: VenueDoc;
  onDelete: () => void;
  onEdit: (cmd: Command) => void;
}) {
  const floor = doc.floors[0];
  if (!floor) return null;
  const floorId = floor.id;

  if (selection.kind === 'wall') {
    const w = (floor.walls ?? []).find((x) => x.id === selection.id);
    if (!w) return null;
    const doors = (floor.openings ?? []).filter((o) => o.wall === w.id).length;
    return (
      <Panel title="Wall" id={w.id} onDelete={onDelete}>
        <div className="rows">
          <Row label="Length" value={`${polylineLength(w.polyline).toFixed(2)} m`} />
          <Row label="Vertices" value={String(w.polyline.length)} />
          <Row label="Kind" value={w.kind ?? 'structural'} />
          {doors > 0 && (
            <Row label="Openings" value={String(doors)} note="removed with the wall" />
          )}
        </div>
        <NumberField
          label="Thickness"
          unit="m"
          value={w.thicknessM ?? 0.2}
          min={0.05}
          max={2}
          step={0.01}
          onChange={(v) => onEdit(setWallThickness(doc, floorId, w.id, v))}
        />
      </Panel>
    );
  }

  if (selection.kind === 'zone') {
    const z = (floor.zones ?? []).find((x) => x.id === selection.id);
    if (!z) return null;
    const area = polygonArea(z.polygon);
    const olf = OCCUPANT_LOAD_FACTORS[z.kind as ZoneKind] ?? null;
    return (
      <Panel title="Zone" id={z.id} onDelete={onDelete}>
        <div className="rows">
          <Row label="Area" value={`${area.toFixed(1)} m²`} />
          <Row label="Vertices" value={String(z.polygon.length)} />
          {olf !== null && (
            <Row
              label="Occupant load"
              value={Math.floor(area / olf).toLocaleString()}
              note={`NFPA 101 · ${olf} m²/p`}
            />
          )}
        </div>
        <SelectField
          label="Use"
          value={String(z.kind)}
          options={Object.keys(OCCUPANT_LOAD_FACTORS).map((k) => [k, ZONE_LABELS[k] ?? k])}
          onChange={(v) => onEdit(setZoneKind(doc, floorId, z.id, v as ZoneKind))}
        />
      </Panel>
    );
  }

  const o = (floor.openings ?? []).find((x) => x.id === selection.id);
  if (!o) return null;
  const narrow = o.widthM < MIN_EGRESS_WIDTH_M;
  return (
    <Panel title="Doorway" id={o.id} onDelete={onDelete}>
      <NumberField
        label="Clear width"
        unit="m"
        value={o.widthM}
        min={0.3}
        max={12}
        step={0.05}
        warn={narrow}
        onChange={(v) => onEdit(setOpeningWidth(doc, floorId, o.id, v))}
      />
      <div className="rows">
        <Row label="Kind" value={o.kind ?? 'door'} />
        <Row
          label="Rate of passage"
          value={`${Math.round(o.widthM * GREEN_GUIDE_LEVEL)} p/min`}
          note="Green Guide, level"
        />
        {narrow && (
          <Row
            label="Minimum"
            value={`${MIN_EGRESS_WIDTH_M.toFixed(2)} m`}
            note="below the egress minimum"
          />
        )}
      </div>
      <label className="check">
        <input
          type="checkbox"
          checked={o.isFireExit ?? false}
          onChange={(e) => onEdit(setOpeningFireExit(doc, floorId, o.id, e.target.checked))}
        />
        <span>Fire exit</span>
      </label>
    </Panel>
  );
}

/** Green Guide rate of passage on the level, persons per metre per minute. */
const GREEN_GUIDE_LEVEL = 82;
/** NFPA 101 requires 32 in clear; 0.85 m leaves margin for door hardware. */
const MIN_EGRESS_WIDTH_M = 0.85;

/** NFPA 101 Table 7.3.1.2, net m² per person. */
const OCCUPANT_LOAD_FACTORS: Record<string, number> = {
  assemblyStandingSpace: 0.46,
  assemblyConcentrated: 0.65,
  assemblyLessConcentrated: 1.4,
  assemblyFixedSeating: 0.65,
  circulation: 1.4,
  mercantile: 2.8,
  business: 9.3,
  storage: 46.5,
  backOfHouse: 9.3,
  queue: 0.46,
  restricted: 9.3,
  exterior: 9.3,
};

const ZONE_LABELS: Record<string, string> = {
  assemblyStandingSpace: 'Assembly — standing',
  assemblyConcentrated: 'Assembly — concentrated',
  assemblyLessConcentrated: 'Assembly — dining/exhibition',
  assemblyFixedSeating: 'Assembly — fixed seating',
  circulation: 'Circulation',
  mercantile: 'Mercantile',
  business: 'Business',
  storage: 'Storage',
  backOfHouse: 'Back of house',
  queue: 'Queue',
  restricted: 'Restricted',
  exterior: 'Exterior',
};

function Panel({
  title,
  id,
  onDelete,
  children,
}: {
  title: string;
  id: string;
  onDelete: () => void;
  children: React.ReactNode;
}) {
  return (
    <>
      <h2 className="panel-title">
        {title}
        <span className="panel-count">{id}</span>
      </h2>
      {children}
      <div className="actions">
        <button type="button" className="btn btn-danger" onClick={onDelete}>
          Delete
        </button>
      </div>
    </>
  );
}

/**
 * A numeric property.
 *
 * Committed on change rather than on blur, so the compliance figures beside it
 * move as the value does — watching the rate of passage climb while widening a
 * door is the point. Rapid edits coalesce into one undo entry.
 */
function NumberField({
  label,
  unit,
  value,
  min,
  max,
  step,
  warn,
  onChange,
}: {
  label: string;
  unit: string;
  value: number;
  min: number;
  max: number;
  step: number;
  warn?: boolean;
  onChange: (v: number) => void;
}) {
  const commit = (raw: string) => {
    const n = Number(raw);
    if (!Number.isFinite(n)) return;
    onChange(Math.min(max, Math.max(min, n)));
  };
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      <span className="field-group">
        <input
          className={`field-input${warn ? ' is-warn' : ''}`}
          type="number"
          value={value}
          min={min}
          max={max}
          step={step}
          onChange={(e) => commit(e.target.value)}
        />
        <span className="field-unit">{unit}</span>
      </span>
    </label>
  );
}

function SelectField({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: Array<[string, string]>;
  onChange: (v: string) => void;
}) {
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      <select className="field-select" value={value} onChange={(e) => onChange(e.target.value)}>
        {options.map(([v, text]) => (
          <option key={v} value={v}>
            {text}
          </option>
        ))}
      </select>
    </label>
  );
}

/** Shoelace area of a closed ring, m². */
function polygonArea(poly: number[][]): number {
  let acc = 0;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    acc += poly[j]![0]! * poly[i]![1]! - poly[i]![0]! * poly[j]![1]!;
  }
  return Math.abs(acc) / 2;
}

/**
 * The density bands, labelled.
 *
 * The colours are meaningless without the thresholds beside them: 6 p/m² is
 * where forward movement ceases, and a reader must be able to tell that band
 * from the one below it at a glance.
 */
function Legend() {
  const bands: Array<[string, string]> = [
    ['#2e7d9a', '0–2'],
    ['#46b08a', '2–3'],
    ['#c4c04a', '3–4'],
    ['#e08a3c', '4–6'],
    ['#d43f3f', '6+'],
  ];
  return (
    <div className="legend" aria-label="Density bands, persons per square metre">
      {bands.map(([colour, label]) => (
        <div className="legend-band" key={label}>
          <span className="legend-swatch" style={{ background: colour }} aria-hidden="true" />
          <span className="legend-label">{label}</span>
        </div>
      ))}
      <span className="legend-unit">p/m²</span>
    </div>
  );
}

function Row({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="row">
      <span className="row-label">{label}</span>
      <span className="row-value">
        {value}
        {note && <span className="row-note">{note}</span>}
      </span>
    </div>
  );
}
