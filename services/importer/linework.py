"""The intermediate representation every reader produces and every later stage consumes.

Both readers (DXF and vector PDF) flatten their very different source models down to one
thing: a flat bag of straight :class:`Segment`\\ s, each tagged with the *source layer* it
came from. Arcs, splines, block references, rectangles and bezier path segments are all
gone by this point — flattened at a chord tolerance chosen by the reader.

Keeping the IR this small is what lets topology repair, layer mapping, calibration and
emission be written once instead of once per format. It is also why adding an SVG reader
later is a day's work: it only has to produce segments.

Everything is a frozen dataclass with tuple coordinates so segments are hashable and
sortable, which is how the repair stages stay deterministic without relying on dict
ordering (rule R2's spirit, applied to Python).
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field, replace
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import Iterable, Sequence

#: A point in whatever units the containing :class:`LineWork` is expressed in.
Point = tuple[float, float]


@dataclass(frozen=True, slots=True, order=True)
class Segment:
    """A straight run of line work, in source drawing units.

    Ordered and hashable so collections of segments can be sorted into a canonical
    order before any grouping step. Two segments that differ only in direction are
    *not* equal; use :meth:`canonical` when direction is irrelevant.
    """

    a: Point
    b: Point
    layer: str = "0"

    @property
    def length(self) -> float:
        """Euclidean length, in source units."""
        return math.hypot(self.b[0] - self.a[0], self.b[1] - self.a[1])

    @property
    def midpoint(self) -> Point:
        """Midpoint, in source units."""
        return ((self.a[0] + self.b[0]) * 0.5, (self.a[1] + self.b[1]) * 0.5)

    @property
    def direction(self) -> Point:
        """Unit vector from ``a`` to ``b``. ``(0.0, 0.0)`` for a degenerate segment."""
        dx, dy = self.b[0] - self.a[0], self.b[1] - self.a[1]
        n = math.hypot(dx, dy)
        if n == 0.0:
            return (0.0, 0.0)
        return (dx / n, dy / n)

    @property
    def angle(self) -> float:
        """Heading in radians, in ``[0, pi)`` — direction-agnostic, for collinear tests."""
        dx, dy = self.b[0] - self.a[0], self.b[1] - self.a[1]
        return math.atan2(dy, dx) % math.pi

    def canonical(self) -> Segment:
        """Return this segment with endpoints in sorted order.

        Duplicate detection has to see ``A->B`` and ``B->A`` as the same wall. Drawings
        contain both constantly — a wall traced once clockwise and once anticlockwise by
        two different people is the single most common form of duplicate geometry.
        """
        return self if self.a <= self.b else replace(self, a=self.b, b=self.a)

    def scaled(self, factor: float) -> Segment:
        """Return this segment with both endpoints multiplied by *factor*."""
        return replace(
            self,
            a=(self.a[0] * factor, self.a[1] * factor),
            b=(self.b[0] * factor, self.b[1] * factor),
        )


@dataclass(frozen=True, slots=True)
class Polyline:
    """A chained run of segments, produced by topology repair.

    ``closed`` means the last point joins the first; the point list does **not** repeat
    the first point, matching how the Venue schema stores polygons.
    """

    points: tuple[Point, ...]
    layer: str = "0"
    closed: bool = False

    @property
    def length(self) -> float:
        """Total arc length."""
        pts = list(self.points) + ([self.points[0]] if self.closed else [])
        return sum(
            math.hypot(q[0] - p[0], q[1] - p[1]) for p, q in zip(pts, pts[1:], strict=False)
        )

    def segments(self) -> list[Segment]:
        """Explode back into segments, preserving the layer tag."""
        pts = list(self.points) + ([self.points[0]] if self.closed else [])
        return [Segment(p, q, self.layer) for p, q in zip(pts, pts[1:], strict=False)]

    def signed_area(self) -> float:
        """Shoelace area; positive when the ring winds anticlockwise.

        Only meaningful for a closed polyline. Used to give every emitted zone polygon
        a consistent winding, because downstream point-in-polygon and triangulation
        code should not have to care which way an architect drew a room.
        """
        pts = self.points
        return 0.5 * sum(
            pts[i][0] * pts[(i + 1) % len(pts)][1] - pts[(i + 1) % len(pts)][0] * pts[i][1]
            for i in range(len(pts))
        )


@dataclass(slots=True)
class LineWork:
    """Everything a reader extracted from one source file, plus what the file claimed.

    ``unit_metres`` is the file's *own* statement about its units (DXF ``$INSUNITS``,
    or PDF points). It is a claim, not a decision — :mod:`importer.calibration` decides,
    and it is allowed to overrule the file.
    """

    segments: list[Segment]
    source: str
    #: Metres per source unit as declared by the file, or ``None`` if it declined to say.
    unit_metres: float | None = None
    #: How the file expressed that claim, for the confirmation dialog ("feet", "unitless").
    unit_note: str = "unknown"
    #: Non-fatal problems noticed while reading (skipped entity types, unresolved xrefs).
    warnings: list[str] = field(default_factory=list)

    def layers(self) -> list[str]:
        """Distinct source layer names, sorted."""
        return sorted({s.layer for s in self.segments})

    def bounds(self) -> tuple[float, float, float, float]:
        """``(min_x, min_y, max_x, max_y)`` over all segments, in source units."""
        if not self.segments:
            return (0.0, 0.0, 0.0, 0.0)
        xs = [c for s in self.segments for c in (s.a[0], s.b[0])]
        ys = [c for s in self.segments for c in (s.a[1], s.b[1])]
        return (min(xs), min(ys), max(xs), max(ys))

    def scaled(self, factor: float) -> LineWork:
        """Return a copy with every coordinate multiplied by *factor*."""
        return LineWork(
            segments=[s.scaled(factor) for s in self.segments],
            source=self.source,
            unit_metres=1.0 if self.unit_metres is not None else None,
            unit_note=self.unit_note,
            warnings=list(self.warnings),
        )


def flatten(points: Sequence[Point], layer: str, *, closed: bool = False) -> list[Segment]:
    """Turn an ordered point run into segments, dropping repeated points.

    Readers call this for every entity. Zero-length segments are dropped here rather
    than in repair because a curve flattener emits them routinely at a tangent
    discontinuity, and they carry no information at any later stage.
    """
    out: list[Segment] = []
    pts = list(points)
    if closed and len(pts) > 2 and pts[0] != pts[-1]:
        pts.append(pts[0])
    for p, q in zip(pts, pts[1:], strict=False):
        if p != q:
            out.append(Segment(p, q, layer))
    return out


def total_length(segments: Iterable[Segment]) -> float:
    """Sum of segment lengths — the cheapest useful "how much drawing is this" metric."""
    return sum(s.length for s in segments)
