/**
 * The life-safety arithmetic.
 *
 * The status bar and the compliance dossier both read these, so they are the
 * one place where a wrong number is not a UI glitch but a wrong answer on a
 * document someone files. Small enough to test exhaustively, consequential
 * enough to be worth it.
 */

import { describe, expect, it } from 'vitest';
import { formatClock, occupantLoad, safetyVerdict } from './store';

describe('occupantLoad', () => {
  it('is NFPA 101 floor area over the load factor, rounded down', () => {
    // 240 m² of concentrated assembly at 0.65 m²/person.
    expect(occupantLoad(240, 0.65)).toBe(369);
  });

  it('rounds down, never up', () => {
    // 0.9 of a person is not a person, and rounding up would licence a venue
    // for someone it has no room for.
    expect(occupantLoad(10, 3)).toBe(3);
    expect(occupantLoad(1, 0.65)).toBe(1);
    expect(occupantLoad(0.3, 0.65)).toBe(0);
  });

  it('is zero for no floor', () => {
    expect(occupantLoad(0, 0.65)).toBe(0);
  });
});

describe('safetyVerdict', () => {
  const base = {
    occupancy: 100,
    occupantLimit: 1000,
    egressS: 100,
    targetEgressS: 480,
    fatalWarnings: 0,
  };

  it('is normal when everything is inside its limit', () => {
    expect(safetyVerdict(base)).toBe('normal');
  });

  it('the worst condition wins', () => {
    // One violation makes the whole venue non-compliant. A panel that averaged
    // its inputs would report a venue with one fatal fault as mostly fine.
    expect(safetyVerdict({ ...base, fatalWarnings: 1 })).toBe('alarm');
    expect(safetyVerdict({ ...base, occupancy: 1001 })).toBe('alarm');
    expect(safetyVerdict({ ...base, egressS: 481 })).toBe('alarm');
  });

  it('warns before the limit, not at it', () => {
    // 90% is where a supervisor should be told, because by 100% it is too late
    // to do anything about it.
    expect(safetyVerdict({ ...base, occupancy: 901 })).toBe('supervise');
    expect(safetyVerdict({ ...base, egressS: 433 })).toBe('supervise');
    expect(safetyVerdict({ ...base, occupancy: 899 })).toBe('normal');
  });

  it('does not judge egress that has not been measured', () => {
    // No run yet is not the same as a run that passed.
    expect(safetyVerdict({ ...base, egressS: null })).toBe('normal');
    expect(safetyVerdict({ ...base, egressS: null, occupancy: 1001 })).toBe('alarm');
  });

  it('does not judge occupancy against an unknown limit', () => {
    // A venue whose area failed to compile has a limit of 0. Treating that as
    // "limit exceeded" would put every unbuilt venue into alarm.
    expect(safetyVerdict({ ...base, occupantLimit: 0, occupancy: 5000 })).toBe('normal');
  });
});

describe('formatClock', () => {
  it('reads as minutes and seconds', () => {
    expect(formatClock(0)).toBe('0:00');
    expect(formatClock(9)).toBe('0:09');
    expect(formatClock(60)).toBe('1:00');
    expect(formatClock(480)).toBe('8:00');
    expect(formatClock(3661)).toBe('61:01');
  });

  it('floors rather than rounds, and never shows a negative', () => {
    expect(formatClock(59.9)).toBe('0:59');
    expect(formatClock(-5)).toBe('0:00');
  });
});
