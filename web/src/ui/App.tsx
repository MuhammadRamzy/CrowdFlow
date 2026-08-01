/**
 * The workspace shell.
 *
 * Owns the engine lifecycle and the playback clock. The canvas is driven
 * imperatively through a ref — React never re-renders per tick, it only re-
 * renders when something a human reads has changed (roughly 4 Hz, throttled).
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { Renderer } from '../canvas/Renderer';
import {
  collectVertices,
  DEFAULT_SNAP,
  DoorTool,
  hitTest,
  snap,
  WallTool,
  ZoneTool,
} from '../canvas/tools';
import type { Point, Selection, ToolId } from '../canvas/tools';
import { History, RemoveOpening, RemoveWall, RemoveZone } from '../doc/commands';
import type { VenueDoc } from '../schema/venue';
import { engineVersion, loadEngine, Run, Venue } from '../engine';
import { useApp } from '../state/store';
import { Inspector } from './Inspector';
import { StatusBar } from './StatusBar';
import { Timeline } from './Timeline';
import { Validation } from './Validation';
import { Report } from '../report/Report';
import {
  IconDoor,
  IconFit,
  IconRedo,
  IconReport,
  IconSelect,
  IconUndo,
  IconWall,
  IconZone,
} from './Icon';
import type { ReportData } from '../report/Report';
import fixtureJson from '../../../fixtures/unit/hall-two-doors.venue.json?raw';

/** What each tool expects the user to do. Present tense, no exclamation. */
const HINTS: Record<Exclude<ToolId, 'select'>, string> = {
  wall: 'Click to place points · Shift constrains to 45° · double-click or Enter to finish · Esc cancels',
  zone: 'Click to outline an area · three points minimum · double-click or Enter to close · Esc cancels',
  door: 'Click a wall to place a 1.8 m doorway · Esc returns to select',
};

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
  // The renderer is created asynchronously in the boot effect, so effects that
  // attach to it must depend on something that changes when it appears.
  // Without this the input handler is installed on the very first render —
  // when rendererRef is still null — and never again, so canvas input works
  // only after the user happens to switch tools.
  const [rendererReady, setRendererReady] = useState(false);
  const [report, setReport] = useState<ReportData | null>(null);
  const [tool, setTool] = useState<ToolId>('select');
  const [canUndo, setCanUndo] = useState(false);
  const [canRedo, setCanRedo] = useState(false);
  const historyRef = useRef<History | null>(null);
  const wallToolRef = useRef(new WallTool());
  const zoneToolRef = useRef(new ZoneTool());
  const doorToolRef = useRef(new DoorTool());
  const [selection, setSelection] = useState<Selection | null>(null);
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
        setRendererReady(true);

        setEngineVersion(engineVersion());
        compile(fixtureJson, 'Hall with two doors');
        setPhase('ready');

        // A dev-only handle on the internals. Driving the real UI is how the
        // last four bugs were found, and reaching the renderer and document
        // from the console is what makes that practical.
        if (import.meta.env.DEV) {
          (window as unknown as Record<string, unknown>).__cf = {
            renderer: () => rendererRef.current,
            doc: () => historyRef.current?.document,
            run: () => runRef.current,
            hitTest,
          };
        }
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

  // Canvas input. The renderer forwards world-space pointer events while a tool
  // is active; panning is suspended so a click means "act here".
  useEffect(() => {
    const r = rendererRef.current;
    if (!r) return;

    const drawing = tool === 'wall' || tool === 'zone' || tool === 'door';
    r.drawing = drawing;
    if (!drawing) {
      wallToolRef.current.cancel();
      zoneToolRef.current.cancel();
      r.clearPreview();
    }

    const floorId = historyRef.current?.document.floors[0]?.id ?? 'f0';

    const resolve = (p: Point, anchor?: Point): Point => {
      const doc = historyRef.current?.document;
      const verts = doc ? collectVertices(doc, floorId) : [];
      return snap(p, verts, { ...DEFAULT_SNAP, ortho: orthoRef.current }, anchor);
    };

    r.onWorldPointer = (raw, kind) => {
      const doc = historyRef.current?.document;
      if (!doc) return;

      if (tool === 'select') {
        if (kind === 'down') setSelection(hitTest(doc, floorId, raw, 0.4));
        return;
      }

      if (tool === 'door') {
        const hit = doorToolRef.current.hover(doc, floorId, raw);
        // Preview the exact spot on the wall, so it is obvious which wall a
        // door will land on before committing.
        r.setPreview([], hit ? hit.point : null);
        if (kind === 'down' && hit) {
          runCommand(doorToolRef.current.place(floorId, hit.wallId, hit.t));
        }
        return;
      }

      const t = tool === 'wall' ? wallToolRef.current : zoneToolRef.current;
      const anchor = t.preview().at(-2) ?? undefined;
      const p = resolve(raw, anchor);

      if (kind === 'move') {
        t.moveTo(p);
      } else if (kind === 'down') {
        t.addPoint(p);
      } else if (kind === 'dblclick') {
        const cmd = t.finish(floorId);
        r.clearPreview();
        if (cmd) runCommand(cmd);
        return;
      }
      r.setPreview(t.preview(), p);
    };

    return () => {
      r.onWorldPointer = null;
    };
  }, [tool, runCommand, rendererReady]);

  // Highlight the selection on the canvas. The inspector panel alone is not
  // enough feedback when the thing selected is one wall among many.
  useEffect(() => {
    const r = rendererRef.current;
    const doc = historyRef.current?.document;
    if (!r) return;
    if (!selection || !doc) {
      r.setSelection(null, false);
      return;
    }
    const floor = doc.floors[0];
    if (!floor) return;

    if (selection.kind === 'wall') {
      const w = (floor.walls ?? []).find((x) => x.id === selection.id);
      r.setSelection(w ? w.polyline.map((q) => ({ x: q[0]!, y: q[1]! })) : null, false);
    } else if (selection.kind === 'zone') {
      const z = (floor.zones ?? []).find((x) => x.id === selection.id);
      r.setSelection(z ? z.polygon.map((q) => ({ x: q[0]!, y: q[1]! })) : null, true);
    } else {
      const o = (floor.openings ?? []).find((x) => x.id === selection.id);
      const w = o && (floor.walls ?? []).find((x) => x.id === o.wall);
      if (!o || !w) {
        r.setSelection(null, false);
        return;
      }
      // Draw the doorway's actual span along its wall, not just a marker: the
      // clear width is the figure that matters and it should be visible.
      const total = polylineLengthOf(w.polyline);
      const half = o.widthM / 2 / total;
      const a = pointAtParam(w.polyline, Math.max(0, o.t - half));
      const b = pointAtParam(w.polyline, Math.min(1, o.t + half));
      r.setSelection(a && b ? [a, b] : null, false);
    }
  }, [selection, canUndo, canRedo, rendererReady]);

  /** Delete whatever is selected. */
  const deleteSelection = useCallback(() => {
    const h = historyRef.current;
    if (!h || !selection) return;
    const floor = h.document.floors[0];
    if (!floor) return;

    if (selection.kind === 'wall') {
      const wall = (floor.walls ?? []).find((w) => w.id === selection.id);
      if (wall) runCommand(new RemoveWall(floor.id, wall));
    } else if (selection.kind === 'zone') {
      const zone = (floor.zones ?? []).find((z) => z.id === selection.id);
      if (zone) runCommand(new RemoveZone(floor.id, zone));
    } else {
      const op = (floor.openings ?? []).find((o) => o.id === selection.id);
      if (op) runCommand(new RemoveOpening(floor.id, op));
    }
    setSelection(null);
  }, [selection, runCommand]);

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
        zoneToolRef.current.cancel();
        rendererRef.current?.clearPreview();
        setSelection(null);
        setTool('select');
      }
      if (e.key === 'Enter' && (tool === 'wall' || tool === 'zone')) {
        const floorId = historyRef.current?.document.floors[0]?.id ?? 'f0';
        const t = tool === 'wall' ? wallToolRef.current : zoneToolRef.current;
        const cmd = t.finish(floorId);
        rendererRef.current?.clearPreview();
        if (cmd) runCommand(cmd);
      }
      if ((e.key === 'Delete' || e.key === 'Backspace') && tool === 'select') {
        e.preventDefault();
        deleteSelection();
      }
      if (!mod) {
        if (e.key === 'v') setTool('select');
        if (e.key === 'w') setTool('wall');
        if (e.key === 'z' && !e.shiftKey) setTool('zone');
        if (e.key === 'd') setTool('door');
      }
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
  }, [tool, undo, redo, runCommand, deleteSelection]);

  /** Assemble the dossier from whatever the engine has actually produced. */
  const openReport = useCallback(async () => {
    const st = useApp.getState();
    // Snapshot the canvas as it stands: the drawing in the report is the venue
    // that was analysed, at the zoom the reviewer left it.
    const r = rendererRef.current;
    const run = runRef.current;
    const plan = (await r?.snapshot()) ?? null;

    // A second frame showing peak density, captured by switching the overlay to
    // its peak field and restoring afterwards. The reviewer wants the worst
    // moment, not whatever the crowd happened to be doing when the button was
    // pressed.
    let heatmap: string | null = null;
    if (r && run && run.spawned > 0) {
      r.setDensity(run.density(true));
      heatmap = await r.snapshot();
      r.setDensity(st.showHeatmap ? run.density(st.heatmapPeak) : null);
    }
    setReport({
      venueName: st.venueName,
      document: historyRef.current?.document ?? null,
      walkableArea: st.walkableArea,
      warnings: st.warnings,
      stats: st.stats,
      egressTime: st.egressTime,
      peakOccupancy: st.peakOccupancy,
      peakDensity: st.peakDensity,
      criticalArea: st.criticalArea,
      thresholds: st.thresholds,
      engineVersion: st.engineVersion,
      plan,
      heatmap,
      generatedAt: new Date(),
    });
  }, []);

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

  if (report) {
    return <Report data={report} onClose={() => setReport(null)} />;
  }

  return (
    <div className="shell">
      <StatusBar />

      <main className="body">
        <nav className="rail" aria-label="Tools">
          <RailButton
            label="Select"
            shortcut="V"
            icon={<IconSelect />}
            active={tool === 'select'}
            onClick={() => setTool('select')}
          />
          <RailButton
            label="Draw wall"
            shortcut="W"
            icon={<IconWall />}
            active={tool === 'wall'}
            onClick={() => setTool('wall')}
          />
          <RailButton
            label="Draw zone"
            shortcut="Z"
            icon={<IconZone />}
            active={tool === 'zone'}
            onClick={() => setTool('zone')}
          />
          <RailButton
            label="Place door"
            shortcut="D"
            icon={<IconDoor />}
            active={tool === 'door'}
            onClick={() => setTool('door')}
          />
          <div className="rail-rule" />
          <RailButton label="Undo" shortcut="⌘Z" icon={<IconUndo />} onClick={undo} disabled={!canUndo} />
          <RailButton label="Redo" shortcut="⇧⌘Z" icon={<IconRedo />} onClick={redo} disabled={!canRedo} />
          <div className="rail-spacer" />
          <RailButton label="Fit view" icon={<IconFit />} onClick={() => rendererRef.current?.fit()} />
          <RailButton label="Compliance report" icon={<IconReport />} onClick={openReport} />
        </nav>

        <div
          className={`canvas-host${tool !== 'select' ? ' is-drawing' : ''}`}
          ref={hostRef}
        >
          {phase === 'loading' && <div className="boot">Starting engine…</div>}
          {tool !== 'select' && <div className="tool-hint">{HINTS[tool]}</div>}
        </div>

        <aside className="inspector" aria-label="Inspector">
          <Inspector
            venueTitle={venueTitle}
            placedAgents={placedAgents}
            onLoadFile={onLoadFile}
            onReset={reset}
            selection={selection}
            document={historyRef.current?.document ?? null}
            onDeleteSelection={deleteSelection}
            onEdit={runCommand}
          />
          <Validation />
        </aside>
      </main>

      <Timeline onReset={reset} hasRun={hasRun} />
    </div>
  );
}

