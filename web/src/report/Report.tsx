/**
 * The compliance dossier.
 *
 * This is the deliverable — the document a safety officer hands to a licensing
 * authority — and it is deliberately the opposite surface to the workspace.
 * The workspace is a dark instrument you sit in for hours; the report is a
 * drawing on paper. That split is the design thesis in
 * `docs/08-frontend-plan.md`, and this is where the second half of it lands.
 *
 * Printed via the browser's own print pipeline rather than a PDF library:
 * print CSS produces a correct, selectable, accessible PDF with no dependency
 * and no server round trip, and it works offline. `window.print()` is the
 * export button.
 *
 * # Every figure is computed
 *
 * Nothing here is illustrative. Occupant load comes from NFPA 101 applied to
 * the area the compiler measured; egress time is what the simulation produced;
 * rates of passage are Green Guide arithmetic on measured clear widths. Where a
 * value has not been produced, the report says so rather than estimating.
 */

import type { CompileWarning, SimStats } from '../engine/bridge';
import { formatClock, occupantLoad } from '../state/store';
import type { Thresholds } from '../state/store';
import type { VenueDoc } from '../schema/venue';

/** Green Guide rate of passage on the level, persons per metre per minute. */
const GREEN_GUIDE_LEVEL = 82;
const MIN_EGRESS_WIDTH_M = 0.85;

export interface ReportData {
  venueName: string;
  document: VenueDoc | null;
  walkableArea: number;
  warnings: CompileWarning[];
  stats: SimStats | null;
  egressTime: number | null;
  peakOccupancy: number;
  peakDensity: number;
  criticalArea: number;
  thresholds: Thresholds;
  engineVersion: string;
  /** Data URL of the venue drawing. */
  plan: string | null;
  generatedAt: Date;
}

interface Finding {
  id: string;
  clause: string;
  status: 'pass' | 'fail' | 'unknown';
  requirement: string;
  measured: string;
  remediation?: string;
}

/**
 * Evaluate the venue against the code thresholds.
 *
 * A finding always states the requirement, the measured value, and — when it
 * fails — what would fix it with the shortfall filled in. A failure without a
 * remediation makes a reader do arithmetic the tool already did.
 */
