"""Command-line import: a drawing in, a venue document out.

    python -m importer plan.dxf --scale mm --layer A-WALL=wall -o venue.json

The output loads directly in the editor. That is the whole point of this being
a command rather than a library call — the import track is useless to anyone who
would have to write Python to reach it.

Exits non-zero with a readable message on any refusal, so it composes.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from importer import ImporterError, ImportOptions, import_file
from importer.calibration import Scale, from_two_points, from_unit_name
from importer.layers import LayerMapping


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="python -m importer",
        description="Import a floorplan into a CrowdFlow venue document.",
        epilog=(
            "Scale is never guessed. Give --scale, --calibrate or --trust-file-units: "
            "a wrong scale makes a venue that is plausibly shaped and entirely the "
            "wrong size, and nothing about the result looks wrong."
        ),
    )
    p.add_argument(
        "drawing",
        type=Path,
        nargs="?",
        help="a .dxf or vector .pdf file (a single-floor venue)",
    )
    p.add_argument(
        "-o",
        "--out",
        type=Path,
        help="where to write the venue JSON (default: alongside the drawing)",
    )
    p.add_argument("--name", default=None, help="venue name (default: the file stem)")

    scale = p.add_argument_group("scale — pick exactly one")
    scale.add_argument(
        "--scale",
        metavar="UNIT",
        help="the drawing's units: mm, cm, m, in, ft",
    )
    scale.add_argument(
        "--calibrate",
        nargs=5,
        type=float,
        metavar=("X1", "Y1", "X2", "Y2", "METRES"),
        help="two known points in drawing units and the real distance between them",
    )
    scale.add_argument(
        "--trust-file-units",
        action="store_true",
        help="accept the file's own $INSUNITS unchecked (often just the template's)",
    )

    p.add_argument(
        "--layer",
        action="append",
        default=[],
        metavar="NAME=ROLE",
        help="map a layer, e.g. A-WALL=wall. Repeatable. Unmapped layers use "
        "heuristics; --list-layers shows what those would decide.",
    )
    building = p.add_argument_group("multi-storey")
    building.add_argument(
        "--floor",
        action="append",
        default=[],
        metavar="ID=FILE[@ELEV]",
        help="a storey and its drawing, e.g. f1=first.dxf@4.0. Repeatable. "
        "Replaces the positional drawing argument.",
    )
    building.add_argument(
        "--stair",
        action="append",
        default=[],
        metavar="X,Y[,WIDTH[,FROM,TO]]",
        help="a staircase at X,Y metres joining two floors (default the first "
        "two). Stairs are never inferred — a stair invented in the wrong place "
        "gives a building an escape route it does not have.",
    )

    p.add_argument(
        "--page",
        type=int,
        default=0,
        help="which page of a PDF (default 0). A drawing set has one plan per "
        "page, and importing them superimposed makes a building nobody drew.",
    )
    p.add_argument(
        "--list-layers",
        action="store_true",
        help="report the layers in the file and stop, without importing",
    )
    return p


def _scale_from(args: argparse.Namespace) -> Scale | None:
    if args.scale:
        return from_unit_name(args.scale)
    if args.calibrate:
        x1, y1, x2, y2, metres = args.calibrate
        return from_two_points((x1, y1), (x2, y2), metres)
    return None


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)

    try:
        pairs = [tuple(spec.split("=", 1)) for spec in args.layer]
    except ValueError:
        print("--layer expects NAME=ROLE", file=sys.stderr)
        return 2
    if any(len(pair) != 2 for pair in pairs):
        print("--layer expects NAME=ROLE", file=sys.stderr)
        return 2

    # A building has no single drawing to take its name from.
    stem = args.drawing.stem if args.drawing is not None else "building"
    opts = ImportOptions(
        layers=LayerMapping.from_pairs(pairs),
        scale=_scale_from(args),
        trust_file_units=args.trust_file_units,
        name=args.name or stem,
        venue_id=f"vnu_{stem}",
        page=args.page,
    )

    if args.floor:
        return _import_building(args, opts)
    if args.drawing is None:
        print("give a drawing, or --floor ID=FILE for a building", file=sys.stderr)
        return 2

    # Listing layers must work *before* a scale is known — deciding which layer
    # is the wall layer is how a user gets far enough to answer the scale
    # question, and demanding the answer first is a loop with no way in.
    if args.list_layers:
        return _list_layers(args.drawing)

    try:
        result = import_file(args.drawing, opts)
    except ImporterError as exc:
        print(f"{args.drawing.name}: {exc}", file=sys.stderr)
        return 1

    out = args.out or args.drawing.with_suffix(".venue.json")
    out.write_text(result.venue.model_dump_json(indent=2, exclude_none=True) + "\n")

    print(f"{out}: {result.wall_count} wall(s), {result.opening_count} opening(s)")
    # The note, not the source enum. `source` is one of four coarse buckets and
    # says "fileHeader" for a unit the user typed on the command line; the note
    # says what actually happened.
    print(
        f"  scale {result.scale.units_per_metre:g} units/m — {result.scale.note} "
        f"(confidence {result.scale.confidence:.2f}"
        f"{'' if result.scale.confirmed else ', UNCONFIRMED'})"
    )
    print(f"  {result.repair}")
    for w in result.warnings:
        print(f"  warning: {w}", file=sys.stderr)
    return 0


def _import_building(args: argparse.Namespace, opts: ImportOptions) -> int:
    """Import several drawings as the storeys of one building."""
    from importer import FloorSpec, StairSpec, import_building

    floors: list[FloorSpec] = []
    for spec in args.floor:
        if "=" not in spec:
            print("--floor expects ID=FILE[@ELEV]", file=sys.stderr)
            return 2
        fid, rest = spec.split("=", 1)
        path_text, _, elev_text = rest.partition("@")
        try:
            elev = float(elev_text) if elev_text else float(len(floors)) * 3.5
        except ValueError:
            print(f"--floor {spec}: elevation is not a number", file=sys.stderr)
            return 2
        floors.append(FloorSpec(fid, fid, Path(path_text), elev))

    stairs: list[StairSpec] = []
    for spec in args.stair:
        parts = spec.split(",")
        if len(parts) not in (2, 3, 5):
            print("--stair expects X,Y[,WIDTH[,FROM,TO]]", file=sys.stderr)
            return 2
        try:
            x, y = float(parts[0]), float(parts[1])
            width = float(parts[2]) if len(parts) > 2 else 1.2
        except ValueError:
            print(f"--stair {spec}: coordinates are not numbers", file=sys.stderr)
            return 2
        ends = (
            (parts[3], parts[4])
            if len(parts) == 5
            else (floors[0].floor_id, floors[1].floor_id if len(floors) > 1 else floors[0].floor_id)
        )
        stairs.append(StairSpec(x=x, y=y, width_m=width, floors=ends))

    try:
        result = import_building(floors, stairs, opts)
    except ImporterError as exc:
        print(f"{exc}", file=sys.stderr)
        return 1

    out = args.out or Path("building.venue.json")
    out.write_text(result.venue.model_dump_json(indent=2, exclude_none=True) + "\n")
    print(
        f"{out}: {len(result.venue.floors)} floor(s), {result.wall_count} wall(s), "
        f"{result.opening_count} opening(s), {len(result.venue.links or [])} stair(s)"
    )
    for w in result.warnings:
        print(f"  warning: {w}", file=sys.stderr)
    return 0


def _list_layers(drawing: Path) -> int:
    """Report what is in the file, so a user can decide the mapping."""
    from importer import dxf, pdf_vector
    from importer.layers import summarise

    try:
        work = (
            pdf_vector.read(drawing)
            if drawing.suffix.lower() == ".pdf"
            else dxf.read(drawing)
        )
    except ImporterError as exc:
        print(f"{drawing.name}: {exc}", file=sys.stderr)
        return 1

    rows = summarise(work)
    width = max((len(r.name) for r in rows), default=5)
    print(f"{'layer'.ljust(width)}  {'segments':>8}  {'length':>10}  suggested")
    for r in rows:
        print(
            f"{r.name.ljust(width)}  {r.segment_count:>8}  {r.total_length:>10.1f}  "
            f"{r.suggested} ({r.confidence:.2f})"
        )
    print(f"\nunits: the file says {work.unit_note}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
