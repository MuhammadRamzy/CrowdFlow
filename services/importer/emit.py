"""Turn repaired geometry into a Venue document.

Two things happen here that are easy to get wrong and expensive to notice late.

**Scaling happens exactly once.** Everything upstream is in drawing units;
everything downstream is in metres. Doing it here, in one place, means there is
no module that might or might not have converted — and no possibility of a
double conversion, which produces a venue a thousand times too small and looks
like a geometry bug for a day.

**Openings become parametric.** The schema stores a door as `(wall, t)` with `t`
a normalised distance along the wall, not as a point in space. That is what
makes moving a wall carry its doors with it, and it means an opening whose
parent wall cannot be identified has nowhere to live — such an opening is
dropped with a warning rather than attached to whichever wall happens to be
nearest, which would put a door in the wrong room.
"""

from __future__ import annotations

from importer.calibration import Scale
from importer.geometry import distance, point_segment_distance
from importer.linework import Point
from importer.schema.venue import Floor, Opening, Provenance, Units, Vec2, VenueDoc, Wall
from importer.topology import OpeningCandidate, WallRun

#: How far an opening may sit from a wall and still be considered to be in it.
#:
#: Generous, because a door block is drawn on its own layer and rarely lands
#: exactly on the wall centreline — but finite, so a door in the middle of a
#: room is reported rather than dragged to an unrelated wall.
_ATTACH_TOLERANCE_M = 0.75


def to_venue(
    runs: list[WallRun],
    openings: list[OpeningCandidate],
    scale: Scale,
    *,
    venue_id: str,
    name: str,
    source: str,
    warnings: list[str],
) -> VenueDoc:
    """Assemble a draft venue. Appends to `warnings` rather than raising.

    Geometry arrives **already in metres** — `pipeline` converts once, straight
    after calibration, because repair's tolerances are metric and meaningless
    against raw drawing units. `scale` is carried here only for provenance.
    """

    walls: list[Wall] = []
    # Metre-space polylines, kept alongside so openings can be attached without
    # scaling a second time.
    wall_points: list[list[Point]] = []

    for i, run in enumerate(runs):
        pts = [(p[0], p[1]) for p in run.polyline.points]
        if len(pts) < 2:
            continue
        wall_points.append(pts)
        walls.append(
            Wall(
                id=f"w_{i:04d}",
                polyline=[Vec2([round(x, 4), round(y, 4)]) for x, y in pts],
                thicknessM=round(run.thickness_m, 4),
                kind="structural",
                layer=run.polyline.layer,
                provenance=_provenance(source, run.confidence),
            )
        )

    schema_openings: list[Opening] = []
    unattached = 0
    for j, cand in enumerate(openings):
        centre = (cand.centre[0], cand.centre[1])
        found = _attach(centre, wall_points)
        if found is None:
            unattached += 1
            continue
        wall_index, t = found
        width = cand.width_m if cand.width_m > 0 else 0.9
        schema_openings.append(
            Opening(
                id=f"op_{j:04d}",
                wall=walls[wall_index].id,
                t=round(t, 6),
                widthM=round(width, 4),
                kind="door",
                # Nothing in a drawing says which doors are fire exits. Guessing
                # would put a false exit into a life-safety calculation, so this
                # is left for a person and the report says the floor has none.
                isFireExit=False,
                provenance=_provenance(source, cand.confidence),
            )
        )

    if unattached:
        warnings.append(
            f"{unattached} opening(s) were more than {_ATTACH_TOLERANCE_M} m from any "
            "wall and were dropped — check the door layer mapping"
        )
    if walls and not schema_openings:
        warnings.append(
            "no openings were found, so the venue has no way out and cannot be "
            "simulated until doors are drawn"
        )

    return VenueDoc(
        schemaVersion="cfs.venue/1.0",
        id=venue_id,
        name=name,
        units=Units(length="m"),
        floors=[
            Floor(
                id="f0",
                name="Ground",
                elevationM=0.0,
                walls=walls,
                openings=schema_openings,
                zones=[],
            )
        ],
        provenance=_provenance(source, scale.confidence),
    )


def _provenance(source: str, confidence: float) -> Provenance:
    """Where an element came from and how much to trust it.

    Carried per element, not per document. A review UI bands elements by
    confidence so a user can accept the crisp 90% and redraw the rest, which is
    only possible if the number travels with the geometry.
    """
    return Provenance(
        source="import",
        # The schema calls this `sourceFile`. It was `detail` here for a while
        # and Pydantic dropped it without a word — every imported venue lost the
        # name of the drawing it came from, and fifteen tests were happy. mypy
        # found it; nothing else would have until someone asked a report where
        # a wall came from.
        sourceFile=source,
        confidence=round(max(0.0, min(1.0, confidence)), 3),
    )


def _attach(
    centre: Point, wall_points: list[list[Point]]
) -> tuple[int, float] | None:
    """Find the wall an opening belongs to, and where along it.

    Returns `(wall index, t)` with `t` normalised by arc length — the schema's
    form — or None when nothing is close enough.
    """
    best: tuple[float, int, float] | None = None

    for wi, pts in enumerate(wall_points):
        # Arc length up to each vertex, so `t` is distance along the wall rather
        # than along the straight line between its ends. For an L-shaped wall
        # those differ substantially and only one of them puts the door in the
        # right place.
        cumulative = [0.0]
        for a, b in zip(pts, pts[1:]):
            cumulative.append(cumulative[-1] + distance(a, b))
        total = cumulative[-1]
        if total <= 0.0:
            continue

        for si, (a, b) in enumerate(zip(pts, pts[1:])):
            d = point_segment_distance(centre, a, b)
            if d > _ATTACH_TOLERANCE_M:
                continue
            seg_len = distance(a, b)
            if seg_len <= 0.0:
                continue
            # Fraction along this segment, clamped: a door just past the end of
            # a wall belongs at its end, not off it.
            along = _project_fraction(centre, a, b)
            t = (cumulative[si] + along * seg_len) / total
            if best is None or d < best[0]:
                best = (d, wi, min(1.0, max(0.0, t)))

    if best is None:
        return None
    return best[1], best[2]


def _project_fraction(p: Point, a: Point, b: Point) -> float:
    """How far along `a→b` the foot of `p` falls, as a fraction in [0, 1]."""
    dx, dy = b[0] - a[0], b[1] - a[1]
    len_sq = dx * dx + dy * dy
    if len_sq <= 0.0:
        return 0.0
    t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len_sq
    return min(1.0, max(0.0, t))
