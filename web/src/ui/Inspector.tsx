/**
 * Venue and run properties.
 *
 * Every value shown is measured by the engine. Occupant load is derived from
 * NFPA 101 Table 7.3.1.2 and the walkable area the compiler computed — not an
 * estimate, and not a placeholder.
 */

import { useRef } from 'react';
import { occupantLoad, useApp } from '../state/store';

interface Props {
  venueTitle: string;
  /** How many agents were actually placed, which may be fewer than requested. */
  placedAgents: number;
  onLoadFile: (f: File) => void;
  onReset: () => void;
}

export function Inspector({ venueTitle, placedAgents, onLoadFile, onReset }: Props) {
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
