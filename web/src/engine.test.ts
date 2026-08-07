/**
 * Summarising repeated runs.
 *
 * The dossier quotes these, so they are compliance arithmetic rather than
 * presentation. Pure, and testable without the engine — the wasm side is
 * covered in `engine/cf-wasm/tests/end_to_end.rs`.
 */

import { describe, expect, it } from 'vitest';
import { summariseEgress } from './engine';

describe('summariseEgress', () => {
  it('reports the spread across runs', () => {
    const s = summariseEgress([100, 110, 120, 130, 140])!;
    expect(s.n).toBe(5);
    expect(s.unfinished).toBe(0);
    expect(s.meanS).toBe(120);
    expect(s.minS).toBe(100);
    expect(s.maxS).toBe(140);
    // Sample standard deviation, n−1: these are runs drawn from the population
    // of possible runs, not the whole of it.
    expect(s.sdS).toBeCloseTo(15.811, 3);
  });

  it('counts unfinished runs rather than dropping them', () => {
    // "Eight of ten cleared" is a materially different statement from a mean
    // over eight, and only one of them is honest.
    const s = summariseEgress([100, 120, Number.NaN, Number.NaN])!;
    expect(s.n).toBe(2);
    expect(s.unfinished).toBe(2);
    expect(s.meanS).toBe(110);
  });

  it('is null when nothing finished', () => {
    // No mean exists. Returning zero would read as an instant evacuation.
    expect(summariseEgress([Number.NaN, Number.NaN])).toBeNull();
    expect(summariseEgress([])).toBeNull();
  });

  it('takes the 95th percentile by nearest rank', () => {
    // Ten runs: the slowest observed, not an interpolation between samples
    // that were never seen.
    const ten = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
    expect(summariseEgress(ten)!.p95S).toBe(100);

    // Twenty runs: the 19th, so one run may legitimately be slower.
    const twenty = Array.from({ length: 20 }, (_, i) => (i + 1) * 10);
    expect(summariseEgress(twenty)!.p95S).toBe(190);
  });

  it('does not need the input sorted', () => {
    const s = summariseEgress([140, 100, 130, 110, 120])!;
    expect(s.minS).toBe(100);
    expect(s.maxS).toBe(140);
    expect(s.p95S).toBe(140);
  });

  it('a single run has no spread', () => {
    const s = summariseEgress([120])!;
    expect(s.n).toBe(1);
    expect(s.sdS).toBe(0);
    expect(s.meanS).toBe(120);
    expect(s.p95S).toBe(120);
  });
});
