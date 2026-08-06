/**
 * Document mutations.
 *
 * **Every change to the venue goes through a `Command`.** Never mutate the
 * document directly — `docs/01-architecture.md` §3 makes this a rule rather
 * than a suggestion, because one discipline buys four things at once: undo and
 * redo, an audit trail of what changed a compliance figure, autosave deltas
 * instead of whole documents, and a path to collaborative editing where
 * commands become operations.
 *
 * A command owns both directions. `apply` performs the change; `invert` returns
 * the command that undoes it. Inverting is computed at construction from the
 * state being replaced, not re-derived at undo time — by then the document has
 * moved on.
 */

import type { Opening, VenueDoc, Wall, Zone, ZoneKind } from '../schema/venue';

/**
 * A reversible change to a document.
 *
 * Generic in the document type because there is more than one: a venue is
 * geometry, a scenario is who walks through it, and they version separately
 * (`docs/02-data-model.md` §1). The discipline is identical for both, so the
 * interface is shared rather than duplicated.
 */
export interface Command<T = VenueDoc> {
  /** Stable identifier, shown in the history and used for coalescing. */
  readonly kind: string;
  /** Human-readable, present tense: "Draw wall", not "Wall drawn". */
  readonly label: string;
  apply(doc: T): T;
  invert(): Command<T>;
  /**
   * Commands sharing a coalesce key merge into one undo entry. A drag produces
   * dozens of moves and a user expects one undo to reverse the whole gesture.
   */
  readonly coalesceKey?: string;
}

/** Structural clone that leaves the original untouched. */
function clone(doc: VenueDoc): VenueDoc {
  return structuredClone(doc);
}

function floorOf(doc: VenueDoc, floorId: string) {
  const f = doc.floors.find((x) => x.id === floorId);
  if (!f) throw new Error(`floor '${floorId}' is not in this document`);
  return f;
}

/** Monotonic ids, prefixed by kind so a document is readable by eye. */
let counter = 0;
export function newId(prefix: string): string {
  counter += 1;
  return `${prefix}_${counter.toString(36)}${Date.now().toString(36).slice(-4)}`;
}

// ---------------------------------------------------------------------------
// Walls
// ---------------------------------------------------------------------------

export class AddWall implements Command {
  readonly kind = 'wall.add';
  readonly label = 'Draw wall';

  constructor(
    private readonly floorId: string,
    private readonly wall: Wall,
  ) {}

  apply(doc: VenueDoc): VenueDoc {
    const next = clone(doc);
    const f = floorOf(next, this.floorId);
    f.walls = [...(f.walls ?? []), this.wall];
    return next;
  }

  invert(): Command {
    return new RemoveWall(this.floorId, this.wall);
  }
}

export class RemoveWall implements Command {
  readonly kind = 'wall.remove';
  readonly label = 'Delete wall';

  constructor(
    private readonly floorId: string,
    private readonly wall: Wall,
  ) {}

  apply(doc: VenueDoc): VenueDoc {
    const next = clone(doc);
    const f = floorOf(next, this.floorId);
    f.walls = (f.walls ?? []).filter((w) => w.id !== this.wall.id);
    // Openings are parameterised on their parent wall, so they cannot outlive
    // it. Leaving them would produce an `openingOrphaned` warning on the very
    // next compile — better to remove them here and let undo restore both.
    f.openings = (f.openings ?? []).filter((o) => o.wall !== this.wall.id);
    return next;
  }

  invert(): Command {
    return new AddWall(this.floorId, this.wall);
  }
}

/** Restores a wall together with the openings that were removed with it. */
export class RestoreWall implements Command {
  readonly kind = 'wall.restore';
  readonly label = 'Restore wall';

  constructor(
    private readonly floorId: string,
    private readonly wall: Wall,
    private readonly openings: Opening[],
  ) {}

