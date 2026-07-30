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
import type { VenueDoc } from '../schema/venue';
import { occupantLoad, useApp } from '../state/store';

interface Props {
  venueTitle: string;
  /** How many agents were actually placed, which may be fewer than requested. */
  placedAgents: number;
  selection: Selection | null;
  document: VenueDoc | null;
  onDeleteSelection: () => void;
  onLoadFile: (f: File) => void;
  onReset: () => void;
}

export function Inspector({
  venueTitle,
  placedAgents,
  selection,
  document: doc,
  onDeleteSelection,
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
        <SelectionPanel selection={selection} doc={doc} onDelete={onDeleteSelection} />
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
}: {
  selection: Selection;
  doc: VenueDoc;
  onDelete: () => void;
}) {
  const floor = doc.floors[0];
  const rows: Array<[string, string, string?]> = [];
  let title = 'Selection';

  if (floor && selection.kind === 'wall') {
    const w = (floor.walls ?? []).find((x) => x.id === selection.id);
    if (w) {
      title = 'Wall';
      rows.push(['Length', `${polylineLength(w.polyline).toFixed(2)} m`]);
      rows.push(['Thickness', `${(w.thicknessM ?? 0.2).toFixed(2)} m`]);
      rows.push(['Vertices', String(w.polyline.length)]);
      rows.push(['Kind', w.kind ?? 'structural']);
      const doors = (floor.openings ?? []).filter((o) => o.wall === w.id).length;
      if (doors > 0) rows.push(['Openings', String(doors), 'removed with the wall']);
    }
  } else if (floor && selection.kind === 'zone') {
    const z = (floor.zones ?? []).find((x) => x.id === selection.id);
    if (z) {
      title = 'Zone';
      const area = polygonArea(z.polygon);
      rows.push(['Area', `${area.toFixed(1)} m²`]);
      rows.push(['Kind', String(z.kind)]);
      rows.push(['Vertices', String(z.polygon.length)]);
    }
  } else if (floor && selection.kind === 'opening') {
    const o = (floor.openings ?? []).find((x) => x.id === selection.id);
    if (o) {
      title = 'Doorway';
      rows.push(['Clear width', `${o.widthM.toFixed(2)} m`, o.widthM < 0.85 ? 'below minimum' : undefined]);
      rows.push(['Kind', o.kind ?? 'door']);
      rows.push(['Fire exit', o.isFireExit ? 'yes' : 'no']);
      // Green Guide: 82 persons per metre per minute on the level.
      rows.push([
        'Rate of passage',
        `${Math.round(o.widthM * 82)} p/min`,
        'Green Guide, level',
      ]);
    }
  }

  if (!rows.length) return null;

  return (
    <>
      <h2 className="panel-title">
        {title}
        <span className="panel-count">{selection.id}</span>
      </h2>
      <div className="rows">
        {rows.map(([label, value, note]) => (
          <Row key={label} label={label} value={value} note={note} />
        ))}
      </div>
      <div className="actions">
        <button type="button" className="btn btn-danger" onClick={onDelete}>
          Delete
        </button>
      </div>
    </>
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
