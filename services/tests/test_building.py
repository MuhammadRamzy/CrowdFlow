"""Importing several drawings as the storeys of one building.

The engine has floors and stairs; the importer emitted exactly one floor. These
join the two, and pin the parts that only go wrong once there is more than one
storey — colliding element ids, and a staircase that names a floor nobody has.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from importer import FloorSpec, ImportOptions, StairSpec, import_building
from importer.calibration import from_unit_name
from importer.layers import LayerMapping

from make_fixture import write_all

FIXTURES = Path(__file__).parent / "fixtures"


@pytest.fixture(scope="session", autouse=True)
def _fixtures() -> None:
    write_all(FIXTURES)


def opts() -> ImportOptions:
    return ImportOptions(
        layers=LayerMapping.from_pairs([("A-WALL", "wall"), ("A-FURN", "furniture")]),
        scale=from_unit_name("mm"),
    )


def two_storey(stairs: list[StairSpec] | None = None):
    return import_building(
        [
            FloorSpec("f0", "Ground", FIXTURES / "hall-mm.dxf", 0.0),
            FloorSpec("f1", "First", FIXTURES / "hall-mm.dxf", 4.0),
        ],
        stairs if stairs is not None else [StairSpec(x=15.0, y=6.0, width_m=1.4)],
        opts(),
    )


def test_two_drawings_become_two_storeys() -> None:
    r = two_storey()
    assert [f.id for f in r.venue.floors] == ["f0", "f1"]
    assert [f.elevationM for f in r.venue.floors] == [0.0, 4.0]
    assert all(f.walls for f in r.venue.floors)


def test_element_ids_do_not_collide_across_floors() -> None:
    """Two storeys from the same drawing both call their first wall `w_0000`.

    Unprefixed, the compiler sees one venue with duplicate ids and openings
    attach to the wrong storey — a building that is subtly not the one drawn.
    """
    r = two_storey()
    ids = [w.id for f in r.venue.floors for w in f.walls]
    assert len(ids) == len(set(ids)), "duplicate wall ids across floors"

    # And every opening still points at a wall on its own floor.
    for f in r.venue.floors:
        wall_ids = {w.id for w in f.walls}
        for o in f.openings or []:
            assert o.wall in wall_ids, f"{o.id} points off its floor"


def test_a_stair_joins_the_two_floors() -> None:
    r = two_storey()
    assert len(r.venue.links) == 1
    link = r.venue.links[0]
    assert [e.floor for e in link.ends] == ["f0", "f1"]
    assert link.clearWidthM == pytest.approx(1.4)
    # Green Guide: 66 persons/m/min on stairs against 82 on the level.
    assert link.flowRatePpmm == pytest.approx(66.0)
    # The footprint has to enclose the point it was placed at.
    xs = [p.root[0] for p in link.ends[0].footprint]
    assert min(xs) < 15.0 < max(xs)


def test_upper_storeys_with_no_stair_are_reported() -> None:
    # A building whose upper floors have no vertical route evacuates through
    # the doors it has left and reports a time it could never achieve.
    r = two_storey(stairs=[])
    assert any("no way out" in w for w in r.warnings), r.warnings


def test_a_stair_naming_an_absent_floor_is_reported_not_dropped_quietly() -> None:
    r = two_storey(stairs=[StairSpec(x=15.0, y=6.0, floors=("f0", "f9"))])
    assert not r.venue.links
    assert any("f9" in w for w in r.warnings), r.warnings


def test_the_repair_report_covers_the_whole_building() -> None:
    one = import_building(
        [FloorSpec("f0", "Ground", FIXTURES / "hall-mm.dxf", 0.0)], [], opts()
    )
    two = two_storey()
    # Two identical storeys should have done twice the work of one.
    assert two.repair.input_segments == one.repair.input_segments * 2
    assert two.repair.output_walls == one.repair.output_walls * 2


def test_a_building_needs_at_least_one_floor() -> None:
    from importer import UnsupportedFileError

    with pytest.raises(UnsupportedFileError):
        import_building([], [], opts())


def test_one_floor_matches_a_plain_import() -> None:
    # The multi-floor path shares the per-floor path rather than reimplementing
    # it, so a one-storey building must equal a one-file import.
    from importer import import_file

    a = import_building(
        [FloorSpec("f0", "Ground", FIXTURES / "hall-mm.dxf", 0.0)], [], opts()
    )
    b = import_file(FIXTURES / "hall-mm.dxf", opts())
    assert len(a.venue.floors[0].walls) == len(b.venue.floors[0].walls)
    assert len(a.venue.floors[0].openings) == len(b.venue.floors[0].openings)
