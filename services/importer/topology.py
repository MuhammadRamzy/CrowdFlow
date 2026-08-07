"""Topology repair — turning a bag of line work into walls, rooms and doorways.

This is the part that earns its keep. A drawing exported from a real practice is not a
graph: the same wall is traced twice by two people, corners miss by 4 mm, a polyline that
looks closed is not, a doorway is a hole in a wall on one layer and a swing arc on
another, and every wall is two parallel lines rather than one centreline. None of that is
a defect in the file — it is what CAD *is*, because CAD is a drawing tool and we are
trying to read it as a model.

The sequence follows `docs/03-track-a-venue-designer.md` A4, minus the steps that need the
planar arrangement (face extraction for room detection) which the vector path gets from an
outline layer instead. Every step is a pure function over segments so it can be tested
alone, and every step reports what it changed so the review UI can explain itself.

**Determinism.** No step depends on the order entities appeared in the file. Inputs are
sorted into canonical order, grouping uses coordinate-derived keys, and ties break on
coordinates. Two exports of the same drawing must produce the same venue, or the diff
between two venue versions becomes meaningless.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field, replace
from typing import TYPE_CHECKING

from importer.geometry import (
    angle_between,
    cluster_points,
    distance,
    dot,
    line_intersection,
    lerp,
    perpendicular_offset,
    project_param,
    sub,
)
from importer.linework import Point, Polyline, Segment

if TYPE_CHECKING:
    from collections.abc import Sequence


@dataclass(frozen=True, slots=True)
class RepairOptions:
    """Tolerances for the repair sequence.

    Every one of these is a judgement call whose right value differs between a crisp CAD
    export and a fax-quality scan, which is why A4's review UI exposes them as a single
    "aggressiveness" slider. The defaults here are tuned for CAD exports: tight enough
    not to invent geometry, loose enough to survive normal drafting slop.
    """

    #: Endpoint clustering radius, metres. 50 mm is well below any real feature and well
    #: above the slop in a drawing that was snapped to a 1 mm grid by hand.
    eps_m: float = 0.05
    #: Segments shorter than this are noise — hatch fragments, curve-flattening stubs.
    min_segment_m: float = 0.02
    #: Two lines within this angle count as collinear/parallel.
    angle_tol_deg: float = 2.0
    #: How far a dangling wall end may be extended to close a junction.
    junction_extend_m: float = 0.25
    #: Gaps in a wall run in this range become candidate openings. The lower bound is
    #: below the 0.85 m minimum clear width the compiler warns about, so a
    #: non-compliant door is *detected and flagged* rather than silently not detected.
    gap_min_m: float = 0.70
    gap_max_m: float = 3.00
    #: Pair parallel lines this far apart into one centreline wall with a thickness.
    pair_parallel: bool = True
    thickness_min_m: float = 0.08
    thickness_max_m: float = 0.50
    #: Two parallel faces must overlap by at least this fraction of the shorter one
    #: before they are believed to be the two sides of the same wall.
    pair_overlap_frac: float = 0.5
    #: Thickness assigned to a wall we could not pair. Matches the schema default.
    default_thickness_m: float = 0.20

    @property
    def angle_tol_rad(self) -> float:
        """`angle_tol_deg` in radians."""
        return math.radians(self.angle_tol_deg)


@dataclass(slots=True)
class RepairReport:
    """What repair did, in numbers a reviewer can be shown.

    The review UI bands elements by confidence; these counts are what justifies the
    band. "We merged 412 duplicate segments and closed 37 junctions" is the difference
    between a user trusting the import and a user redrawing it by hand.
    """

    input_segments: int = 0
    dropped_short: int = 0
    snapped_endpoints: int = 0
    removed_duplicates: int = 0
    merged_collinear: int = 0
    closed_junctions: int = 0
    paired_parallel: int = 0
    bridged_openings: int = 0
    output_walls: int = 0
    warnings: list[str] = field(default_factory=list)

    def summary(self) -> str:
        """One-line human summary, for the CLI and the import job record."""
        return (
            f"{self.input_segments} segments -> {self.output_walls} walls "
            f"(dropped {self.dropped_short} short, {self.removed_duplicates} duplicate; "
            f"snapped {self.snapped_endpoints} endpoints; merged {self.merged_collinear} "
            f"collinear; paired {self.paired_parallel} parallel; closed "
            f"{self.closed_junctions} junctions; inferred {self.bridged_openings} openings)"
        )


@dataclass(frozen=True, slots=True)
class WallRun:
    """A repaired wall: a polyline plus the thickness we believe it has."""

    polyline: Polyline
    thickness_m: float
    #: 0..1. Lowered when the wall's thickness was assumed rather than measured.
    confidence: float = 1.0


@dataclass(frozen=True, slots=True)
class OpeningCandidate:
    """A doorway, before it is attached to a wall.

    Held in world coordinates because that is what both sources produce — a gap between
    two wall runs, or a cluster of geometry on a door layer. :mod:`importer.emit` turns
    it into the schema's parametric `(wall, t)` form.
    """

    centre: Point
    width_m: float
    #: Unit vector along the wall the opening sits in.
    direction: Point
    #: ``"gap"`` when inferred from a hole in a wall run, ``"door"`` when a door layer
    #: said so. Door-layer evidence outranks gap inference where the two collide.
    origin: str
    confidence: float


# --------------------------------------------------------------------------- steps


def drop_short(segments: Sequence[Segment], min_length: float) -> tuple[list[Segment], int]:
    """Remove segments shorter than *min_length*.

    Run first, because a zero-length segment has no direction and would join a
    meaningless collinear group, and because curve flattening produces them in bulk.
    """
    kept = [s for s in segments if s.length >= min_length]
    return kept, len(segments) - len(kept)


def snap_endpoints(segments: Sequence[Segment], eps: float) -> tuple[list[Segment], int]:
    """Cluster all endpoints within *eps* and move each to its cluster centroid.

    After this, "the same point" is testable with ``==``, which every later step relies
    on. Segments that collapse to zero length are dropped — that is a real outcome when
    a drawing contains a 3 mm stub between two walls.
    """
    if not segments:
        return [], 0
    points = [p for s in segments for p in (s.a, s.b)]
    mapping = cluster_points(points, eps)
    moved = sum(1 for p in set(points) if mapping[p] != p)
    out: list[Segment] = []
    for s in segments:
        a, b = mapping[s.a], mapping[s.b]
        if a != b:
            out.append(replace(s, a=a, b=b))
    return out, moved


def dedupe(segments: Sequence[Segment]) -> tuple[list[Segment], int]:
    """Remove exact duplicates, ignoring direction.

    Only meaningful after snapping. Before it, "duplicates" differ in the ninth decimal
    place and this does nothing.
    """
    seen: dict[Segment, Segment] = {}
    for s in segments:
        seen.setdefault(s.canonical(), s)
    out = sorted(seen.values())
    return out, len(segments) - len(out)


def _line_key(seg: Segment, angle_tol: float) -> tuple[float, float, Point]:
    """Return ``(heading, signed offset, unit normal)`` describing the segment's line.

    Headings live in ``[0, pi)``. Near-pi headings are shifted to just below zero so a
    line at 179.5 deg and one at 0.5 deg sort next to each other — without that, the
    wrap-around splits every near-horizontal wall into two collinear groups that never
    merge.
    """
    th = seg.angle
    if th > math.pi - angle_tol:
        th -= math.pi
    n = (-math.sin(th), math.cos(th))
    return th, dot(n, seg.a), n


def _collinear_groups(
    segments: Sequence[Segment], opts: RepairOptions
) -> list[list[Segment]]:
    """Partition segments into groups sharing a line, within the angle/offset tolerance.

    Greedy over a canonical sort, which makes it order-independent. A segment joins the
    open group whose representative it matches; otherwise it starts a new one.
    """
    keyed = sorted(
        ((_line_key(s, opts.angle_tol_rad), s) for s in segments),
        key=lambda kv: (kv[0][0], kv[0][1], kv[1]),
    )
    groups: list[list[Segment]] = []
    rep: tuple[float, float] | None = None
    for (th, off, _n), seg in keyed:
        if (
            rep is not None
            and angle_between(th, rep[0]) <= opts.angle_tol_rad
            and abs(off - rep[1]) <= opts.eps_m
        ):
            groups[-1].append(seg)
        else:
            groups.append([seg])
            rep = (th, off)
    return groups


def merge_collinear(
    segments: Sequence[Segment],
    opts: RepairOptions,
    *,
    bridge_openings: bool,
) -> tuple[list[Segment], list[OpeningCandidate], dict[Segment, list[Segment]], int]:
    """Collapse overlapping collinear segments into single runs.

    Returns the merged segments, any openings bridged, a map from each output segment to
    the inputs that produced it (so a caller can carry per-segment attributes such as
    thickness through the merge), and the number of segments eliminated.

    When *bridge_openings* is set, a hole between two runs of the same line whose width
    falls in ``[gap_min_m, gap_max_m]`` is treated as a doorway: the runs are joined
    across it and an :class:`OpeningCandidate` is recorded at its centre. This is the
    right model — a doorway is a hole *in* a wall, and the wall continues past it — and
    it is also what makes the schema's parametric `(wall, t)` opening expressible at all.
    A gap between two genuinely separate walls has no `t` to be at.
    """
    out: list[Segment] = []
    openings: list[OpeningCandidate] = []
    provenance: dict[Segment, list[Segment]] = {}
    removed = 0

    for group in _collinear_groups(segments, opts):
        anchor = min(group)
        a, b = anchor.a, anchor.b
        u = anchor.direction
        # Project every endpoint onto the group's line, in metres along it.
        spans: list[tuple[float, float, Segment]] = []
        for s in group:
            ta = dot(sub(s.a, a), u)
            tb = dot(sub(s.b, a), u)
            spans.append((min(ta, tb), max(ta, tb), s))
        spans.sort(key=lambda sp: (sp[0], sp[1], sp[2]))

        runs: list[tuple[float, float, list[Segment]]] = []
        for lo, hi, s in spans:
            if runs and lo <= runs[-1][1] + opts.eps_m:
                prev_lo, prev_hi, members = runs[-1]
                runs[-1] = (prev_lo, max(prev_hi, hi), [*members, s])
            else:
                runs.append((lo, hi, [s]))

        if bridge_openings and len(runs) > 1:
            joined: list[tuple[float, float, list[Segment]]] = [runs[0]]
            for lo, hi, members in runs[1:]:
                gap = lo - joined[-1][1]
                if opts.gap_min_m <= gap <= opts.gap_max_m:
                    mid = joined[-1][1] + gap * 0.5
                    centre = (a[0] + u[0] * mid, a[1] + u[1] * mid)
                    openings.append(
                        OpeningCandidate(
                            centre=centre,
                            width_m=gap,
                            direction=u,
                            origin="gap",
                            confidence=0.65,
                        )
                    )
                    prev_lo, prev_hi, prev_members = joined[-1]
                    joined[-1] = (prev_lo, max(prev_hi, hi), [*prev_members, *members])
                else:
                    joined.append((lo, hi, members))
            runs = joined

        for lo, hi, members in runs:
            seg = Segment(
                (a[0] + u[0] * lo, a[1] + u[1] * lo),
                (a[0] + u[0] * hi, a[1] + u[1] * hi),
                anchor.layer,
            ).canonical()
            out.append(seg)
            provenance.setdefault(seg, []).extend(members)
            removed += len(members) - 1

    out.sort()
    return out, openings, provenance, removed


def pair_parallel_walls(
    segments: Sequence[Segment], opts: RepairOptions
) -> tuple[list[Segment], dict[Segment, float], int]:
    """Fold each pair of parallel wall faces into one centreline with a thickness.

    CAD draws a wall as its two faces. Imported literally, a 230 mm wall becomes two
    obstacles with a 230 mm slot between them — geometrically defensible, useless as a
    model, and it triples the navmesh's triangle count for no information.

    Pairing is greedy over candidates sorted by separation (closest faces pair first),
    which is order-independent and matches the intuition that the nearest parallel line
    is the other face of the same wall. Faces must overlap along their shared direction
    by :attr:`RepairOptions.pair_overlap_frac` before we believe it — two parallel walls
    at opposite ends of a corridor are parallel and near, and are not one wall.

    Returns the resulting segments, the thickness measured for each, and the pair count.
    """
    thickness: dict[Segment, float] = {}
    if not opts.pair_parallel or len(segments) < 2:
        return list(segments), thickness, 0

    candidates: list[tuple[float, float, Segment, Segment]] = []
    ordered = sorted(segments)
    for i, s in enumerate(ordered):
        for t in ordered[i + 1 :]:
            if angle_between(s.angle, t.angle) > opts.angle_tol_rad:
                continue
            sep = abs(perpendicular_offset(t.midpoint, s.a, s.b))
            if not (opts.thickness_min_m <= sep <= opts.thickness_max_m):
                continue
            # Overlap along s's direction.
            u = s.direction
            s0, s1 = 0.0, s.length
            t0 = dot(sub(t.a, s.a), u)
            t1 = dot(sub(t.b, s.a), u)
            lo, hi = min(t0, t1), max(t0, t1)
            overlap = min(s1, hi) - max(s0, lo)
            shorter = min(s.length, t.length)
            if shorter <= 0.0 or overlap / shorter < opts.pair_overlap_frac:
                continue
            candidates.append((sep, -overlap, s, t))

    candidates.sort(key=lambda c: (c[0], c[1], c[2], c[3]))
    used: set[Segment] = set()
    out: list[Segment] = []
    pairs = 0
    for sep, _negoverlap, s, t in candidates:
        if s in used or t in used:
            continue
        used.add(s)
        used.add(t)
        pairs += 1
        # Centreline: midpoints of the two faces' overlapping extent, on s's line,
        # offset half the separation toward t.
        u = s.direction
        n = (-u[1], u[0])
        side = math.copysign(1.0, perpendicular_offset(t.midpoint, s.a, s.b))
        shift = (n[0] * side * sep * 0.5, n[1] * side * sep * 0.5)
        t0 = dot(sub(t.a, s.a), u)
        t1 = dot(sub(t.b, s.a), u)
        lo = min(0.0, t0, t1)
        hi = max(s.length, t0, t1)
        a = (s.a[0] + u[0] * lo + shift[0], s.a[1] + u[1] * lo + shift[1])
        b = (s.a[0] + u[0] * hi + shift[0], s.a[1] + u[1] * hi + shift[1])
        centre = Segment(a, b, s.layer).canonical()
        out.append(centre)
        thickness[centre] = sep

    out.extend(s for s in ordered if s not in used)
    out.sort()
    return out, thickness, pairs


def close_junctions(
    segments: Sequence[Segment], opts: RepairOptions
) -> tuple[list[Segment], int]:
    """Extend dangling wall ends to meet the neighbour they were meant to touch.

    Endpoint snapping already fused ends within `eps_m`. This handles the next size up:
    a corner that misses by 120 mm because the two walls were drawn in different
    sessions, or a polyline the drafter closed by eye. Left alone, such a corner is a
    hairline crack that the navmesh compiler will happily route a crowd through, and
    the resulting egress numbers are fiction.

    Only ends with degree 1 are considered, only extension along the segment's own
    direction is allowed, and only up to `junction_extend_m` — so this can lengthen a
    wall by a hand's breadth, never invent one. The bound is an order of magnitude below
    `gap_min_m`, which is what stops it quietly welding a doorway shut.
    """
    if not segments:
        return [], 0

    degree: dict[Point, int] = {}
    for s in segments:
        degree[s.a] = degree.get(s.a, 0) + 1
        degree[s.b] = degree.get(s.b, 0) + 1

    out = list(segments)
    closed = 0
    # Visited in canonical segment order, not list order, so the result does not
    # depend on how entities happened to appear in the file. `si` is an index —
    # named apart from the `s` above, which is a Segment, because one letter
    # meaning two things in one function is how this got mistyped.
    for si in sorted(range(len(out)), key=lambda k: out[k]):
        seg = out[si]
        for which in ("a", "b"):
            end: Point = getattr(seg, which)
            if degree.get(end, 0) != 1:
                continue
            other: Point = seg.b if which == "a" else seg.a
            best: tuple[float, Point] | None = None
            for j, cand in enumerate(out):
                if cand is seg or (j == si):
                    continue
                if end in (cand.a, cand.b):
                    continue
                hit = line_intersection(seg.a, seg.b, cand.a, cand.b)
                if hit is None:
                    continue
                # The hit must lie beyond the dangling end, within reach, and on the
                # other segment's actual extent (not its infinite line).
                if dot(sub(hit, end), sub(end, other)) <= 0.0:
                    continue
                reach = distance(end, hit)
                if reach > opts.junction_extend_m:
                    continue
                tc = project_param(hit, cand.a, cand.b)
                if not (-1e-9 <= tc <= 1.0 + 1e-9):
                    continue
                if best is None or reach < best[0]:
                    best = (reach, hit)
            if best is not None:
                # Explicit rather than `replace(seg, **{which: ...})`: the
                # dynamic form defeats the type checker, and `which` only ever
                # takes two values.
                seg = (
                    replace(seg, a=best[1])
                    if which == "a"
                    else replace(seg, b=best[1])
                )
                out[si] = seg
                closed += 1
    return sorted(out), closed


def chain(segments: Sequence[Segment], opts: RepairOptions) -> list[Polyline]:
    """Join segments end-to-end into polylines, splitting at junctions.

    A vertex where three walls meet ends all three polylines rather than picking one to
    continue — an arbitrary choice there would make the wall list depend on iteration
    order, and would attach a door to whichever wall happened to win.

    Collinear interior vertices are dropped: a wall traced in eleven pieces should be
    one two-point wall in the document, not an eleven-point one, because every vertex is
    something a user has to drag if they want to move it.
    """
    adjacency: dict[Point, list[Point]] = {}
    for s in segments:
        adjacency.setdefault(s.a, []).append(s.b)
        adjacency.setdefault(s.b, []).append(s.a)
    for neighbours in adjacency.values():
        neighbours.sort()

    layer_of: dict[frozenset[Point], str] = {
        frozenset((s.a, s.b)): s.layer for s in sorted(segments)
    }
    unused: set[frozenset[Point]] = {frozenset((s.a, s.b)) for s in segments}
    polylines: list[Polyline] = []

    def walk(start: Point, first: Point) -> list[Point]:
        pts = [start, first]
        prev, cur = start, first
        while True:
            edge = frozenset((prev, cur))
            unused.discard(edge)
            nbrs = [n for n in adjacency[cur] if frozenset((cur, n)) in unused]
            if len(adjacency[cur]) != 2 or len(nbrs) != 1:
                break
            nxt = nbrs[0]
            pts.append(nxt)
            prev, cur = cur, nxt
            if cur == start:
                break
        return pts

    endpoints = sorted(p for p, n in adjacency.items() if len(n) != 2)
    for start in endpoints:
        for first in list(adjacency[start]):
            if frozenset((start, first)) not in unused:
                continue
            pts = walk(start, first)
            polylines.append(_finish(pts, layer_of[frozenset((start, first))], opts))

    while unused:
        edge = min(unused, key=lambda e: sorted(e))
        a, b = sorted(edge)
        pts = walk(a, b)
        polylines.append(_finish(pts, layer_of[edge], opts))

    polylines.sort(key=lambda p: p.points)
    return polylines


def _finish(points: list[Point], layer: str, opts: RepairOptions) -> Polyline:
    """Close a walked run if it returns to its start, then drop collinear vertices."""
    closed = len(points) > 3 and points[0] == points[-1]
    if closed:
        points = points[:-1]
    return Polyline(tuple(_simplify(points, opts, closed=closed)), layer, closed)


def _simplify(points: list[Point], opts: RepairOptions, *, closed: bool) -> list[Point]:
    """Drop vertices whose two neighbours are collinear with them."""
    if len(points) < 3:
        return points
    keep: list[Point] = []
    n = len(points)
    span = range(n) if closed else range(1, n - 1)
    if not closed:
        keep.append(points[0])
    for i in span:
        prev = points[(i - 1) % n]
        cur = points[i]
        nxt = points[(i + 1) % n]
        if abs(perpendicular_offset(cur, prev, nxt)) > opts.eps_m:
            keep.append(cur)
    if not closed:
        keep.append(points[-1])
    return keep if len(keep) >= (3 if closed else 2) else points


def door_openings(
    door_segments: Sequence[Segment], opts: RepairOptions
) -> list[OpeningCandidate]:
    """Turn door-layer geometry into opening candidates.

    A door on a door layer is a leaf, a swing arc, a frame, or all three. Rather than
    trying to recognise the symbol, each connected cluster of door geometry is reduced
    to its widest extent — which for a leaf-plus-arc is the opening width, because the
    leaf and the arc both span exactly the opening. The direction that maximises the
    extent is taken as the wall direction; :mod:`importer.emit` then finds the wall it
    belongs to by proximity, so a slight disagreement costs nothing.

    Candidates outside ``[gap_min_m, gap_max_m]`` are dropped with a warning rather than
    emitted: a 6 m "door" is a mis-mapped layer, and importing it as an opening would
    put a hole the size of a truck in a fire compartment.
    """
    out: list[OpeningCandidate] = []
    for cluster in _cluster_segments(door_segments, opts.eps_m * 4.0):
        pts = sorted({p for s in cluster for p in (s.a, s.b)})
        if len(pts) < 2:
            continue
        # The opening width is the **leaf length**, which is the longest single
        # straight segment in the cluster.
        #
        # Not the widest extent across the cluster, which was the first attempt
        # and is wrong for the way doors are actually drawn. CAD shows a door
        # swung open at 90 degrees: a leaf perpendicular to the wall plus an arc
        # from closed to open. The widest extent is then hinge-to-leaf-tip
        # *diagonally* — 1.56 m for a 1.1 m door, comfortably inside the
        # plausible range and comfortably wrong.
        #
        # The leaf and the arc radius both equal the opening. The arc arrives
        # here already flattened into many short segments, so the longest single
        # segment is the leaf.
        longest = max(cluster, key=lambda seg: (seg.length, seg.a, seg.b))
        leaf = longest.length
        far = max(
            ((distance(p, q), p, q) for i, p in enumerate(pts) for q in pts[i + 1 :]),
            key=lambda t: (t[0], t[1], t[2]),
        )
        # Fall back to the extent for a door drawn as a plain gap-spanning line,
        # where there is no leaf to be longer than everything else.
        width, p, q = (leaf, longest.a, longest.b) if opts.gap_min_m <= leaf <= opts.gap_max_m else far
        if not (opts.gap_min_m <= width <= opts.gap_max_m):
            continue
        d = distance(p, q)
        u = ((q[0] - p[0]) / d, (q[1] - p[1]) / d)
        out.append(
            OpeningCandidate(
                centre=lerp(p, q, 0.5),
                width_m=width,
                direction=u,
                origin="door",
                confidence=0.9,
            )
        )
    out.sort(key=lambda o: (o.centre, o.width_m))
    return out


def _cluster_segments(segments: Sequence[Segment], eps: float) -> list[list[Segment]]:
    """Group segments into connected components, treating endpoints within *eps* as joined."""
    ordered = sorted(segments)
    parent = list(range(len(ordered)))

    def find(i: int) -> int:
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i

    for i, s in enumerate(ordered):
        for j in range(i + 1, len(ordered)):
            t = ordered[j]
            if any(distance(p, q) <= eps for p in (s.a, s.b) for q in (t.a, t.b)):
                ri, rj = find(i), find(j)
                if ri != rj:
                    parent[max(ri, rj)] = min(ri, rj)

    groups: dict[int, list[Segment]] = {}
    for i, s in enumerate(ordered):
        groups.setdefault(find(i), []).append(s)
    return [groups[k] for k in sorted(groups)]


# --------------------------------------------------------------------------- driver


@dataclass(slots=True)
class RepairedGeometry:
    """Everything repair produced, ready for :mod:`importer.emit`."""

    walls: list[WallRun]
    openings: list[OpeningCandidate]
    outlines: list[Polyline]
    obstacles: list[Polyline]
    report: RepairReport


def repair_walls(
    wall_segments: Sequence[Segment], opts: RepairOptions, report: RepairReport
) -> tuple[list[WallRun], list[OpeningCandidate]]:
    """Run the full wall repair sequence and return walls plus inferred openings.

    Order matters and is not arbitrary:

    1. **drop short** first, so degenerate segments never join a collinear group.
    2. **snap**, so ``==`` means "same point" for everything after.
    3. **dedupe**, which only works once snapping has made duplicates identical.
    4. **merge collinear** without bridging — collapse overlapping traces of one wall.
    5. **close junctions** on the cleaned-up runs, not on the raw ones.
    6. **snap and dedupe again**, because closing junctions moved endpoints.
    7. **pair parallel faces** into centrelines, which is when thickness is measured.
    8. **merge collinear again, bridging doorways** — done last because a double-line
       wall shows its doorway on both faces, and bridging before pairing would find the
       same door twice and put two openings in one wall.
    9. **chain** into polylines.
    """
    report.input_segments = len(wall_segments)
    segs, dropped = drop_short(wall_segments, opts.min_segment_m)
    report.dropped_short += dropped

    segs, moved = snap_endpoints(segs, opts.eps_m)
    report.snapped_endpoints += moved

    segs, dupes = dedupe(segs)
    report.removed_duplicates += dupes

    segs, _, _, merged = merge_collinear(segs, opts, bridge_openings=False)
    report.merged_collinear += merged

    segs, closed = close_junctions(segs, opts)
    report.closed_junctions += closed

    segs, moved = snap_endpoints(segs, opts.eps_m)
    report.snapped_endpoints += moved
    segs, dupes = dedupe(segs)
    report.removed_duplicates += dupes

    segs, thickness, pairs = pair_parallel_walls(segs, opts)
    report.paired_parallel += pairs

    segs, openings, provenance, merged = merge_collinear(segs, opts, bridge_openings=True)
    report.merged_collinear += merged
    report.bridged_openings += len(openings)

    # Carry measured thickness through the final merge.
    merged_thickness: dict[Segment, float] = {}
    for out_seg, sources in provenance.items():
        measured = sorted(thickness[s] for s in sources if s in thickness)
        if measured:
            merged_thickness[out_seg] = measured[len(measured) // 2]

    polylines = chain(segs, opts)
    walls: list[WallRun] = []
    for pl in polylines:
        measured = sorted(
            merged_thickness[s.canonical()]
            for s in pl.segments()
            if s.canonical() in merged_thickness
        )
        if measured:
            walls.append(WallRun(pl, measured[len(measured) // 2], confidence=1.0))
        else:
            walls.append(WallRun(pl, opts.default_thickness_m, confidence=0.7))
    report.output_walls += len(walls)
    return walls, openings


def close_loops(polylines: Sequence[Polyline], opts: RepairOptions) -> list[Polyline]:
    """Mark an almost-closed polyline as closed.

    An outline drawn as a polyline whose last vertex misses the first by 8 mm is the
    single most common defect in a real floorplan, and an unclosed outline produces no
    room at all — the failure is total and silent. The tolerance is deliberately looser
    than `eps_m`: this is the one place where being wrong costs a duplicated vertex
    rather than a fabricated wall.
    """
    out: list[Polyline] = []
    tol = opts.eps_m * 4.0
    for pl in polylines:
        if pl.closed or len(pl.points) < 3:
            out.append(pl)
            continue
        if distance(pl.points[0], pl.points[-1]) <= tol:
            pts = pl.points[:-1] if distance(pl.points[0], pl.points[-1]) > 0 else pl.points
            out.append(Polyline(pts, pl.layer, closed=True))
        else:
            out.append(pl)
    return out
