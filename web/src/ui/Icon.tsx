/**
 * The icon set.
 *
 * Drawn rather than borrowed. Unicode glyphs (⌖ │ ▢ ◠ ↶ ⤢) were the first
 * attempt and were wrong in three ways: they render differently on every
 * platform, their weights and optical sizes do not match each other, and
 * several fall back to a missing-glyph box on Linux. An instrument panel whose
 * controls change shape depending on the machine is not an instrument panel.
 *
 * # The drawing rules
 *
 * These are drafting marks, not app icons. Every one is built from the same
 * primitives a technical drawing uses:
 *
 * - **16 unit grid**, so every shape lands on a whole or half unit.
 * - **1.5 unit stroke**, uniform. No filled shapes except where a mark is
 *   genuinely solid (a vertex node, the play triangle).
 * - **Butt caps and mitre joins.** Rounded caps read as friendly; this product
 *   is not friendly, it is precise.
 * - **`currentColor` throughout**, so an icon inherits its control's state
 *   rather than carrying colour of its own — the same rule the palette follows.
 *
 * Where a tool has a real-world drafting convention, the icon uses it: a
 * doorway is a leaf and its swing arc, exactly as it appears on a floor plan.
 */

interface IconProps {
  /** Pixel size. 16 is the rail default; 14 suits inline controls. */
  size?: number;
  className?: string;
}

function Svg({ size = 16, className, children }: IconProps & { children: React.ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="butt"
      strokeLinejoin="miter"
      className={className}
      aria-hidden="true"
      focusable="false"
    >
      {children}
    </svg>
  );
}

/** Selection: a survey crosshair with a centre mark. */
export function IconSelect(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M8 1.5v3.5M8 11v3.5M1.5 8h3.5M11 8h3.5" />
      <circle cx="8" cy="8" r="2.75" />
    </Svg>
  );
}

/** Wall: a drafted run with node markers at each end. */
export function IconWall(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M2.5 12.5 8 3.5l5.5 9" />
      <circle cx="2.5" cy="12.5" r="1.25" fill="currentColor" stroke="none" />
      <circle cx="13.5" cy="12.5" r="1.25" fill="currentColor" stroke="none" />
      <circle cx="8" cy="3.5" r="1.25" fill="currentColor" stroke="none" />
    </Svg>
  );
}

/** Zone: a closed region, hatched the way an area is called out on a plan. */
export function IconZone(p: IconProps) {
  return (
    <Svg {...p}>
      <rect x="2.25" y="2.25" width="11.5" height="11.5" />
      <path d="M2.25 9.5 9.5 2.25M6 13.75l7.75-7.75" strokeWidth={1} opacity={0.55} />
    </Svg>
  );
}

/** Doorway: a leaf and its swing arc — the floor-plan convention. */
export function IconDoor(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M2 13.5h2.5M11.5 13.5H14" />
      <path d="M4.5 13.5V4" />
      <path d="M4.5 4a9.5 9.5 0 0 1 9.5 9.5" strokeWidth={1} opacity={0.65} />
    </Svg>
  );
}

export function IconUndo(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M2.5 6.5h7a3.5 3.5 0 0 1 0 7H6" />
      <path d="M5.5 3.5 2.5 6.5l3 3" />
    </Svg>
  );
}

export function IconRedo(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M13.5 6.5h-7a3.5 3.5 0 0 0 0 7H10" />
      <path d="M10.5 3.5l3 3-3 3" />
    </Svg>
  );
}

/** Fit to view: corner marks pulling outward, as on a crop frame. */
export function IconFit(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M2 5.5V2h3.5M10.5 2H14v3.5M14 10.5V14h-3.5M5.5 14H2v-3.5" />
      <rect x="5.5" y="5.5" width="5" height="5" strokeWidth={1} opacity={0.55} />
    </Svg>
  );
}

/** Report: a sheet with ruled lines and a folded corner. */
export function IconReport(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M3 1.75h6.5L13 5.25v9H3z" />
      <path d="M9.5 1.75v3.5H13" strokeWidth={1} />
      <path d="M5.5 8h5M5.5 10.5h5" strokeWidth={1} opacity={0.7} />
    </Svg>
  );
}

export function IconPlay(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M5 3.5 12.5 8 5 12.5z" fill="currentColor" stroke="none" />
    </Svg>
  );
}

export function IconPause(p: IconProps) {
  return (
    <Svg {...p}>
      <rect x="4.5" y="3.5" width="2.5" height="9" fill="currentColor" stroke="none" />
      <rect x="9" y="3.5" width="2.5" height="9" fill="currentColor" stroke="none" />
    </Svg>
  );
}

/** Reset: a full revolution, distinct from undo's partial turn. */
export function IconReset(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M13.25 8a5.25 5.25 0 1 1-1.85-4" />
      <path d="M13.5 2v3.25h-3.25" />
    </Svg>
  );
}

/** Open: a sheet entering a tray. */
export function IconOpen(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M2 12.5v-9h4.5L8 5.5h6v7z" />
      <path d="M8 8v4M6 10l2 2 2-2" strokeWidth={1.25} />
    </Svg>
  );
}

/** Place agents: a scatter of bodies. */
export function IconAgents(p: IconProps) {
  return (
    <Svg {...p}>
      <circle cx="4" cy="4.5" r="1.6" fill="currentColor" stroke="none" />
      <circle cx="11.5" cy="3.5" r="1.6" fill="currentColor" stroke="none" />
      <circle cx="8" cy="8.5" r="1.6" fill="currentColor" stroke="none" />
      <circle cx="3.5" cy="12" r="1.6" fill="currentColor" stroke="none" />
      <circle cx="12" cy="11.5" r="1.6" fill="currentColor" stroke="none" />
    </Svg>
  );
}

/** Delete: a cross, unmistakable at 14 px. */
export function IconDelete(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M4 4l8 8M12 4l-8 8" />
    </Svg>
  );
}

/** Annunciator state marks. Distinct in shape, not only in colour, so the
 *  panel remains readable to a colour-blind reader and in monochrome print. */
export function IconNormal(p: IconProps) {
  return (
    <Svg {...p}>
      <circle cx="8" cy="8" r="4" fill="currentColor" stroke="none" />
    </Svg>
  );
}

export function IconSupervise(p: IconProps) {
  return (
    <Svg {...p}>
      <path d="M8 3l5 9H3z" fill="currentColor" stroke="none" />
    </Svg>
  );
}

export function IconAlarm(p: IconProps) {
  return (
    <Svg {...p}>
      <rect x="3.5" y="3.5" width="9" height="9" fill="currentColor" stroke="none" />
    </Svg>
  );
}
