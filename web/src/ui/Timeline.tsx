/**
 * Transport controls.
 *
 * The clock reads simulated time, not wall time — at 8x a run advances eight
 * seconds per second, and the number that matters for egress is the simulated
 * one.
 */

import { formatClock, useApp } from '../state/store';
import { IconPause, IconPlay, IconReset } from './Icon';

const SPEEDS = [0.5, 1, 2, 4, 8, 16];

export function Timeline({ onReset, hasRun }: { onReset: () => void; hasRun: boolean }) {
  const { running, setRunning, speed, setSpeed, stats, simulable, egressTime } = useApp();
  const canPlay = simulable && hasRun && (stats?.active ?? 0) > 0;

  return (
    <footer className="timeline">
      <div className="transport">
        <button
          type="button"
          className="btn btn-icon"
          onClick={() => setRunning(!running)}
          disabled={!canPlay}
          aria-label={running ? 'Pause' : 'Play'}
          title={running ? 'Pause' : 'Play'}
        >
          {running ? <IconPause size={14} /> : <IconPlay size={14} />}
        </button>
        <button
          type="button"
          className="btn btn-icon"
          onClick={onReset}
          disabled={!simulable}
          aria-label="Reset"
          title="Reset"
        >
          <IconReset size={14} />
        </button>
      </div>

      <div className="clock">
        <span className="clock-time">{formatClock(stats?.time ?? 0)}</span>
        <span className="clock-unit">simulated</span>
      </div>

      <div className="speeds" role="group" aria-label="Playback speed">
        {SPEEDS.map((s) => (
          <button
            key={s}
            type="button"
            className={`speed${speed === s ? ' is-active' : ''}`}
            onClick={() => setSpeed(s)}
            aria-pressed={speed === s}
          >
            {s}×
          </button>
        ))}
      </div>

      <div className="counters">
        {stats ? (
          <>
            <span className="counter">
              <b>{stats.active.toLocaleString()}</b> in venue
            </span>
            <span className="counter">
              <b>{stats.exited.toLocaleString()}</b> cleared
            </span>
            {egressTime !== null && (
              <span className="counter is-done">
                cleared in <b>{formatClock(egressTime)}</b>
              </span>
            )}
          </>
        ) : (
          <span className="counter is-muted">Place agents to begin</span>
        )}
      </div>
    </footer>
  );
}
