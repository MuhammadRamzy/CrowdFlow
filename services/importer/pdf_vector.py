"""Read line work out of a vector PDF.

A PDF that came from CAD carries the same lines a DXF would, drawn as path
operators. It carries no layers and no units — so this produces geometry in
**points** (1/72 inch) with every segment on one synthetic layer, and leans
harder on the calibration stage than the DXF path does.

pdfminer.six is MIT. `docs/07-infrastructure-and-cost.md` §5 rules out anything
AGPL or non-commercial anywhere in the product path, which is why **PyMuPDF is
not used here** despite being the obvious tool — it is AGPL-3.0.

# What this cannot do

There are no layers in a PDF, so `layers.LayerRole` heuristics have nothing to
work with and everything arrives as one bucket. A user must either map that
bucket to walls wholesale or clean up afterwards. Recovering structure from
stroke width and colour is possible and is not attempted here; guessing wrong
would silently drop walls.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from importer.errors import NoGeometryError, UnsupportedFileError
from importer.linework import LineWork, Point, Segment, flatten
from importer.units import metres_per_unit

#: Everything from a PDF lands here. There are no layers to distinguish.
PDF_LAYER = "pdf"

#: Curves are flattened at this many segments each.
#:
#: Fixed rather than adaptive so the same file always yields the same points.
#: The whole vector path is deterministic, and an adaptive tolerance makes
#: output depend on floating-point comparisons that vary between platforms.
_CURVE_SEGMENTS = 16


def read(path: str | Path, page: int = 0) -> LineWork:
    """Extract stroked and filled paths from one page, in PDF points.

    Only the first page by default. A drawing set has one plan per page and
    importing all of them superimposed would produce a building nobody drew.
    """
    p = Path(path)
    try:
        from pdfminer.high_level import extract_pages
        from pdfminer.layout import LAParams, LTCurve
    except ImportError as exc:  # pragma: no cover - dependency is declared
        raise UnsupportedFileError(
            "PDF import needs pdfminer.six — pip install -e '.[dev]'"
        ) from exc

    segments: list[Segment] = []
    pages = 0
    try:
        # No layout analysis: this wants geometry, not paragraphs, and the text
        # grouping is both irrelevant and much of the cost.
        for i, layout in enumerate(extract_pages(str(p), laparams=LAParams())):
            pages += 1
            if i != page:
                continue
            for element in layout:
                segments.extend(_segments_of(element, LTCurve))
    except OSError as exc:
        raise UnsupportedFileError(f"cannot read {p.name}: {exc}") from exc
    except Exception as exc:  # pdfminer raises a wide variety on malformed input
        raise UnsupportedFileError(f"{p.name} is not a readable PDF: {exc}") from exc

    if page >= pages:
        raise UnsupportedFileError(
            f"{p.name} has {pages} page(s); page {page} was asked for"
        )
    if not segments:
        raise NoGeometryError(
            f"{p.name} page {page} has no vector line work. A scanned or "
            "photographed plan is a raster import (track A5), not this."
        )

    return LineWork(
        segments=segments,
        source=p.name,
        # A PDF point is 1/72 inch. That is the unit the *page* is in, which is
        # not the unit the *building* is in — a plan at 1:100 puts 1 m of wall
        # in 0.72 pt. Calibration has to close that gap, and this is only the
        # first half of it.
        unit_metres=metres_per_unit("pt"),
        unit_note="PDF points (page units, not drawing scale)",
        warnings=[
            "PDF has no layers, so every segment is on one bucket — map it to "
            "a role explicitly or clean up afterwards",
            f"read page {page} of {pages}",
        ],
    )


def _segments_of(element: Any, curve_cls: type) -> list[Segment]:
    """Flatten one layout element, recursing into containers."""
    out: list[Segment] = []

    if isinstance(element, curve_cls):
        return _flatten_curve(element)

    # LTFigure and friends nest.
    children = getattr(element, "_objs", None)
    if children:
        for child in children:
            out.extend(_segments_of(child, curve_cls))
    return out


def _flatten_curve(curve: Any) -> list[Segment]:
    """A pdfminer curve to straight segments.

    `pts` is already the flattened outline for lines and rectangles. Béziers
    arrive as their control points, which is wrong to treat as a polyline — it
    cuts the corner — so they are subdivided.
    """
    pts: list[Point] = [(float(x), float(y)) for x, y in getattr(curve, "pts", [])]
    if len(pts) < 2:
        return []

    original = getattr(curve, "original_path", None)
    if original:
        expanded = _expand_path(original)
        if expanded:
            pts = expanded

    closed = bool(getattr(curve, "is_closed", lambda: False)())
    return flatten(pts, PDF_LAYER, closed=closed)


def _expand_path(path: list[Any]) -> list[Point]:
    """Walk a pdfminer `original_path`, subdividing any cubic segments."""
    out: list[Point] = []
    cursor: Point | None = None

    for op in path:
        if not op:
            continue
        kind, coords = op[0], [(float(x), float(y)) for x, y in op[1:]]
        if kind in ("m", "l") and coords:
            out.extend(coords)
            cursor = coords[-1]
        elif kind == "c" and len(coords) == 3 and cursor is not None:
            out.extend(_bezier(cursor, coords[0], coords[1], coords[2]))
            cursor = coords[2]
        elif kind == "h":
            # Close: the flattener adds the closing edge itself.
            continue
    return out


def _bezier(p0: Point, p1: Point, p2: Point, p3: Point) -> list[Point]:
    """Subdivide a cubic Bézier, excluding its start point."""
    out: list[Point] = []
    for i in range(1, _CURVE_SEGMENTS + 1):
        t = i / _CURVE_SEGMENTS
        u = 1.0 - t
        x = u**3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t**3 * p3[0]
        y = u**3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t**3 * p3[1]
        out.append((x, y))
    return out