  apply(doc: VenueDoc): VenueDoc {
    const next = clone(doc);
    const f = floorOf(next, this.floorId);
    f.walls = [...(f.walls ?? []), this.wall];
    f.openings = [...(f.openings ?? []), ...this.openings];
    return next;
  }

  invert(): Command {
    return new RemoveWall(this.floorId, this.wall);
  }
}

// ---------------------------------------------------------------------------
// Openings
// ---------------------------------------------------------------------------

export class AddOpening implements Command {
  readonly kind = 'opening.add';
  readonly label = 'Add door';

  constructor(
    private readonly floorId: string,
    private readonly opening: Opening,
  ) {}

  apply(doc: VenueDoc): VenueDoc {
    const next = clone(doc);
    const f = floorOf(next, this.floorId);
    f.openings = [...(f.openings ?? []), this.opening];
    return next;
  }

  invert(): Command {
    return new RemoveOpening(this.floorId, this.opening);
  }
}

export class RemoveOpening implements Command {
  readonly kind = 'opening.remove';
  readonly label = 'Delete door';

  constructor(
    private readonly floorId: string,
    private readonly opening: Opening,
  ) {}

  apply(doc: VenueDoc): VenueDoc {
    const next = clone(doc);
    const f = floorOf(next, this.floorId);
    f.openings = (f.openings ?? []).filter((o) => o.id !== this.opening.id);
    return next;
  }

  invert(): Command {
    return new AddOpening(this.floorId, this.opening);
  }
}

// ---------------------------------------------------------------------------
// Zones
// ---------------------------------------------------------------------------

export class AddZone implements Command {
  readonly kind = 'zone.add';
  readonly label = 'Draw zone';

  constructor(
    private readonly floorId: string,
    private readonly zone: Zone,
  ) {}

  apply(doc: VenueDoc): VenueDoc {
    const next = clone(doc);
    const f = floorOf(next, this.floorId);
    f.zones = [...(f.zones ?? []), this.zone];
    return next;
  }

  invert(): Command {
    return new RemoveZone(this.floorId, this.zone);
  }
}

export class RemoveZone implements Command {
  readonly kind = 'zone.remove';
  readonly label = 'Delete zone';

  constructor(
    private readonly floorId: string,
    private readonly zone: Zone,
  ) {}

  apply(doc: VenueDoc): VenueDoc {
    const next = clone(doc);
    const f = floorOf(next, this.floorId);
    f.zones = (f.zones ?? []).filter((z) => z.id !== this.zone.id);
    return next;
  }

  invert(): Command {
    return new AddZone(this.floorId, this.zone);
  }
}

// ---------------------------------------------------------------------------
// Property edits
// ---------------------------------------------------------------------------

/**
 * Change one field of one element.
 *
 * Generic rather than a command per property: the alternative is a dozen
 * near-identical classes that differ only in a field name, and each one is
 * another chance to write an `invert` that restores the wrong thing.
 *
 * The previous value is captured at construction, not read back at undo time —
 * by then the document has moved on and the old value is gone.
 */
export class SetProperty<T> implements Command {
  readonly kind: string;
  readonly label: string;
  readonly coalesceKey: string;
  private readonly previous: T;

  constructor(
    private readonly floorId: string,
    private readonly collection: 'walls' | 'openings' | 'zones',
    private readonly elementId: string,
    private readonly field: string,
    private readonly value: T,
    doc: VenueDoc,
    label?: string,
  ) {
    this.kind = `${collection}.set.${field}`;
    this.label = label ?? `Change ${field}`;
    // A drag over a numeric field emits many commands; they merge into one
    // undo entry because a user considers the whole gesture a single change.
    this.coalesceKey = `${collection}:${elementId}:${field}`;

    const el = SetProperty.find(doc, floorId, collection, elementId);
    this.previous = (el as Record<string, unknown>)?.[field] as T;
  }

  private static find(
    doc: VenueDoc,
    floorId: string,
    collection: 'walls' | 'openings' | 'zones',
    id: string,
  ): unknown {
    const f = doc.floors.find((x) => x.id === floorId);
    if (!f) return undefined;
    const list = (f[collection] ?? []) as Array<{ id: string }>;
    return list.find((e) => e.id === id);
  }