/** Total arc length of a polyline, metres. */
function polylineLengthOf(poly: number[][]): number {
  let total = 0;
  for (let i = 0; i + 1 < poly.length; i++) {
    total += Math.hypot(poly[i + 1]![0]! - poly[i]![0]!, poly[i + 1]![1]! - poly[i]![1]!);
  }
  return total;
}

/** Point at normalised arc-length `t`, matching `Polyline::point_at` in Rust. */
function pointAtParam(poly: number[][], t: number): { x: number; y: number } | null {
  if (poly.length < 2) return null;
  const total = polylineLengthOf(poly);
  if (total <= 0) return { x: poly[0]![0]!, y: poly[0]![1]! };
  const target = Math.max(0, Math.min(1, t)) * total;
  let walked = 0;
  for (let i = 0; i + 1 < poly.length; i++) {
    const seg = Math.hypot(poly[i + 1]![0]! - poly[i]![0]!, poly[i + 1]![1]! - poly[i]![1]!);
    if (walked + seg >= target) {
      const local = seg > 0 ? (target - walked) / seg : 0;
      return {
        x: poly[i]![0]! + (poly[i + 1]![0]! - poly[i]![0]!) * local,
        y: poly[i]![1]! + (poly[i + 1]![1]! - poly[i]![1]!) * local,
      };
    }
    walked += seg;
  }
  return { x: poly.at(-1)![0]!, y: poly.at(-1)![1]! };
}

function RailButton({
  label,
  shortcut,
  icon,
  active,
  disabled,
  onClick,
}: {
  label: string;
  shortcut?: string;
  icon: React.ReactNode;
  active?: boolean;
  disabled?: boolean;
  onClick?: () => void;
}) {
  // The shortcut belongs in the tooltip, not the accessible name: a screen
  // reader announcing "Draw wall W" is worse than "Draw wall".
  const title = shortcut ? `${label}  ${shortcut}` : label;
  return (
    <button
      type="button"
      className={`rail-btn${active ? ' is-active' : ''}`}
      title={title}
      aria-label={label}
      aria-pressed={active}
      disabled={disabled}
      onClick={onClick}
    >
      {icon}
    </button>
  );
}
