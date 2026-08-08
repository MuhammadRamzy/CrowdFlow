/**
 * The compliance dossier, rendered.
 *
 * This is the artefact that leaves the building — the thing a reviewer reads
 * and an authority may be shown. So the checks here are about what it *says*:
 * that a failing venue says so, that the arithmetic behind a finding is on the
 * page, and that the caveats it is obliged to carry are actually carried.
 *
 * How it prints is still a person's job.
 */

import { cleanup, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Report, type ReportData } from './Report';

function data(over: Partial<ReportData> = {}): ReportData {
  return {
    egressStats: null,
    egressCurve: null,
    exitUsage: null,
    compliance: null,
    venueName: 'Test hall',
    document: null,
    walkableArea: 240,
    warnings: [],
    stats: {
      tick: 2000,
      time: 100,
      active: 0,
      exited: 300,
      spawned: 300,
      blocked: 0,
      maxOverlap: 0,
      escaped: 0,
    },
    egressTime: 120,
    peakOccupancy: 300,
    peakDensity: 1.4,
    criticalArea: 0,
    thresholds: { occupantLoadFactorM2: 0.65, targetEgressS: 480, criticalDensity: 6 },
    engineVersion: '0.1.0',
    plan: null,
    heatmap: null,
    generatedAt: new Date('2026-08-07T12:00:00Z'),
    ...over,
  };
}

afterEach(cleanup);

describe('Report', () => {
  it('renders a passing venue without throwing', () => {
    render(<Report data={data()} onClose={vi.fn()} />);
    expect(screen.getByText('Test hall')).toBeTruthy();
  });

  it('names the engine version, so a figure can be traced to what produced it', () => {
    render(<Report data={data()} onClose={vi.fn()} />);
    // Twice by design: once in the parameters table and once in the statement.
    expect(screen.getAllByText(/0\.1\.0/).length).toBeGreaterThanOrEqual(2);
  });

  it('states the validated envelope and where this run fell', () => {
    render(<Report data={data({ peakDensity: 1.4 })} onClose={vi.fn()} />);
    expect(screen.getByText(/Validated envelope: crowd density up to 2/i)).toBeTruthy();
    expect(screen.getByText(/within the validated envelope/i)).toBeTruthy();
  });

  it('says plainly when a run went outside the envelope', () => {
    // Above 2 persons/m² the model is measurably slow (ADR 0007). A reader
    // must not have to know that to read the number correctly.
    render(<Report data={data({ peakDensity: 3.1, criticalArea: 4.2 })} onClose={vi.fn()} />);
    expect(screen.getByText(/outside the validated envelope/i)).toBeTruthy();
    expect(screen.getByText(/upper bound rather than an estimate/i)).toBeTruthy();
  });

  it('asks for a distribution when it only has one run', () => {
    render(<Report data={data({ egressStats: null })} onClose={vi.fn()} />);
    expect(screen.getByText(/derived from a single simulation run/i)).toBeTruthy();
  });

  it('quotes the 95th percentile when it has a distribution', () => {
    render(
      <Report
        data={data({
          egressStats: {
            n: 10,
            unfinished: 0,
            meanS: 120,
            sdS: 8,
            minS: 108,
            maxS: 136,
            p95S: 136,
          },
        })}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText(/egress time across repeated runs/i)).toBeTruthy();
    // In the table's row header and in the prose above it.
    expect(screen.getAllByText(/95th percentile/i).length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText(/10 runs/i)).toBeTruthy();
    // The single-run caveat must be gone — it would contradict the section.
    expect(screen.queryByText(/derived from a single simulation run/i)).toBeNull();
  });

  it('does not hide runs that never cleared', () => {
    render(
      <Report
        data={data({
          egressStats: {
            n: 8,
            unfinished: 2,
            meanS: 130,
            sdS: 9,
            minS: 118,
            maxS: 150,
            p95S: 150,
          },
        })}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText(/2 did not clear/i)).toBeTruthy();
    expect(screen.getByText(/only in the cases where it emptied at all/i)).toBeTruthy();
  });

  it('reports a fatal compile warning rather than presenting results over it', () => {
    render(
      <Report
        data={data({
          warnings: [
            { code: 'noWalkableArea', fatal: true, message: 'floor has no walkable area' },
          ],
        })}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText(/floor has no walkable area/i)).toBeTruthy();
  });

  it('closes when asked', () => {
    const onClose = vi.fn();
    render(<Report data={data()} onClose={onClose} />);
    const close = screen.getByRole('button', { name: /back to workspace/i });
    close.click();
    expect(onClose).toHaveBeenCalled();
  });
});