  apply(doc: VenueDoc): VenueDoc {
    const next = clone(doc);
    const f = floorOf(next, this.floorId);
    const list = (f[this.collection] ?? []) as unknown as Array<Record<string, unknown>>;
    const el = list.find((e) => e.id === this.elementId);
    if (el) el[this.field] = this.value as unknown;
    return next;
  }

  invert(): Command {
    // Reconstructing needs a document to read the old value from, which is
    // exactly what we already captured — so build the inverse directly.
    const inverse = Object.create(SetProperty.prototype) as SetProperty<T>;
    Object.assign(inverse, {
      floorId: this.floorId,
      collection: this.collection,
      elementId: this.elementId,
      field: this.field,
      value: this.previous,
      previous: this.value,
      kind: this.kind,
      label: this.label,
      coalesceKey: this.coalesceKey,
    });
    return inverse;
  }
}

/** Set a doorway's clear width, in metres. */
export function setOpeningWidth(
  doc: VenueDoc,
  floorId: string,
  openingId: string,
  widthM: number,
): Command {
  return new SetProperty(floorId, 'openings', openingId, 'widthM', widthM, doc, 'Change door width');
}

/** Mark a doorway as a fire exit, or not. */
export function setOpeningFireExit(
  doc: VenueDoc,
  floorId: string,
  openingId: string,
  isFireExit: boolean,
): Command {
  return new SetProperty(
    floorId,
    'openings',
    openingId,
    'isFireExit',
    isFireExit,
    doc,
    isFireExit ? 'Mark as fire exit' : 'Clear fire exit',
  );
}

/** Set a wall's thickness, in metres. */
export function setWallThickness(
  doc: VenueDoc,
  floorId: string,
  wallId: string,
  thicknessM: number,
): Command {
  return new SetProperty(
    floorId,
    'walls',
    wallId,
    'thicknessM',
    thicknessM,
    doc,
    'Change wall thickness',
  );
}

/**
 * Set a zone's occupancy classification.
 *
 * This is the field NFPA 101 occupant load is derived from, so changing it
 * changes the venue's legal capacity — which is why it is a command with an
 * undo entry rather than a quiet edit.
 */
