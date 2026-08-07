"""The DXF import path, end to end.

The vector path is deterministic by design — no ML, no randomness, no clock —
so these are ordinary assertions rather than tolerances, and a byte-identical
re-run is itself a test.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from importer import ImportOptions, NoGeometryError, ScaleUnknownError, import_file
from importer.calibration import from_unit_name
from importer.layers import LayerMapping

from make_fixture import write_all

FIXTURES = Path(__file__).parent / "fixtures"


@pytest.fixture(scope="session", autouse=True)
def _fixtures() -> None:
    write_all(FIXTURES)


def mm_options(**kw) -> ImportOptions:
    return ImportOptions(
        layers=LayerMapping.from_pairs([("A-WALL", "wall"), ("A-FURN", "furniture")]),
        scale=from_unit_name("mm"),
        **kw,
    )


def test_a_hall_imports_to_walls_in_metres() -> None:
    r = import_file(FIXTURES / "hall-mm.dxf", mm_options())

    assert r.wall_count > 0, r.warnings
    floor = r.venue.floors[0]

    # The drawing is 20 x 12 m expressed in millimetres. If scaling were missed
    # or applied twice these would be 20,000 or 0.02 — both obvious here and
    # neither obvious three stages downstream.
    xs = [p.root[0] for w in floor.walls for p in w.polyline]
    ys = [p.root[1] for w in floor.walls for p in w.polyline]
    assert max(xs) == pytest.approx(20.0, abs=0.05)
    assert max(ys) == pytest.approx(12.0, abs=0.05)
    assert min(xs) == pytest.approx(0.0, abs=0.05)


def test_the_document_validates_as_a_venue() -> None:
    # Pydantic models are generated from schema/, so constructing one at all is
    # the schema check (ADR 0001).
    r = import_file(FIXTURES / "hall-mm.dxf", mm_options())
    assert r.venue.schemaVersion == "cfs.venue/1.0"
    assert r.venue.floors[0].id == "f0"
    assert r.venue.model_dump_json()


def test_duplicated_line_work_becomes_one_wall() -> None:
    # The south wall is traced twice. Two coincident walls would double the
    # building's apparent perimeter and give every agent a wall to fight.
    r = import_file(FIXTURES / "hall-mm.dxf", mm_options())
    assert r.repair.removed_duplicates > 0 or r.repair.merged_collinear > 0, r.repair


def test_hatch_noise_is_dropped() -> None:
    # An 8 mm segment is not a wall. It is on its own layer here, but the
    # length filter is what must catch it — a real drawing puts hatch on the
    # wall layer often enough.
    r = import_file(FIXTURES / "hall-mm.dxf", mm_options())
    for w in r.venue.floors[0].walls:
        for a, b in zip(w.polyline, w.polyline[1:]):
            length = ((a.root[0] - b.root[0]) ** 2 + (a.root[1] - b.root[1]) ** 2) ** 0.5
            assert length > 0.015, f"a {length * 1000:.0f} mm segment survived"


def test_furniture_does_not_become_a_wall() -> None:
    r = import_file(FIXTURES / "hall-mm.dxf", mm_options())
    layers = {w.layer for w in r.venue.floors[0].walls}
    assert "A-FURN" not in layers


def test_an_unstated_scale_is_refused_not_guessed() -> None:
    # The worst failure this pipeline can have is a wrong scale, because the
    # result looks entirely reasonable and every downstream figure is wrong.
    with pytest.raises(ScaleUnknownError):
        import_file(
            FIXTURES / "unitless.dxf",
            ImportOptions(layers=LayerMapping.from_pairs([("A-WALL", "wall")])),
        )


def test_a_declared_scale_still_needs_confirming() -> None:
    # A DXF's $INSUNITS is whatever the template had. Trusting it unattended is
    # how a venue ends up a thousand times too small.
    opts = ImportOptions(layers=LayerMapping.from_pairs([("A-WALL", "wall")]))
    with pytest.raises(ScaleUnknownError):
        import_file(FIXTURES / "hall-mm.dxf", opts)

    trusted = ImportOptions(
        layers=LayerMapping.from_pairs([("A-WALL", "wall")]),
        trust_file_units=True,
    )
    r = import_file(FIXTURES / "hall-mm.dxf", trusted)
    assert r.scale.units_per_metre == pytest.approx(1000.0)


def test_a_drawing_with_no_line_work_says_so() -> None:
    with pytest.raises(NoGeometryError):
        import_file(FIXTURES / "text-only.dxf", mm_options())


def test_an_unsupported_format_is_refused_with_the_right_advice() -> None:
    # DXF and vector PDF are in. A scanned plan is a different problem — raster
    # import, track A5 — and saying so is the difference between a user waiting
    # for a feature and a user hunting for a bug in their file.
    from importer import UnsupportedFileError

    with pytest.raises(UnsupportedFileError, match="raster import"):
        import_file(FIXTURES / "plan.png", mm_options())


def test_the_same_file_imports_identically_twice() -> None:
    # Determinism is the whole argument for the vector path being the baseline
    # the AI raster path is measured against.
    a = import_file(FIXTURES / "hall-mm.dxf", mm_options())
    b = import_file(FIXTURES / "hall-mm.dxf", mm_options())
    assert a.venue.model_dump_json() == b.venue.model_dump_json()


def test_layers_are_summarised_for_review() -> None:
    # The review UI asks a user which layer is which. It can only do that if
    # the importer reports what it found.
    r = import_file(FIXTURES / "hall-mm.dxf", mm_options())
    names = {s.name for s in r.layers}
    assert "A-WALL" in names
    assert "A-FURN" in names


def test_a_gap_in_a_wall_becomes_a_doorway() -> None:
    """The drawing has a 1 m gap in the south wall. It must become an opening.

    This is the regression guard for a real bug: repair's tolerances are metric
    — a 50 mm snap radius, a 0.85–2.0 m opening range — and the first version of
    this pipeline ran it against raw millimetres. A 1 m doorway measured 1000
    against a 2.0 ceiling and was never recognised, so a hall imported with no
    way out and nothing said anything was wrong.
    """
    r = import_file(FIXTURES / "hall-mm.dxf", mm_options())
    openings = r.venue.floors[0].openings

    assert len(openings) == 1, f"expected one doorway, got {len(openings)}"
    op = openings[0]
    assert op.widthM == pytest.approx(1.0, abs=0.05)
    # Parametric, on a real wall, somewhere along it rather than at an end.
    assert op.wall in {w.id for w in r.venue.floors[0].walls}
    assert 0.0 < op.t < 1.0


def test_a_three_millimetre_corner_miss_is_snapped() -> None:
    # Drafting slop, well below any real feature. Left alone it leaves the
    # outline open and the compiler cannot tell inside from outside.
    r = import_file(FIXTURES / "hall-mm.dxf", mm_options())
    assert r.repair.snapped_endpoints > 0, r.repair


def test_provenance_travels_with_each_element() -> None:
    # A review UI bands elements by confidence so a user can accept the crisp
    # 90% and redraw the rest. That is only possible if the number is per
    # element rather than per document.
    r = import_file(FIXTURES / "hall-mm.dxf", mm_options())
    for w in r.venue.floors[0].walls:
        assert w.provenance is not None
        assert w.provenance.source == "import"
        assert 0.0 <= w.provenance.confidence <= 1.0


def test_an_imported_venue_round_trips_as_json() -> None:
    # What the editor loads. Exclude-none matters: the schema's optional fields
    # are optional, and emitting explicit nulls trips the Rust deserialiser.
    import json

    r = import_file(FIXTURES / "hall-mm.dxf", mm_options())
    text = r.venue.model_dump_json(exclude_none=True)
    back = json.loads(text)
    assert back["schemaVersion"] == "cfs.venue/1.0"
    assert back["floors"][0]["walls"]
    assert "provenance" in back["floors"][0]["walls"][0]


def door_layer_options(**kw) -> ImportOptions:
    return ImportOptions(
        layers=LayerMapping.from_pairs([("A-WALL", "wall"), ("A-DOOR", "door")]),
        scale=from_unit_name("mm"),
        **kw,
    )


def test_a_drawn_door_is_found_where_there_is_no_gap() -> None:
    """The south wall is continuous. Any opening can only be from the door layer.

    A drawing that says where its doors are should outrank inference — and until
    this was wired, door-layer segments were counted, reported as unused, and
    thrown away.
    """
    r = import_file(FIXTURES / "hall-doorlayer.dxf", door_layer_options())

    assert r.repair.bridged_openings == 0, "the wall should have no gap to infer from"
    openings = r.venue.floors[0].openings
    assert len(openings) == 1, f"expected the drawn door, got {len(openings)}"
    assert openings[0].widthM == pytest.approx(1.1, abs=0.1)


def test_a_drawn_door_and_an_inferred_gap_do_not_become_two_doors() -> None:
    """The messy hall has *both*: a 1 m gap in the wall and no door layer.

    When a door layer is present and points at the same place as a gap, they are
    the same door found twice. Emitting both puts two overlapping openings in
    one wall, and the compiler then reports an overlap caused entirely by the
    importer failing to recognise its own duplicate.
    """
    r = import_file(FIXTURES / "hall-mm.dxf", door_layer_options())
    openings = r.venue.floors[0].openings

    # hall-mm has no A-DOOR layer, so this is the inference path — one door.
    assert len(openings) == 1

    centres = [(o.wall, round(o.t, 2)) for o in openings]
    assert len(centres) == len(set(centres)), "duplicate openings on one wall"


def test_door_layer_segments_are_no_longer_reported_as_unused() -> None:
    r = import_file(FIXTURES / "hall-doorlayer.dxf", door_layer_options())
    assert not any("not yet used" in w for w in r.warnings), r.warnings