describe('egress shape and exit usage', () => {
  it('shows how the venue cleared, not just when it finished', () => {
    render(
      <Report
        data={data({ egressCurve: [17.9, 31.8, 36.2, 37.2] })}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText(/how the venue cleared/i)).toBeTruthy();
    expect(screen.getByText(/90% cleared/i)).toBeTruthy();
    // It is the tail that describes the risk, so the reasoning is on the page
    // rather than left for the reader to supply.
    expect(screen.getByText(/tail that describes the risk/i)).toBeTruthy();
  });

  it('does not invent a time for a percentile nobody reached', () => {
    render(<Report data={data({ egressCurve: [12.0, null, null, null] })} onClose={vi.fn()} />);
    // An em dash, not a zero — zero reads as an instantaneous evacuation.
    expect(screen.getAllByText('—').length).toBeGreaterThan(0);
  });

  it('attributes the crowd to the doors that carried it', () => {
    render(
      <Report
        data={data({
          exitUsage: [
            { count: 47, specificFlow: 12.4 },
            { count: 33, specificFlow: 8.7 },
          ],
        })}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText(/which exits carried the crowd/i)).toBeTruthy();
    expect(screen.getByText('47')).toBeTruthy();
    // 47 of 80 is 59%.
    expect(screen.getByText('59%')).toBeTruthy();
  });

  it('flags an exit that carried nobody without calling it faulty', () => {
    render(
      <Report
        data={data({
          exitUsage: [
            { count: 80, specificFlow: 20.0 },
            { count: 0, specificFlow: 0 },
          ],
        })}
        onClose={vi.fn()}
      />,
    );
    // An unused exit may simply be far from the crowd. Saying so is the
    // difference between a finding and an accusation.
    expect(screen.getByText(/not necessarily faulty/i)).toBeTruthy();
  });
})

describe('rule packs in the dossier', () => {
  const pack = (over = {}) => ({
    id: 'nfpa101',
    name: 'NFPA 101 — Life Safety Code',
    source: 'NFPA 101 (2024), assembly occupancy',
    reviewed: false,
    findings: [
      {
        ruleId: 'nfpa101.occupant-load',
        clause: '7.3.1.2',
        title: 'Occupant load must not exceed what the floor area allows',
        status: 'pass' as const,
        measured: 300,
        limit: 369,
        working: '240.00 m² ÷ 0.65 m²/person, rounded down = 369',
        note: 'Concentrated assembly use.',
      },
    ],
    ...over,
  });

  it('cites the standard and the clause', () => {
    render(<Report data={data({ compliance: [pack()] })} onClose={vi.fn()} />);
    expect(screen.getByText(/NFPA 101 — Life Safety Code/)).toBeTruthy();
    expect(screen.getByText('7.3.1.2')).toBeTruthy();
  });

  it('shows the arithmetic behind every verdict', () => {
    // A compliance figure a reader cannot reproduce is one they must take on
    // trust, and these documents do not get taken on trust.
    render(<Report data={data({ compliance: [pack()] })} onClose={vi.fn()} />);
    expect(screen.getByText(/240.00 m² ÷ 0.65 m²\/person/)).toBeTruthy();
  });

  it('says plainly that the rules are unreviewed', () => {
    render(<Report data={data({ compliance: [pack()] })} onClose={vi.fn()} />);
    expect(
      screen.getByText(/not been reviewed by a qualified fire engineer/i),
    ).toBeTruthy();
  });

  it('drops the warning once a pack has been reviewed', () => {
    render(<Report data={data({ compliance: [pack({ reviewed: true })] })} onClose={vi.fn()} />);
    expect(screen.queryByText(/not been reviewed by a qualified/i)).toBeNull();
  });

  it('shows an unassessed rule as n/a, never as a pass', () => {
    // Reporting an unchecked rule as compliant turns "we did not check" into
    // "we checked and it was fine", on a document an authority may act on.
    const p = pack({
      findings: [
        {
          ruleId: 'greenGuide.rate-of-passage',
          clause: '9.11',
          title: 'Achieved evacuation time',
          status: 'notAssessed' as const,
          measured: null,
          limit: null,
          working: 'not assessed — egress time is not known for this venue',
          note: '',
        },
      ],
    });
    render(<Report data={data({ compliance: [p] })} onClose={vi.fn()} />);
    // Scoped to the pack's own table — the dossier's other findings say "pass"
    // legitimately, and a bare query would match those.
    const section = screen.getByText(/NFPA 101 — Life Safety Code/).closest('section')!;
    expect(within(section).getByText('n/a')).toBeTruthy();
    expect(within(section).queryByText(/^pass$/i)).toBeNull();
  });
})
