"""Read a drawing, decide its scale, repair it, and emit a venue.

The five stages are separately testable and separately reviewable, which is the
point: when an import comes out wrong the question is always *which stage*, and
a monolith cannot answer it. `ImportResult` therefore carries the intermediate
findings — layer roles, repair counts, scale provenance — not just the document.

**Scale is never guessed silently.** A drawing that does not state its units, or
states them implausibly, raises rather than picking something reasonable. A
wrong scale produces a venue that is plausibly shaped and completely the wrong
size, every downstream figure inherits it, and nothing about the result looks
wrong. That is the worst failure this pipeline can have, so it is the one it
refuses to have quietly.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

from importer import calibration, dxf, pdf_vector
from importer.calibration import Scale
from importer.errors import ScaleUnknownError, UnsupportedFileError
from importer.layers import LayerMapping, LayerRole, LayerSummary, partition_by_role, summarise
from importer.geometry import distance
from importer.linework import LineWork, Segment
from importer.schema.venue import VenueDoc
from importer.topology import (
    OpeningCandidate,
    door_openings,
    RepairOptions,
    RepairReport,
    WallRun,
    repair_walls,
)


@dataclass(frozen=True)
class ImportOptions:
    """Everything a caller may decide. Defaults suit a clean CAD export."""

    #: Layer name to role, e.g. `{"A-WALL": "wall"}`. Unlisted layers fall to
    #: the heuristics in `importer.layers`.
    layers: LayerMapping = field(default_factory=LayerMapping)
    repair: RepairOptions = field(default_factory=RepairOptions)
    #: Overrides whatever the file claims about its units. This is how a user
    #: answers the confirmation dialog.
    scale: Scale | None = None
    #: Accept the file's own unit declaration without a human confirming it.
    #:
    #: Off by default. A DXF `$INSUNITS` is frequently wrong — it is whatever
    #: the template had — and trusting it unattended is how a venue ends up a
    #: thousand times too small.
    trust_file_units: bool = False
    venue_id: str = "vnu_import"
    name: str = "Imported venue"
    #: Which page of a PDF to read. A drawing set has one plan per page, and
    #: importing all of them superimposed produces a building nobody drew.
    page: int = 0


@dataclass
class ImportResult:
    """The document, and enough of the working to review it."""

    venue: VenueDoc
    scale: Scale
    layers: list[LayerSummary]
    repair: RepairReport
    warnings: list[str]

    @property
    def wall_count(self) -> int:
        floors = self.venue.floors or []
        return sum(len(f.walls or []) for f in floors)

    @property
    def opening_count(self) -> int:
        floors = self.venue.floors or []
        return sum(len(f.openings or []) for f in floors)


def import_file(path: str | Path, opts: ImportOptions | None = None) -> ImportResult:
    """Run the whole pipeline over one drawing."""
    opts = opts or ImportOptions()
    p = Path(path)

    # 1. read
    suffix = p.suffix.lower()
    if suffix == ".dxf":
        work = dxf.read(p)
    elif suffix == ".pdf":
        work = pdf_vector.read(p, page=opts.page)
    else:
        raise UnsupportedFileError(
            f"{p.name}: DXF and vector PDF are supported. A scanned or "
            "photographed plan is a raster import (track A5), which is not "
            "built."
        )

    warnings = list(work.warnings)

    # 2. calibrate
    scale = _decide_scale(work, opts, warnings)

    # Convert to metres **here**, before anything downstream looks at a length.
    #
    # Repair's tolerances are metric — a 50 mm snapping radius, a 0.85-2.0 m
    # opening range — and they are the whole of its judgement. Run against a
    # millimetre drawing they are nonsense: an 8 mm hatch fragment survives a
    # 0.02 filter because it measures 8, and a 1 m doorway is never recognised
    # because it measures 1000 against a 2.0 ceiling. The first version of this
    # pipeline scaled in `emit` instead and imported a hall with no doors.
    #
    # Once here, and once only. Everything after this is in metres.
    summaries = summarise(work)
    work = _to_metres(work, scale)

    # 3. map layers
    buckets = partition_by_role(work, opts.layers)
    wall_segments = buckets.get(LayerRole.WALL, [])
    if not wall_segments:
        warnings.append(
            "no layer was identified as walls — every segment fell to another "
            "role or was ignored. Map the wall layer explicitly."
        )

    # 4. repair
    report = RepairReport()
    runs, openings = repair_walls(wall_segments, opts.repair, report)

    # A door layer, where present, outranks gaps inferred from wall runs. The
    # drawing *said* where the doors are; inference is the fallback for drawings
    # that did not. Where both point at the same place, the drawing wins and the
    # gap is dropped rather than emitted as a second door beside the first.
    door_segments = buckets.get(LayerRole.DOOR, [])
    if door_segments:
        from_layer = door_openings(door_segments, opts.repair)
        openings = _prefer_drawn(from_layer, openings, opts.repair)

    # 5. emit
    venue = _emit(runs, openings, scale, opts, p.name, warnings)

    return ImportResult(
        venue=venue,
        scale=scale,
        layers=summaries,
        repair=report,
        warnings=warnings,
    )


def _prefer_drawn(
    drawn: list[OpeningCandidate],
    inferred: list[OpeningCandidate],
    opts: RepairOptions,
) -> list[OpeningCandidate]:
    """Merge door-layer openings with gap-inferred ones, drawn evidence first.

    An inferred gap that sits on top of a drawn door is the *same door*, found
    twice. Emitting both puts two overlapping openings in one wall, which the
    compiler then reports as an overlap — a warning caused entirely by the
    importer being unable to recognise its own duplicate.

    Kept apart by centre distance rather than by identity, because the two
    stages derive the centre differently and will never agree exactly.
    """
    reach = max(opts.gap_max_m, 1.0)
    out = list(drawn)
    for gap in inferred:
        if any(distance(gap.centre, d.centre) <= reach for d in drawn):
            continue
        out.append(gap)
    out.sort(key=lambda o: (o.centre, o.width_m))
    return out


def _to_metres(work: LineWork, scale: Scale) -> LineWork:
    """Rescale every segment from drawing units to metres."""
    k = 1.0 / scale.units_per_metre
    return LineWork(
        segments=[
            Segment((s.a[0] * k, s.a[1] * k), (s.b[0] * k, s.b[1] * k), s.layer)
            for s in work.segments
        ],
        source=work.source,
        unit_metres=1.0,
        unit_note="metres (converted)",
        warnings=work.warnings,
    )


def _decide_scale(work: LineWork, opts: ImportOptions, warnings: list[str]) -> Scale:
    """Settle drawing-units-per-metre, or refuse to proceed.

    Order of authority: what the caller supplied, then what the file claims —
    and only when the caller has said the file may be trusted.
    """
    if opts.scale is not None:
        scale = opts.scale
    elif work.unit_metres is not None and opts.trust_file_units:
        scale = calibration.from_file_header(work.unit_metres)
    elif work.unit_metres is not None:
        raise ScaleUnknownError(
            f"{work.source} declares its units as {work.unit_note}, but that has "
            "not been confirmed. Pass a Scale, or set trust_file_units=True to "
            "accept the file's own claim."
        )
    else:
        raise ScaleUnknownError(
            f"{work.source} does not declare its units. Calibrate from two known "
            "points, or supply a Scale."
        )

    # A scale can be arithmetically valid and still absurd. Say so rather than
    # emitting a 4 mm building.
    check = calibration.check_plausible(work, scale)
    if not check.ok:
        warnings.append(f"scale looks wrong: {check.message}")
    return scale


def _emit(
    runs: list[WallRun],
    openings: list[OpeningCandidate],
    scale: Scale,
    opts: ImportOptions,
    source: str,
    warnings: list[str],
) -> VenueDoc:
    """Late import so `emit`'s schema dependency stays out of the read path."""
    from importer import emit

    return emit.to_venue(
        runs,
        openings,
        scale,
        venue_id=opts.venue_id,
        name=opts.name,
        source=source,
        warnings=warnings,
    )
