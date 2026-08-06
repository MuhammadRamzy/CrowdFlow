/**
 * The arrival profile, plotted and editable.
 *
 * # What it shows
 *
 * The **cumulative** fraction of a population that has entered by time t —
 * not a rate. Cumulative because that is what the schema stores and what the
 * engine inverts to decide when agent 600 of 1000 walks in, and because a
 * planner states arrivals cumulatively: "sixty per cent are in by half seven".
 *
 * Every arrival kind is drawn, not just the editable one. An instantaneous
 * arrival is a step at t=0 and a uniform arrival is a straight ramp; showing
 * them in the same axes makes the three options comparable at a glance instead
 * of being three unrelated words in a segmented control. Only `curve` responds
 * to dragging, and the plot says so.
 *
 * # Design
 *
 * One series, so no legend — the panel heading names it. Grid and axes are
 * recessive `--line`; the curve is the only saturated thing in the frame,
 * because in this product a coloured pixel means something. Handles carry a
 * transparent hit target far larger than the mark they draw, and take arrow
 * keys, so the curve is editable without a mouse.
 */

import { useRef, useState } from 'react';
import type { Arrival } from '../schema/scenario';
import { formatClock } from '../state/store';

const W = 248;
const H = 96;
const PAD = { l: 26, r: 10, t: 10, b: 18 };
const PLOT_W = W - PAD.l - PAD.r;
const PLOT_H = H - PAD.t - PAD.b;

/** The profile an arrival actually produces, as `(t, cumulative fraction)`. */
export function profileOf(arrival: Arrival, durationS: number): [number, number][] {
  const d = Math.max(1, durationS);
  switch (arrival.kind) {
    case 'preplaced':
      // Everyone is on the floor before the first tick: a vertical step.
      return [
        [0, 0],
        [0, 1],
        [d, 1],
      ];
    case 'uniform':
      return [
        [0, 0],
        [d, 1],
      ];
    case 'curve':
      return arrival.points;
  }
}

interface Props {
  points: [number, number][];
  durationS: number;
  /** Absent for the non-editable kinds; the plot then draws without handles. */
  onChange?: (points: [number, number][]) => void;
}

