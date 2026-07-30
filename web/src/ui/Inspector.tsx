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
  onLoadFile: (f: File) => void;
  onReset: () => void;
}

export function Inspector({ venueTitle, onLoadFile, onReset }: Props) {
  const fileRef = useRef<HTMLInputElement | null>(null);
  const {
    walkableArea,
    simulable,
    stats,
    requestedAgents,
    setRequestedAgents,
    thresholds,
    engineVersion,
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
