"""Generate the DXF fixtures the importer tests read.

Written rather than committed as binary so the *contents* are reviewable: a
fixture whose geometry nobody can read is a fixture nobody can reason about
when a test starts failing.
"""

from __future__ import annotations

from pathlib import Path

import ezdxf


def hall_with_two_doors(path: Path) -> None:
    """A 20 x 12 m hall in millimetres, with a gap in the south wall.

    Deliberately messy in the ways real drawings are:

    - a duplicated wall segment, as if traced twice;
    - a 3 mm gap at one corner, below any real feature;
    - a hatch fragment far too short to be a wall;
    - furniture on its own layer, which must not become a wall.
    """
    doc = ezdxf.new("R2010", setup=True)
    doc.header["$INSUNITS"] = 4  # millimetres
    ms = doc.modelspace()

    def wall(a, b):
        ms.add_line(a, b, dxfattribs={"layer": "A-WALL"})

    # South wall, with a 1.0 m doorway between x=9 and x=10.
    wall((0, 0), (9000, 0))
    wall((10000, 0), (20000, 0))
    # East, north, west — with a 3 mm miss at the north-west corner.
    wall((20000, 0), (20000, 12000))
    wall((20000, 12000), (0, 12000))
    wall((0, 12003), (0, 0))

    # Traced twice: repair must collapse this, not emit two walls.
    wall((0, 0), (9000, 0))

    # Hatch noise, 8 mm long.
    ms.add_line((5000, 6000), (5008, 6000), dxfattribs={"layer": "A-HATCH"})

    # Furniture, on its own layer.
    ms.add_lwpolyline(
        [(3000, 3000), (4000, 3000), (4000, 4000), (3000, 4000)],
        close=True,
        dxfattribs={"layer": "A-FURN"},
    )

    # A curve, to exercise arc flattening.
    ms.add_arc((10000, 6000), 1500, 0, 90, dxfattribs={"layer": "A-WALL"})

    doc.saveas(path)


def unitless(path: Path) -> None:
    """A drawing that declines to say what its units are."""
    doc = ezdxf.new("R2010", setup=True)
    doc.header["$INSUNITS"] = 0
    ms = doc.modelspace()
    ms.add_line((0, 0), (20, 0), dxfattribs={"layer": "A-WALL"})
    ms.add_line((20, 0), (20, 12), dxfattribs={"layer": "A-WALL"})
    doc.saveas(path)


def text_only(path: Path) -> None:
    """A drawing with no line work at all."""
    doc = ezdxf.new("R2010", setup=True)
    doc.modelspace().add_text("NOT A PLAN", dxfattribs={"layer": "A-ANNO"})
    doc.saveas(path)


def write_all(into: Path) -> None:
    into.mkdir(parents=True, exist_ok=True)
    hall_with_two_doors(into / "hall-mm.dxf")
    unitless(into / "unitless.dxf")
    text_only(into / "text-only.dxf")


if __name__ == "__main__":
    write_all(Path(__file__).parent / "fixtures")
    print("fixtures written")
