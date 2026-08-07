"""Plane geometry primitives used by topology repair.

Small, pure, and free of any notion of walls or doors — that vocabulary belongs in
:mod:`importer.topology`. Keeping the two apart is what makes the repair steps testable
one at a time, which matters because every one of them is a tolerance decision that will
be argued about later.

No numpy. The data is a few thousand segments and the pipeline runs once per upload;
a dependency and a float-ordering surprise both cost more than the microseconds saved.
"""

from __future__ import annotations

import math
from typing import TYPE_CHECKING

from importer.linework import Point

if TYPE_CHECKING:
    from collections.abc import Iterable, Sequence


def distance(p: Point, q: Point) -> float:
    """Euclidean distance."""
    return math.hypot(q[0] - p[0], q[1] - p[1])


def sub(p: Point, q: Point) -> Point:
    """``p - q``."""
    return (p[0] - q[0], p[1] - q[1])


def dot(p: Point, q: Point) -> float:
    """Dot product."""
    return p[0] * q[0] + p[1] * q[1]


def cross(p: Point, q: Point) -> float:
    """2D cross product (z component)."""
    return p[0] * q[1] - p[1] * q[0]


def lerp(p: Point, q: Point, t: float) -> Point:
    """Point at parameter *t* along ``p -> q``."""
    return (p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t)


def project_param(p: Point, a: Point, b: Point) -> float:
    """Parameter of *p* projected onto the infinite line through ``a, b``.

    Not clamped — callers that need a point *on the segment* clamp themselves, and
    callers doing collinear-interval arithmetic need the unclamped value.
    Returns 0.0 for a degenerate segment.
    """
    ab = sub(b, a)
    denom = dot(ab, ab)
    if denom == 0.0:
        return 0.0
    return dot(sub(p, a), ab) / denom


def point_segment_distance(p: Point, a: Point, b: Point) -> float:
    """Distance from *p* to the closest point of segment ``a-b``."""
    t = max(0.0, min(1.0, project_param(p, a, b)))
    return distance(p, lerp(a, b, t))


def perpendicular_offset(p: Point, a: Point, b: Point) -> float:
    """Signed distance from *p* to the infinite line through ``a, b``.

    Sign is positive to the left of ``a -> b``. Used to tell a wall's two faces apart
    when pairing parallel lines into a single thick wall.
    """
    ab = sub(b, a)
    n = math.hypot(*ab)
    if n == 0.0:
        return distance(p, a)
    return cross(ab, sub(p, a)) / n


def angle_between(a: float, b: float) -> float:
    """Smallest absolute difference between two undirected headings, in ``[0, pi/2]``.

    Both arguments are headings modulo pi (see :attr:`Segment.angle`), so a line at
    179 deg and one at 1 deg differ by 2 deg, not 178.
    """
    d = abs(a - b) % math.pi
    return min(d, math.pi - d)


def line_intersection(
    a1: Point, a2: Point, b1: Point, b2: Point
) -> Point | None:
    """Intersection of two *infinite* lines, or ``None`` if parallel.

    Junction closing extends a dangling wall end to meet its neighbour, which needs the
    intersection of the lines, not of the segments — the whole point is that the
    segments do not currently reach each other.
    """
    d1 = sub(a2, a1)
    d2 = sub(b2, b1)
    denom = cross(d1, d2)
    if abs(denom) < 1e-12:
        return None
    t = cross(sub(b1, a1), d2) / denom
    return (a1[0] + d1[0] * t, a1[1] + d1[1] * t)


def polygon_area(points: Sequence[Point]) -> float:
    """Absolute shoelace area of a closed ring (first point not repeated)."""
    n = len(points)
    if n < 3:
        return 0.0
    s = sum(
        points[i][0] * points[(i + 1) % n][1] - points[(i + 1) % n][0] * points[i][1]
        for i in range(n)
    )
    return abs(s) * 0.5


def bounding_box(points: Iterable[Point]) -> tuple[float, float, float, float]:
    """``(min_x, min_y, max_x, max_y)``. Zeros for an empty input."""
    pts = list(points)
    if not pts:
        return (0.0, 0.0, 0.0, 0.0)
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    return (min(xs), min(ys), max(xs), max(ys))


def quantise(p: Point, cell: float) -> tuple[int, int]:
    """Grid cell index for *p*.

    The workhorse of endpoint clustering. A uniform grid is used rather than a kd-tree
    because it is order-independent by construction: bucket membership depends only on
    coordinates, never on insertion order, so the clustering result cannot drift when
    the reader emits entities in a different order.
    """
    return (math.floor(p[0] / cell), math.floor(p[1] / cell))


def cluster_points(points: Sequence[Point], eps: float) -> dict[Point, Point]:
    """Map each input point to the centroid of its cluster.

    Points within *eps* of a shared representative collapse together. Implemented as a
    single pass over points sorted lexicographically, against a grid index of already
    placed representatives — which makes the output a pure function of the point *set*,
    not of the order it arrived in.

    Two points slightly more than *eps* apart can still end up in one cluster via a
    chain of intermediates. That is intended: a wall traced in six overlapping pieces
    produces exactly such a chain, and splitting it would leave a hairline crack that
    the navmesh compiler would happily route a crowd through.
    """
    if eps <= 0.0:
        return {p: p for p in points}

    cell = eps
    grid: dict[tuple[int, int], list[int]] = {}
    reps: list[list[Point]] = []  # cluster index -> member points
    assignment: dict[Point, int] = {}

    for p in sorted(set(points)):
        gx, gy = quantise(p, cell)
        found: int | None = None
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                for idx in grid.get((gx + dx, gy + dy), ()):
                    if any(distance(p, m) <= eps for m in reps[idx]):
                        found = idx
                        break
                if found is not None:
                    break
            if found is not None:
                break
        if found is None:
            found = len(reps)
            reps.append([])
        reps[found].append(p)
        grid.setdefault((gx, gy), []).append(found)
        assignment[p] = found

    centroids = [
        (sum(m[0] for m in members) / len(members), sum(m[1] for m in members) / len(members))
        for members in reps
    ]
    return {p: centroids[idx] for p, idx in assignment.items()}


def merge_intervals(
    intervals: Sequence[tuple[float, float]], gap_tol: float
) -> list[tuple[float, float]]:
    """Union of 1D intervals, joining any pair separated by at most *gap_tol*.

    Collinear overlapping segments — the classic result of tracing the same wall twice —
    reduce to one interval per run. The *gaps that remain* after this are the interesting
    ones: they are candidate doorways.
    """
    if not intervals:
        return []
    ordered = sorted((min(lo, hi), max(lo, hi)) for lo, hi in intervals)
    out: list[tuple[float, float]] = [ordered[0]]
    for lo, hi in ordered[1:]:
        last_lo, last_hi = out[-1]
        if lo <= last_hi + gap_tol:
            out[-1] = (last_lo, max(last_hi, hi))
        else:
            out.append((lo, hi))
    return out
