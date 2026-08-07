"""Read line work out of a DXF.

Only the entity types that carry building geometry are read, and everything else
is *counted and reported* rather than silently ignored — a drawing whose walls
are all splines should say so, not import as an empty venue.

ezdxf is MIT-licensed. `docs/07-infrastructure-and-cost.md` §5 rules out
anything AGPL or non-commercial anywhere in the product path, which is also why
PyMuPDF is not used for the PDF path.
"""

from __future__ import annotations

import math
from pathlib import Path
from typing import Any

import ezdxf
from ezdxf.document import Drawing

from importer.errors import NoGeometryError, UnsupportedFileError
from importer.linework import LineWork, Point, Segment, flatten
from importer.units import insunits_metres, insunits_name

#: Entity types read directly. Anything else is counted as skipped.
_SUPPORTED = frozenset({"LINE", "LWPOLYLINE", "POLYLINE", "ARC", "CIRCLE"})

#: Segments per full turn when flattening an arc.
#:
#: 64 puts the sagitta of a 1 m radius arc under 1.2 mm, which is far below the
#: 50 mm endpoint-snapping tolerance repair uses — so flattening never invents a
#: feature that repair would then treat as real.
_ARC_SEGMENTS = 64


def read(path: str | Path) -> LineWork:
    """Extract every wall-ish entity from `path`, in drawing units.

    Coordinates are left in the file's own units. Deciding what those units mean
    is `importer.calibration`'s job, and it is allowed to overrule what the file
    claims — a drawing that says millimetres and measures 8 metres across is
    lying, and only the calibration stage has the context to notice.
    """
    p = Path(path)
    try:
        doc = ezdxf.readfile(str(p))  # type: ignore[attr-defined]
    except OSError as exc:
        raise UnsupportedFileError(f"cannot read {p.name}: {exc}") from exc
    except ezdxf.DXFError as exc:  # type: ignore[attr-defined]
        raise UnsupportedFileError(f"{p.name} is not a readable DXF: {exc}") from exc

    segments: list[Segment] = []
    skipped: dict[str, int] = {}

    for e in doc.modelspace():
        kind = e.dxftype()
        if kind not in _SUPPORTED:
            skipped[kind] = skipped.get(kind, 0) + 1
            continue
        layer = str(getattr(e.dxf, "layer", "0"))
        segments.extend(_segments_of(e, kind, layer))

    warnings = [
        f"{n} {kind} entit{'y' if n == 1 else 'ies'} skipped — not line work"
        for kind, n in sorted(skipped.items())
    ]

    if not segments:
        raise NoGeometryError(
            f"{p.name} has no lines, polylines or arcs in model space"
            + (f" ({'; '.join(warnings)})" if warnings else "")
        )

    code = _insunits(doc)
    return LineWork(
        segments=segments,
        source=p.name,
        unit_metres=insunits_metres(code),
        unit_note=insunits_name(code),
        warnings=warnings,
    )


def _insunits(doc: Drawing) -> int | None:
    """The file's `$INSUNITS`, or None when it declines to say.

    Zero means "unitless" in DXF, which is a statement that the file does not
    know — carrying it through as a code would let a later stage read it as a
    unit. It becomes None here so there is one representation of "unknown".
    """
    try:
        code = int(doc.header.get("$INSUNITS", 0))
    except (KeyError, TypeError, ValueError):
        return None
    return code or None


def _segments_of(e: Any, kind: str, layer: str) -> list[Segment]:
    """Flatten one entity to straight segments."""
    if kind == "LINE":
        a = (float(e.dxf.start.x), float(e.dxf.start.y))
        b = (float(e.dxf.end.x), float(e.dxf.end.y))
        return [] if a == b else [Segment(a, b, layer)]

    if kind == "LWPOLYLINE":
        pts = [(float(x), float(y)) for x, y, *_ in e.get_points()]
        return flatten(pts, layer, closed=bool(e.closed))

    if kind == "POLYLINE":
        pts = [(float(v.dxf.location.x), float(v.dxf.location.y)) for v in e.vertices]
        return flatten(pts, layer, closed=bool(e.is_closed))

    if kind == "CIRCLE":
        c = (float(e.dxf.center.x), float(e.dxf.center.y))
        return _arc(c, float(e.dxf.radius), 0.0, 360.0, layer, closed=True)

    if kind == "ARC":
        c = (float(e.dxf.center.x), float(e.dxf.center.y))
        return _arc(
            c,
            float(e.dxf.radius),
            float(e.dxf.start_angle),
            float(e.dxf.end_angle),
            layer,
            closed=False,
        )

    return []


def _arc(
    centre: Point,
    radius: float,
    start_deg: float,
    end_deg: float,
    layer: str,
    *,
    closed: bool,
) -> list[Segment]:
    """Flatten a circular arc, sampled at a fixed angular step.

    A fixed step rather than a curvature-adaptive one so the same file always
    produces the same points: the whole vector path is deterministic, and an
    adaptive tolerance would make output depend on floating-point comparisons
    that differ between platforms.
    """
    if radius <= 0.0:
        return []
    sweep = (end_deg - start_deg) % 360.0
    if closed:
        sweep = 360.0
    elif sweep == 0.0:
        # A zero sweep on an open arc is a degenerate entity, not a full circle.
        return []

    steps = max(2, round(_ARC_SEGMENTS * sweep / 360.0))
    pts: list[Point] = []
    for i in range(steps + 1):
        t = math.radians(start_deg + sweep * i / steps)
        pts.append((centre[0] + radius * math.cos(t), centre[1] + radius * math.sin(t)))
    if closed:
        pts.pop()
    return flatten(pts, layer, closed=closed)
