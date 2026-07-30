/**
 * The workspace shell.
 *
 * Owns the engine lifecycle and the playback clock. The canvas is driven
 * imperatively through a ref — React never re-renders per tick, it only re-
 * renders when something a human reads has changed (roughly 4 Hz, throttled).
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { Renderer } from '../canvas/Renderer';
import { engineVersion, loadEngine, Run, Venue } from '../engine/bridge';
import { useApp } from '../state/store';
import { Inspector } from './Inspector';
import { StatusBar } from './StatusBar';
import { Timeline } from './Timeline';
import { Validation } from './Validation';
import fixtureJson from '../../../fixtures/unit/hall-two-doors.venue.json?raw';

/** Physics runs at 20 Hz; this is the tick length in milliseconds. */
const TICK_MS = 50;

export function App() {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const rendererRef = useRef<Renderer | null>(null);
  const venueRef = useRef<Venue | null>(null);
  const runRef = useRef<Run | null>(null);
  const rafRef = useRef<number>(0);
  const accRef = useRef<number>(0);
  const lastRef = useRef<number>(0);
  const uiClockRef = useRef<number>(0);

  const [venueTitle, setVenueTitle] = useState('Hall with two doors');
  // A ref change does not re-render, so whether a run exists must live in
  // state — otherwise Play stays disabled after agents are placed.
  const [hasRun, setHasRun] = useState(false);
  const [placedAgents, setPlacedAgents] = useState(0);

  const {
    phase,
    error,
    running,
    speed,
    requestedAgents,
    setPhase,
    setEngineVersion,
    setVenue,
    setRunning,
    setStats,
    setEgressTime,
    resetPeak,
    showHeatmap,
    heatmapPeak,
    setDensityFindings,
  } = useApp();

  /** Compile a venue document and hand its geometry to the canvas. */
  const compile = useCallback(
    (json: string, name: string) => {
      venueRef.current?.free();
      runRef.current?.free();
      runRef.current = null;
      setHasRun(false);

      const venue = Venue.compile(json);
      venueRef.current = venue;
      setVenueTitle(name);
      setVenue({
        name,
        walkableArea: venue.walkableArea,
        warnings: venue.warnings,
        simulable: venue.simulable,
      });
      setStats(null);
      setEgressTime(null);
      resetPeak();
      setRunning(false);
      rendererRef.current?.setVenue(venue.geometry);
      rendererRef.current?.clearAgents();
      rendererRef.current?.setDensity(null);
    },
    [setVenue, setStats, setEgressTime, setRunning, resetPeak],
  );

  // Boot: load wasm, mount the canvas, compile the starting venue.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        await loadEngine();
        if (cancelled || !hostRef.current) return;

        const r = new Renderer();
        await r.mount(hostRef.current);
        rendererRef.current = r;

        setEngineVersion(engineVersion());
        compile(fixtureJson, 'Hall with two doors');
        setPhase('ready');
      } catch (e) {
        setPhase('error', e instanceof Error ? e.message : String(e));
      }
    })();

    return () => {
      cancelled = true;
      cancelAnimationFrame(rafRef.current);
      rendererRef.current?.destroy();
      runRef.current?.free();
      venueRef.current?.free();
    };
    // Boot once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** Create a run and place agents. */
  const reset = useCallback(() => {
    const venue = venueRef.current;
    if (!venue || !venue.simulable) return;

    runRef.current?.free();
    const run = venue.simulate(Date.now() % 1_000_000);
    // Placement rejects candidates that would overlap an existing body, so
    // fewer agents may be placed than requested. Report what was actually
    // placed rather than what was asked for.
    setPlacedAgents(run.spawn(requestedAgents));
    runRef.current = run;
    setHasRun(true);

    resetPeak();
    setStats(run.stats());
    setEgressTime(null);
    rendererRef.current?.setAgents(run.positions(), run.states());
    rendererRef.current?.setDensity(showHeatmap ? run.density(heatmapPeak) : null);
  }, [requestedAgents, showHeatmap, heatmapPeak, setStats, setEgressTime, resetPeak]);

  // The playback clock. Fixed timestep with an accumulator: the engine always
  // advances by exactly 50 ms, whatever the frame rate, so a slow frame changes
  // how much is simulated per second — never the physics itself.
  useEffect(() => {
    const frame = (now: number) => {
      rafRef.current = requestAnimationFrame(frame);
      const run = runRef.current;
      const renderer = rendererRef.current;
      if (!run || !renderer) return;

      const dtMs = Math.min(250, now - (lastRef.current || now));
      lastRef.current = now;

      if (running) {
        accRef.current += dtMs * speed;
        // Cap the catch-up so a background tab does not stall on resume.
        let budget = 40;
        while (accRef.current >= TICK_MS && budget-- > 0) {
          run.step();
          accRef.current -= TICK_MS;
        }
        if (budget <= 0) accRef.current = 0;

        if (run.active === 0 && useApp.getState().egressTime === null) {
          setEgressTime(run.time);
          setRunning(false);
        }
      }

      renderer.setAgents(run.positions(), run.states());

      // Chrome and the heatmap update at ~4 Hz. The density field changes far
      // slower than agents move, and neither is read at 60 Hz by a human.
      if (now - uiClockRef.current > 250) {
        uiClockRef.current = now;
        setStats(run.stats());
        renderer.setDensity(showHeatmap ? run.density(heatmapPeak) : null);
        setDensityFindings(run.peakDensity, run.criticalArea);
      }
    };

    rafRef.current = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(rafRef.current);
  }, [running, speed, setStats, setEgressTime, setRunning, showHeatmap, heatmapPeak, setDensityFindings]);

  const onLoadFile = useCallback(
    async (file: File) => {
      try {
        compile(await file.text(), file.name.replace(/\.venue\.json$/, ''));
      } catch (e) {
        setPhase('error', e instanceof Error ? e.message : String(e));
      }
    },
    [compile, setPhase],
  );

  if (phase === 'error') {
    return (
      <div className="boot boot-error" role="alert">
        <h1>Engine failed to start</h1>
        <p>{error}</p>
        <p className="boot-hint">
          Build the engine with <code>pnpm engine</code>, then reload.
        </p>
      </div>
    );
  }

  return (
    <div className="shell">
      <StatusBar />

      <main className="body">
        <nav className="rail" aria-label="Tools">
          <RailButton label="Select" glyph="⌖" active />
          <RailButton label="Wall" glyph="│" disabled />
          <RailButton label="Zone" glyph="▢" disabled />
          <RailButton label="Door" glyph="◠" disabled />
          <div className="rail-spacer" />
          <RailButton label="Fit view" glyph="⤢" onClick={() => rendererRef.current?.fit()} />
        </nav>

        <div className="canvas-host" ref={hostRef}>
          {phase === 'loading' && <div className="boot">Starting engine…</div>}
        </div>

        <aside className="inspector" aria-label="Inspector">
          <Inspector
            venueTitle={venueTitle}
            placedAgents={placedAgents}
            onLoadFile={onLoadFile}
            onReset={reset}
          />
          <Validation />
        </aside>
      </main>

      <Timeline onReset={reset} hasRun={hasRun} />
    </div>
  );
}

function RailButton({
  label,
  glyph,
  active,
  disabled,
  onClick,
}: {
  label: string;
  glyph: string;
  active?: boolean;
  disabled?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      className={`rail-btn${active ? ' is-active' : ''}`}
      title={disabled ? `${label} — not yet available` : label}
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
    >
      <span aria-hidden="true">{glyph}</span>
    </button>
  );
}
