/**
 * Application state.
 *
 * The engine objects (`Venue`, `Run`) are deliberately *not* in the store. They
 * own wasm memory and change every tick; putting them behind React state would
 * either re-render at 20 Hz or lie about what is current. The store holds the
 * things that drive chrome — what venue is loaded, whether it is playing, the
 * latest counters — and the canvas reads the engine directly.
 */

import { create } from 'zustand';
import type { CompileWarning, SimStats } from '../engine/bridge';

export type Phase = 'loading' | 'ready' | 'error';

/** Life-safety verdict. Modelled on an annunciator panel; see docs/08. */
export type SafetyState = 'normal' | 'supervise' | 'alarm';

export interface Thresholds {
  /** NFPA 101 occupant load factor, m² per person, for concentrated assembly. */
  occupantLoadFactorM2: number;
  /** Target egress time in seconds. Green Guide's 8-minute benchmark. */
  targetEgressS: number;
  /** Crowd-science critical density, persons/m². */
  criticalDensity: number;
}

export interface AppState {
  phase: Phase;
  error: string | null;
  engineVersion: string;

  venueName: string;
  walkableArea: number;
  warnings: CompileWarning[];
  simulable: boolean;

  running: boolean;
  /** Simulated seconds per wall-clock second. */
  speed: number;
  requestedAgents: number;
  stats: SimStats | null;
  /** Seconds at which the last agent left, once a run completes. */
  egressTime: number | null;
  /**
   * Highest occupancy reached during the current run.
   *
   * The verdict is judged against this, not against the instantaneous count.
   * A real annunciator latches: an overcrowding event that has since cleared is
   * still an overcrowding event, and a panel that forgot it the moment the room
   * emptied would report a venue as compliant precisely because the failure
   * resolved itself.
   */
  peakOccupancy: number;
  /** Whether the density overlay is drawn. */
  showHeatmap: boolean;
  /** `peak` shows the worst each cell reached; otherwise the current tick. */
  heatmapPeak: boolean;
  /** Highest density reached anywhere, persons/m². */
  peakDensity: number;
  /** Floor area, m², that reached the crush threshold at any point. */
  criticalArea: number;

  thresholds: Thresholds;

  setPhase: (p: Phase, error?: string) => void;
  setEngineVersion: (v: string) => void;
  setVenue: (v: { name: string; walkableArea: number; warnings: CompileWarning[]; simulable: boolean }) => void;
  setRunning: (r: boolean) => void;
  setSpeed: (s: number) => void;
  setRequestedAgents: (n: number) => void;
  setStats: (s: SimStats | null) => void;
  resetPeak: () => void;
  setShowHeatmap: (v: boolean) => void;
  setHeatmapPeak: (v: boolean) => void;
  setDensityFindings: (peakDensity: number, criticalArea: number) => void;
  setEgressTime: (t: number | null) => void;
}

export const useApp = create<AppState>((set) => ({
  phase: 'loading',
  error: null,
  engineVersion: '',

  venueName: '',
  walkableArea: 0,
  warnings: [],
  simulable: false,

  running: false,
  speed: 1,
  requestedAgents: 500,
  stats: null,
  egressTime: null,
  peakOccupancy: 0,
  showHeatmap: true,
  heatmapPeak: false,
  peakDensity: 0,
  criticalArea: 0,

  thresholds: {
    // NFPA 101 Table 7.3.1.2, concentrated assembly without fixed seating.
    occupantLoadFactorM2: 0.65,
    targetEgressS: 480,
    criticalDensity: 6,
  },

  setPhase: (phase, error) => set({ phase, error: error ?? null }),
  setEngineVersion: (engineVersion) => set({ engineVersion }),
  setVenue: (v) =>
    set({
      venueName: v.name,
      walkableArea: v.walkableArea,
      warnings: v.warnings,
      simulable: v.simulable,
    }),
  setRunning: (running) => set({ running }),
  setSpeed: (speed) => set({ speed }),
  setRequestedAgents: (requestedAgents) => set({ requestedAgents }),
  setStats: (stats) =>
    set((prev) => ({
      stats,
      peakOccupancy: stats ? Math.max(prev.peakOccupancy, stats.active) : prev.peakOccupancy,
    })),
  resetPeak: () => set({ peakOccupancy: 0, peakDensity: 0, criticalArea: 0 }),
  setShowHeatmap: (showHeatmap) => set({ showHeatmap }),
  setHeatmapPeak: (heatmapPeak) => set({ heatmapPeak }),
  setDensityFindings: (peakDensity, criticalArea) => set({ peakDensity, criticalArea }),
  setEgressTime: (egressTime) => set({ egressTime }),
}));

/** NFPA 101 occupant load for a floor area. */
export function occupantLoad(areaM2: number, olf: number): number {
  return Math.floor(areaM2 / olf);
}

/**
 * The overall life-safety verdict.
 *
 * Ordered so the worst condition wins: one violation makes the whole venue
 * non-compliant, and a panel that averaged its inputs would hide that.
 */
export function safetyVerdict(args: {
  occupancy: number;
  occupantLimit: number;
  egressS: number | null;
  targetEgressS: number;
  fatalWarnings: number;
}): SafetyState {
  const { occupancy, occupantLimit, egressS, targetEgressS, fatalWarnings } = args;
  if (fatalWarnings > 0) return 'alarm';
  if (occupantLimit > 0 && occupancy > occupantLimit) return 'alarm';
  if (egressS !== null && egressS > targetEgressS) return 'alarm';

  if (occupantLimit > 0 && occupancy > occupantLimit * 0.9) return 'supervise';
  if (egressS !== null && egressS > targetEgressS * 0.9) return 'supervise';
  return 'normal';
}

/** `mm:ss`, for egress times. */
export function formatClock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const m = Math.floor(s / 60);
  return `${m}:${String(s % 60).padStart(2, '0')}`;
}