function evaluate(d: ReportData): Finding[] {
  const findings: Finding[] = [];
  const limit = occupantLoad(d.walkableArea, d.thresholds.occupantLoadFactorM2);
  const floor = d.document?.floors[0];
  const doors = floor?.openings ?? [];
  const exits = doors.filter((o) => o.isFireExit);

  // --- occupant load ---------------------------------------------------
  if (d.peakOccupancy > 0) {
    const pass = d.peakOccupancy <= limit;
    findings.push({
      id: 'occupant-load',
      clause: 'NFPA 101 Table 7.3.1.2',
      status: pass ? 'pass' : 'fail',
      requirement: `Maximum ${limit.toLocaleString()} persons at ${d.thresholds.occupantLoadFactorM2} m² per person`,
      measured: `Peak ${d.peakOccupancy.toLocaleString()} persons`,
      remediation: pass
        ? undefined
        : `Reduce permitted occupancy to ${limit.toLocaleString()}, or increase net floor area by ` +
          `${((d.peakOccupancy - limit) * d.thresholds.occupantLoadFactorM2).toFixed(1)} m².`,
    });
  }

  // --- egress time -----------------------------------------------------
  if (d.egressTime !== null) {
    const pass = d.egressTime <= d.thresholds.targetEgressS;
    findings.push({
      id: 'egress-time',
      clause: 'Guide to Safety at Sports Grounds — 8 minute benchmark',
      status: pass ? 'pass' : 'fail',
      requirement: `Clearance within ${formatClock(d.thresholds.targetEgressS)}`,
      measured: `Cleared in ${formatClock(d.egressTime)}`,
      remediation: pass
        ? undefined
        : 'Increase total exit width or add an additional exit; see exit capacity below.',
    });
  }

  // --- exit capacity ---------------------------------------------------
  if (exits.length > 0 && d.peakOccupancy > 0) {
    const totalWidth = exits.reduce((sum, o) => sum + o.widthM, 0);
    const minutes = d.thresholds.targetEgressS / 60;
    const capacity = Math.floor(totalWidth * GREEN_GUIDE_LEVEL * minutes);
    const pass = capacity >= d.peakOccupancy;
    const needed = d.peakOccupancy / (GREEN_GUIDE_LEVEL * minutes);
    findings.push({
      id: 'exit-capacity',
      clause: 'Green Guide — rates of passage, 82 persons/m/min level',
      status: pass ? 'pass' : 'fail',
      requirement: `Capacity for ${d.peakOccupancy.toLocaleString()} persons in ${formatClock(d.thresholds.targetEgressS)}`,
      measured:
        `${totalWidth.toFixed(2)} m total clear width across ${exits.length} ` +
        `${exits.length === 1 ? 'exit' : 'exits'} — ${capacity.toLocaleString()} persons`,
      remediation: pass
        ? undefined
        : `Provide at least ${needed.toFixed(2)} m of clear exit width, an increase of ` +
          `${(needed - totalWidth).toFixed(2)} m.`,
    });
  }

  // --- minimum exit width ----------------------------------------------
  const narrow = doors.filter((o) => o.isFireExit && o.widthM < MIN_EGRESS_WIDTH_M);
  if (exits.length > 0) {
    findings.push({
      id: 'exit-width',
      clause: 'NFPA 101 §7.2.1 — minimum clear width',
      status: narrow.length === 0 ? 'pass' : 'fail',
      requirement: `Every exit at least ${MIN_EGRESS_WIDTH_M.toFixed(2)} m clear`,
      measured:
        narrow.length === 0
          ? `Narrowest exit ${Math.min(...exits.map((o) => o.widthM)).toFixed(2)} m`
          : `${narrow.length} ${narrow.length === 1 ? 'exit' : 'exits'} below minimum`,
      remediation:
        narrow.length === 0
          ? undefined
          : `Widen ${narrow.map((o) => o.id).join(', ')} to at least ${MIN_EGRESS_WIDTH_M.toFixed(2)} m.`,
    });
  }

  // --- provision of exits ----------------------------------------------
  findings.push({
    id: 'exit-provision',
    clause: 'NFPA 101 §7.4 — number of means of egress',
    status: exits.length >= 2 ? 'pass' : 'fail',
    requirement: 'At least two remote means of egress for an assembly space',
    measured: `${exits.length} marked fire ${exits.length === 1 ? 'exit' : 'exits'}`,
    remediation: exits.length >= 2 ? undefined : 'Provide a second exit remote from the first.',
  });

  // --- crowd density ---------------------------------------------------
  if (d.peakDensity > 0) {
    const pass = d.peakDensity < d.thresholds.criticalDensity;
    findings.push({
      id: 'crowd-density',
      clause: 'Crowd science — crush threshold',
      status: pass ? 'pass' : 'fail',
      requirement: `Peak density below ${d.thresholds.criticalDensity} persons/m²`,
      measured:
        `Peak ${d.peakDensity.toFixed(2)} persons/m²` +
        (d.criticalArea > 0 ? `, ${d.criticalArea.toFixed(1)} m² at or above threshold` : ''),
      remediation: pass
        ? undefined
        : 'Widen the constriction causing the build-up, or stagger arrivals to reduce peak demand.',
    });
  }

  return findings;
}