export function ArrivalPlot({ points, durationS, onChange }: Props) {
  const svgRef = useRef<SVGSVGElement | null>(null);
  const [active, setActive] = useState<number | null>(null);
  const d = Math.max(1, durationS);

  const x = (t: number) => PAD.l + (t / d) * PLOT_W;
  const y = (f: number) => PAD.t + (1 - f) * PLOT_H;

  const path = points.map(([t, f], i) => `${i === 0 ? 'M' : 'L'}${x(t)} ${y(f)}`).join(' ');
  const area = `${path} L${x(points.at(-1)?.[0] ?? d)} ${y(0)} L${x(points[0]?.[0] ?? 0)} ${y(0)} Z`;

  /** Pointer position as `(t seconds, fraction)`, clamped to the axes. */
  const toData = (e: React.PointerEvent): [number, number] | null => {
    const svg = svgRef.current;
    if (!svg) return null;
    const r = svg.getBoundingClientRect();
    const px = ((e.clientX - r.left) / r.width) * W;
    const py = ((e.clientY - r.top) / r.height) * H;
    const t = ((px - PAD.l) / PLOT_W) * d;
    const f = 1 - (py - PAD.t) / PLOT_H;
    return [Math.min(d, Math.max(0, t)), Math.min(1, Math.max(0, f))];
  };

  const move = (i: number, next: [number, number]) => {
    if (!onChange) return;
    const copy = points.map((p): [number, number] => [p[0], p[1]]);
    copy[i] = next;
    onChange(copy);
  };

  const dragging = active !== null && onChange !== undefined;

  const caption =
    active !== null && points[active]
      ? `${formatClock(points[active]![0])} · ${Math.round(points[active]![1] * 100)}%`
      : onChange
        ? 'Drag a point · click the plot to add · double-click a point to remove'
        : null;

  return (
    <div className="plot">
      <svg
        ref={svgRef}
        className="plot-svg"
        viewBox={`0 0 ${W} ${H}`}
        role="img"
        aria-label="Cumulative arrival profile"
        onPointerMove={(e) => {
          if (!dragging) return;
          const p = toData(e);
          if (p) move(active, p);
        }}
        onPointerUp={() => setActive(null)}
        onPointerLeave={() => setActive(null)}
        onClick={(e) => {
          // A click on empty plot adds a point. Guarded on the target so
          // releasing a drag over the canvas does not add one as well.
          if (!onChange || e.target !== e.currentTarget) return;
          const p = toData(e as unknown as React.PointerEvent);
          if (!p) return;
          onChange([...points.map((q): [number, number] => [q[0], q[1]]), p]);
        }}
      >
        {/* Grid: quarters on both axes, recessive. */}
        {[0, 0.25, 0.5, 0.75, 1].map((f) => (
          <line key={`h${f}`} className="plot-grid" x1={PAD.l} y1={y(f)} x2={W - PAD.r} y2={y(f)} />
        ))}
        {[0.25, 0.5, 0.75].map((f) => (
          <line
            key={`v${f}`}
            className="plot-grid"
            x1={x(f * d)}
            y1={PAD.t}
            x2={x(f * d)}
            y2={PAD.t + PLOT_H}
          />
        ))}

        <path className="plot-area" d={area} />
        <path className="plot-line" d={path} />

        {onChange &&
          points.map(([t, f], i) => (
            <g key={i}>
              {/* The visible mark and, beneath the pointer, a target four times
                  its size — a 4 px dot is not a control. */}
              <circle
                className="plot-hit"
                cx={x(t)}
                cy={y(f)}
                r={9}
                onPointerDown={(e) => {
                  e.currentTarget.setPointerCapture(e.pointerId);
                  setActive(i);
                }}
                onPointerUp={(e) => {
                  e.currentTarget.releasePointerCapture(e.pointerId);
                  setActive(null);
                }}
                onDoubleClick={() => {
                  if (points.length <= 2) return;
                  onChange(points.filter((_, j) => j !== i).map((q): [number, number] => [q[0], q[1]]));
                }}
              />
              <circle
                className={`plot-node${active === i ? ' is-active' : ''}`}
                cx={x(t)}
                cy={y(f)}
                r={3.5}
                tabIndex={0}
                role="slider"
                aria-label={`Arrival point ${i + 1}`}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round(f * 100)}
                aria-valuetext={`${formatClock(t)}, ${Math.round(f * 100)} per cent arrived`}
                onFocus={() => setActive(i)}
                onBlur={() => setActive(null)}
                onKeyDown={(e) => {
                  const coarse = e.shiftKey ? 5 : 1;
                  const dt = (d / 100) * coarse;
                  const df = 0.01 * coarse;
                  const map: Record<string, [number, number]> = {
                    ArrowLeft: [-dt, 0],
                    ArrowRight: [dt, 0],
                    ArrowUp: [0, df],
                    ArrowDown: [0, -df],
                  };
                  const delta = map[e.key];
                  if (!delta) return;
                  e.preventDefault();
                  move(i, [
                    Math.min(d, Math.max(0, t + delta[0])),
                    Math.min(1, Math.max(0, f + delta[1])),
                  ]);
                }}
              />
            </g>
          ))}

        <text className="plot-tick" x={PAD.l - 4} y={y(1) + 3} textAnchor="end">
          100%
        </text>
        <text className="plot-tick" x={PAD.l - 4} y={y(0) + 3} textAnchor="end">
          0
        </text>
        <text className="plot-tick" x={PAD.l} y={H - 5}>
          0
        </text>
        <text className="plot-tick" x={W - PAD.r} y={H - 5} textAnchor="end">
          {formatClock(d)}
        </text>
      </svg>
      {caption && <p className="plot-caption">{caption}</p>}
    </div>
  );
}
