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


def hall_with_a_door_layer(path: Path) -> None:
    """The same hall, but the doors are *drawn* rather than left as gaps.

    A door on a door layer is a leaf plus a swing arc. Both span exactly the
    opening, which is what lets the importer read a width without recognising
    the symbol.

    The south wall is continuous here — no gap — so anything the importer finds
    can only have come from the door layer.
    """
    doc = ezdxf.new("R2010", setup=True)
    doc.header["$INSUNITS"] = 4
    ms = doc.modelspace()

    for a, b in [
        ((0, 0), (20000, 0)),
        ((20000, 0), (20000, 12000)),
        ((20000, 12000), (0, 12000)),
        ((0, 12000), (0, 0)),
    ]:
        ms.add_line(a, b, dxfattribs={"layer": "A-WALL"})

    # A 1.1 m door at x = 9000: leaf, then its swing arc.
    ms.add_line((9000, 0), (9000, 1100), dxfattribs={"layer": "A-DOOR"})
    ms.add_arc((9000, 0), 1100, 0, 90, dxfattribs={"layer": "A-DOOR"})

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


def hall_pdf(path: Path) -> None:
    """The same hall as a vector PDF, drawn at 1:100.

    20 m of wall at 1:100 is 200 mm on the page, which is 566.9 points. Nothing
    in the file says so — that is the whole difficulty of the PDF path, and why
    calibration matters more here than for DXF.
    """
    from reportlab.pdfgen import canvas
    from reportlab.lib.units import mm

    c = canvas.Canvas(str(path))
    scale = 10.0 * mm / 1000.0  # 1:100, drawing mm to page points

    def wall(x1, y1, x2, y2):
        c.line(100 + x1 * scale, 100 + y1 * scale, 100 + x2 * scale, 100 + y2 * scale)

    wall(0, 0, 9000, 0)
    wall(10000, 0, 20000, 0)
    wall(20000, 0, 20000, 12000)
    wall(20000, 12000, 0, 12000)
    wall(0, 12000, 0, 0)
    c.showPage()
    c.save()


def write_all(into: Path) -> None:
    into.mkdir(parents=True, exist_ok=True)
    hall_with_two_doors(into / "hall-mm.dxf")
    hall_with_a_door_layer(into / "hall-doorlayer.dxf")
    unitless(into / "unitless.dxf")
    text_only(into / "text-only.dxf")
    hall_pdf(into / "hall.pdf")


if __name__ == "__main__":
    write_all(Path(__file__).parent / "fixtures")
    print("fixtures written")