export function Report(props: { data: ReportData; onClose: () => void }) {
  const d = props.data;
  const findings = evaluate(d);
  const failures = findings.filter((f) => f.status === 'fail');
  const limit = occupantLoad(d.walkableArea, d.thresholds.occupantLoadFactorM2);
  const fatal = d.warnings.filter((w) => w.fatal);
  const ran = d.stats !== null && d.stats.spawned > 0;

  return (
    <div className="report-root">
      <div className="report-bar no-print">
        <span className="report-bar-title">Compliance dossier</span>
        <div className="report-bar-actions">
          <button type="button" className="btn" onClick={props.onClose}>
            Back to workspace
          </button>
          <button type="button" className="btn btn-primary" onClick={() => window.print()}>
            Print or save as PDF
          </button>
        </div>
      </div>

      <article className="sheet">
        <header className="sheet-head">
          <div>
            <p className="eyebrow">Crowd flow and egress analysis</p>
            <h1>{d.venueName || 'Untitled venue'}</h1>
          </div>
          <div className={`verdict verdict-${failures.length ? 'fail' : 'pass'}`}>
            {failures.length === 0 ? 'No exceptions' : `${failures.length} exception${failures.length === 1 ? '' : 's'}`}
          </div>
        </header>

        <section className="summary">
          <Metric label="Walkable floor area" value={`${d.walkableArea.toFixed(1)} m²`} />
          <Metric
            label="Occupant load"
            value={limit > 0 ? limit.toLocaleString() : '—'}
            note={`NFPA 101 · ${d.thresholds.occupantLoadFactorM2} m²/p`}
          />
          <Metric
            label="Peak occupancy"
            value={d.peakOccupancy > 0 ? d.peakOccupancy.toLocaleString() : '—'}
            note={ran ? 'simulated' : 'not simulated'}
          />
          <Metric
            label="Clearance time"
            value={d.egressTime !== null ? formatClock(d.egressTime) : '—'}
            note={`target ${formatClock(d.thresholds.targetEgressS)}`}
          />
        </section>

        {d.plan && (
          <figure className="plan">
            <img src={d.plan} alt={`Plan of ${d.venueName}`} />
            <figcaption>
              Figure 1 — Venue plan as analysed. Walls shown solid; doorways in green;
              crowd density where a simulation has been run.
            </figcaption>
          </figure>
        )}

        <section>
          <h2>1 · Basis of assessment</h2>
          <table className="spec">
            <tbody>
              <tr>
                <th>Analysis method</th>
                <td>
                  Agent-based microscopic simulation. Social Force Model with position-based
                  contact resolution, 20 Hz fixed timestep.
                </td>
              </tr>
              <tr>
                <th>Population</th>
                <td>
                  {ran
                    ? `${d.stats!.spawned.toLocaleString()} agents, walking speed sampled from a normal distribution (mean 1.34 m/s, sd 0.26; Weidmann 1993)`
                    : 'No simulation has been run for this venue version.'}
                </td>
              </tr>
              <tr>
                <th>Egress scenario</th>
                <td>
                  Full evacuation from a standing start, every agent routing to its nearest
                  available exit by walkable distance.
                </td>
              </tr>
              <tr>
                <th>Codes applied</th>
                <td>NFPA 101 Life Safety Code; Guide to Safety at Sports Grounds (Green Guide)</td>
              </tr>
              <tr>
                <th>Engine</th>
                <td className="mono">{d.engineVersion}</td>
              </tr>
              <tr>
                <th>Generated</th>
                <td>{d.generatedAt.toISOString().replace('T', ' ').slice(0, 19)} UTC</td>
              </tr>
            </tbody>
          </table>
        </section>

        <section>
          <h2>2 · Findings</h2>
          {findings.length === 0 ? (
            <p className="muted">
              No findings. Run a simulation to assess occupancy, egress time and density.
            </p>
          ) : (
            <table className="findings">
              <thead>
                <tr>
                  <th>Item</th>
                  <th>Requirement</th>
                  <th>Measured</th>
                  <th>Result</th>
                </tr>
              </thead>
              <tbody>
                {findings.map((f) => (
                  <tr key={f.id} className={f.status === 'fail' ? 'is-fail' : undefined}>
                    <td>
                      <span className="finding-clause">{f.clause}</span>
                    </td>
                    <td>{f.requirement}</td>
                    <td>{f.measured}</td>
                    <td className="result">{f.status === 'pass' ? 'Pass' : 'Fail'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>

        {failures.length > 0 && (
          <section>
            <h2>3 · Recommendations</h2>
            <ol className="recommendations">
              {failures.map((f) => (
                <li key={f.id}>
                  <strong>{f.clause}.</strong> {f.remediation}
                </li>
              ))}
            </ol>
          </section>
        )}

        {d.warnings.length > 0 && (
          <section>
            <h2>{failures.length > 0 ? '4' : '3'} · Model diagnostics</h2>
            <p className="muted">
              Issues raised by the geometry compiler. Fatal entries prevent simulation and
              invalidate any result above.
            </p>
            <ul className="diagnostics">
              {fatal.map((w, i) => (
                <li key={`f${i}`} className="is-fatal">
                  <span className="diag-code">{w.code}</span> {w.message}
                </li>
              ))}
              {d.warnings
                .filter((w) => !w.fatal)
                .map((w, i) => (
                  <li key={`a${i}`}>
                    <span className="diag-code">{w.code}</span> {w.message}
                  </li>
                ))}
            </ul>
          </section>
        )}

        <footer className="statement">
          <h3>Verification statement</h3>
          <p>
            This analysis was produced by CrowdFlow Studio ({d.engineVersion}), an agent-based
            crowd-flow and evacuation model. Results are decision support for a competent person
            and are <strong>not a substitute</strong> for assessment by a qualified fire safety
            engineer or the approval of the authority having jurisdiction.
          </p>
          <p>
            Figures are derived from a single simulation run. A submission should quote a
            distribution across repeated runs rather than a single value. Model assumptions and
            input parameters are listed in section 1.
          </p>
        </footer>
      </article>
    </div>
  );
}

function Metric({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="metric">
      <span className="metric-label">{label}</span>
      <span className="metric-value">{value}</span>
      {note && <span className="metric-note">{note}</span>}
    </div>
  );
}
