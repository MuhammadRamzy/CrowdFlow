/**
 * The workspace shell.
 *
 * Owns the engine lifecycle and the playback clock. The canvas is driven
 * imperatively through a ref — React never re-renders per tick, it only re-
 * renders when something a human reads has changed (roughly 4 Hz, throttled).
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { Renderer } from '../canvas/Renderer';
import { collectVertices, DEFAULT_SNAP, snap, WallTool } from '../canvas/tools';
import type { Point, ToolId } from '../canvas/tools';
import { History } from '../doc/commands';
import type { VenueDoc } from '../schema/venue';
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
  const [tool, setTool] = useState<ToolId>('select');
  const [canUndo, setCanUndo] = useState(false);
  const [canRedo, setCanRedo] = useState(false);
  const historyRef = useRef<History | null>(null);
  const wallToolRef = useRef(new WallTool());
  const orthoRef = useRef(false);

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
    (json: string, name: string, keepHistory = false) => {
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

      if (!keepHistory) {
        const parsed = JSON.parse(json) as VenueDoc;
        historyRef.current = new History(parsed);
        setCanUndo(false);
        setCanRedo(false);
      }
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

  /** Recompile after an edit, preserving history and the current view. */
  const recompile = useCallback(() => {
    const h = historyRef.current;
    if (!h) return;
    compile(JSON.stringify(h.document), venueTitle, true);
    setCanUndo(h.canUndo);
    setCanRedo(h.canRedo);
  }, [compile, venueTitle]);

  const runCommand = useCallback(
    (cmd: Parameters<History['run']>[0]) => {
      const h = historyRef.current;
      if (!h) return;
      h.run(cmd);
      recompile();
    },
    [recompile],
  );

  const undo = useCallback(() => {
    const h = historyRef.current;
    if (!h?.canUndo) return;
    h.undo();
    recompile();
  }, [recompile]);

  const redo = useCallback(() => {
    const h = historyRef.current;
    if (!h?.canRedo) return;
    h.redo();
    recompile();
  }, [recompile]);

  // Drawing input. The renderer forwards world-space pointer events while a
  // tool is active; panning is suspended so a click means "place a point".
  useEffect(() => {
    const r = rendererRef.current;
    if (!r) return;
    r.drawing = tool === 'wall';
    if (tool !== 'wall') {
      wallToolRef.current.cancel();
      r.clearPreview();
      r.onWorldPointer = null;
      return;
    }

    const floorId = historyRef.current?.document.floors[0]?.id ?? 'f0';

    const resolve = (p: Point): Point => {
      const doc = historyRef.current?.document;
      const verts = doc ? collectVertices(doc, floorId) : [];
      const wt = wallToolRef.current;
      const anchor = wt.preview().at(-2) ?? undefined;
      return snap(p, verts, { ...DEFAULT_SNAP, ortho: orthoRef.current }, anchor);
    };

    r.onWorldPointer = (raw, kind) => {
      const wt = wallToolRef.current;
      const p = resolve(raw);
      if (kind === 'move') {
        wt.moveTo(p);
      } else if (kind === 'down') {
        wt.addPoint(p);
      } else if (kind === 'dblclick') {
        const cmd = wt.finish(floorId);
        r.clearPreview();
        if (cmd) runCommand(cmd);
        return;
      }
      r.setPreview(wt.preview(), p);
    };

    return () => {
      r.onWorldPointer = null;
    };
  }, [tool, runCommand]);

  // Keyboard: undo/redo, finish or abandon a wall, ortho lock.
  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.key === 'Shift') orthoRef.current = true;

      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === 'z') {
        e.preventDefault();
        if (e.shiftKey) redo();
        else undo();
        return;
      }
      if (e.key === 'Escape') {
        wallToolRef.current.cancel();
        rendererRef.current?.clearPreview();
        setTool('select');
      }
      if (e.key === 'Enter' && tool === 'wall') {
        const floorId = historyRef.current?.document.floors[0]?.id ?? 'f0';
        const cmd = wallToolRef.current.finish(floorId);
        rendererRef.current?.clearPreview();
        if (cmd) runCommand(cmd);
      }
      if (e.key === 'w' && !mod) setTool('wall');
      if (e.key === 'v' && !mod) setTool('select');
    };
    const up = (e: KeyboardEvent) => {
      if (e.key === 'Shift') orthoRef.current = false;
    };
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    return () => {
      window.removeEventListener('keydown', down);
      window.removeEventListener('keyup', up);
    };
  }, [tool, undo, redo, runCommand]);

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
          <RailButton
            label="Select (V)"
            glyph="⌖"
            active={tool === 'select'}
            onClick={() => setTool('select')}
          />
          <RailButton
            label="Draw wall (W)"
            glyph="│"
            active={tool === 'wall'}
            onClick={() => setTool('wall')}
          />
          <RailButton label="Zone" glyph="▢" disabled />
          <RailButton label="Door" glyph="◠" disabled />
          <div className="rail-spacer" />
          <RailButton label="Undo (⌘Z)" glyph="↶" onClick={undo} disabled={!canUndo} />
          <RailButton label="Redo (⇧⌘Z)" glyph="↷" onClick={redo} disabled={!canRedo} />
          <RailButton label="Fit view" glyph="⤢" onClick={() => rendererRef.current?.fit()} />
        </nav>

        <div className={`canvas-host${tool === 'wall' ? ' is-drawing' : ''}`} ref={hostRef}>
          {phase === 'loading' && <div className="boot">Starting engine…</div>}
          {tool === 'wall' && (
            <div className="tool-hint">
              Click to place points · Shift to constrain · double-click or Enter to finish ·
              Esc to cancel
            </div>
          )}
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