export function setZoneKind(
  doc: VenueDoc,
  floorId: string,
  zoneId: string,
  kind: ZoneKind,
): Command {
  return new SetProperty(floorId, 'zones', zoneId, 'kind', kind, doc, 'Change zone use');
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

/** Edits closer together than this merge into one undo entry. */
const COALESCE_WINDOW_MS = 700;

/**
 * An undo/redo stack over a document.
 *
 * Holds inverse commands rather than document snapshots. A venue with tens of
 * thousands of vertices makes snapshots expensive to keep and slow to compare;
 * an inverse command is a handful of bytes and states exactly what changed,
 * which is also what an audit trail needs.
 */
export class History<T = VenueDoc> {
  private undoStack: Command<T>[] = [];
  private redoStack: Command<T>[] = [];
  private lastRunAt = 0;

  constructor(
    private doc: T,
    /** Cap on retained entries. 200 is far more than a session needs. */
    private readonly limit = 200,
  ) {}

  get document(): T {
    return this.doc;
  }

  /**
   * How many entries deep the undo stack is.
   *
   * Exposed so a caller can tell a coalesced edit from a new one without the
   * `run` method having to report it — `SessionHistory` uses this to keep two
   * documents' undo stacks interleaved in the right order.
   */
  get depth(): number {
    return this.undoStack.length;
  }

  get canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  get canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  /** Label of the change undo would reverse, for the menu item. */
  get undoLabel(): string | null {
    return this.undoStack.at(-1)?.label ?? null;
  }

  get redoLabel(): string | null {
    return this.redoStack.at(-1)?.label ?? null;
  }

  /**
   * Apply a command and push its inverse.
   *
   * Commands sharing a `coalesceKey` with the previous one replace it rather
   * than stacking: dragging a width slider emits dozens of edits and a user
   * expects one undo to reverse the whole gesture, not thirty.
   */
  run(cmd: Command<T>): T {
    this.doc = cmd.apply(this.doc);

    const top = this.undoStack.at(-1);
    const merge =
      cmd.coalesceKey !== undefined &&
      top?.coalesceKey === cmd.coalesceKey &&
      Date.now() - this.lastRunAt < COALESCE_WINDOW_MS;
    this.lastRunAt = Date.now();

    if (merge) {
      // Keep the *older* inverse: it restores the value before the gesture
      // began, which is what undoing the gesture means.
      this.redoStack = [];
      return this.doc;
    }

    this.undoStack.push(cmd.invert());
    if (this.undoStack.length > this.limit) this.undoStack.shift();
    // A new action makes the redo branch unreachable. Keeping it would let a
    // user redo their way into a document that never existed.
    this.redoStack = [];
    return this.doc;
  }

  undo(): T {
    const cmd = this.undoStack.pop();
    if (!cmd) return this.doc;
    this.doc = cmd.apply(this.doc);
    this.redoStack.push(cmd.invert());
    return this.doc;
  }

  redo(): T {
    const cmd = this.redoStack.pop();
    if (!cmd) return this.doc;
    this.doc = cmd.apply(this.doc);
    this.undoStack.push(cmd.invert());
    return this.doc;
  }

  /** Replace the document and clear history — loading a different venue. */
  reset(doc: T): void {
    this.doc = doc;
    this.undoStack = [];
    this.redoStack = [];
  }
}

/** Which document an undo entry belongs to. */
export type DocumentKind = 'venue' | 'scenario';

/**
 * The part of a `History` that `SessionHistory` needs.
 *
 * Structural rather than generic so histories over different document types
 * can share one stack without the combined type having to name either.
 */
export interface UndoableHistory {
  readonly depth: number;
  readonly undoLabel: string | null;
  undo(): unknown;
  redo(): unknown;
}

/**
 * One undo stack across two documents.
 *
 * A venue and a scenario version separately, so each keeps its own `History`.
 * But a user has one ⌘Z, and it must reverse *the last thing they did* — not
 * the last thing they did to whichever panel happens to be open. This keeps
 * the interleaving: a list of which document each edit touched, in order.
 *
 * Coalesced edits are detected by watching the sub-history's depth rather than
 * by asking it whether it merged. A drag that merges into the previous entry
 * must not push a second entry here, or one undo would reverse the drag and the
 * next would reverse nothing.
 */
export class SessionHistory {
  private order: DocumentKind[] = [];
  private redoOrder: DocumentKind[] = [];

  constructor(private readonly histories: Record<DocumentKind, UndoableHistory>) {}

  get canUndo(): boolean {
    return this.order.length > 0;
  }

  get canRedo(): boolean {
    return this.redoOrder.length > 0;
  }

  /** Label of the change undo would reverse, for a menu item or tooltip. */
  get undoLabel(): string | null {
    const which = this.order.at(-1);
    return which ? this.histories[which].undoLabel : null;
  }

  /** Record that `which` history just ran a command, if it kept it. */
  record(which: DocumentKind, depthBefore: number): void {
    if (this.histories[which].depth <= depthBefore) return;
    this.order.push(which);
    this.redoOrder = [];
  }

  /** Undo the most recent edit to either document. Returns which one moved. */
  undo(): DocumentKind | null {
    const which = this.order.pop();
    if (!which) return null;
    this.histories[which].undo();
    this.redoOrder.push(which);
    return which;
  }

  redo(): DocumentKind | null {
    const which = this.redoOrder.pop();
    if (!which) return null;
    this.histories[which].redo();
    this.order.push(which);
    return which;
  }

  clear(): void {
    this.order = [];
    this.redoOrder = [];
  }
}
