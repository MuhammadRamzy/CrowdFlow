/**
 * The life-safety status bar — the signature element (docs/08-frontend-plan.md).
 *
 * Modelled on a fire alarm annunciator: the panel that already lives in these
 * buildings and whose vocabulary a safety officer reads without a legend. It is
 * permanent rather than buried in a report, because the product's proposition
 * is "your venue is always being audited and you can always see the verdict".
 *
 * Every figure here is computed by the engine or derived from a published code
 * threshold. Nothing is illustrative.
 */

import { formatClock, occupantLoad, safetyVerdict, useApp } from '../state/store';
import type { SafetyState } from '../state/store';

const GLYPH: Record<SafetyState, string> = {
  normal: '●',
  supervise: '▲',
  alarm: '■',
};

const LABEL: Record<SafetyState, string> = {
  normal: 'NORMAL',
  supervise: 'SUPERVISE',
  alarm: 'ALARM',
};

interface FieldProps {
  label: string;
  value: string;
  limit?: string;
  /** Units are separated so they can be set quieter than the number. */
  unit?: string;
  breached?: boolean;
}

function Field({ label, value, limit, unit, breached }: FieldProps) {
  return (
    <div className="sb-field">
      <span className="sb-label">{label}</span>
      <span className={`sb-value${breached ? ' is-breached' : ''}`}>
        {value}
        {limit !== undefined && <span className="sb-limit"> / {limit}</span>}
        {unit && <span className="sb-unit"> {unit}</span>}
        {breached && <span className="sb-x" aria-label="exceeds limit"> ✕</span>}
      </span>
    </div>
  );
}

export function StatusBar() {
  const { walkableArea, warnings, stats, egressTime, thresholds, venueName, peakOccupancy } =
    useApp();

  const limit = occupantLoad(walkableArea, thresholds.occupantLoadFactorM2);
  const occupancy = stats?.active ?? 0;
  const fatal = warnings.filter((w) => w.fatal).length;

  const verdict = safetyVerdict({
    // Judged on the peak, not the current count — see `peakOccupancy`.
    occupancy: Math.max(occupancy, peakOccupancy),
    occupantLimit: limit,
    egressS: egressTime,
    targetEgressS: thresholds.targetEgressS,
    fatalWarnings: fatal,
  });

  const occupancyBreached = limit > 0 && Math.max(occupancy, peakOccupancy) > limit;
  const egressBreached = egressTime !== null && egressTime > thresholds.targetEgressS;

  return (
    <header className={`statusbar state-${verdict}`} role="status" aria-live="polite">
      <div className="sb-verdict">
        <span className="sb-glyph" aria-hidden="true">
          {GLYPH[verdict]}
        </span>
        <span className="sb-state">{LABEL[verdict]}</span>
      </div>

      <div className="sb-venue" title={venueName}>
        {venueName || '—'}
      </div>

      <div className="sb-fields">
        <Field
          label="OCCUPANCY"
          value={occupancy.toLocaleString()}
          limit={limit > 0 ? limit.toLocaleString() : undefined}
          breached={occupancyBreached}
        />
        <Field
          label="PEAK"
          value={peakOccupancy > 0 ? peakOccupancy.toLocaleString() : '—'}
          limit={limit > 0 ? limit.toLocaleString() : undefined}
          breached={limit > 0 && peakOccupancy > limit}
        />
        <Field
          label="EGRESS"
          value={egressTime === null ? '—' : formatClock(egressTime)}
          limit={formatClock(thresholds.targetEgressS)}
          breached={egressBreached}
        />
        <Field
          label="FLOOR AREA"
          value={walkableArea > 0 ? walkableArea.toFixed(1) : '—'}
          unit="m²"
        />
        <Field
          label="CLEARED"
          value={stats ? `${stats.exited.toLocaleString()}` : '—'}
          limit={stats ? stats.spawned.toLocaleString() : undefined}
        />
      </div>

      {fatal > 0 && (
        <div className="sb-note">
          {fatal} fatal {fatal === 1 ? 'issue' : 'issues'} — cannot simulate
        </div>
      )}
    </header>
  );
}
