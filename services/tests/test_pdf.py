"""Vector PDF import.

A PDF that came from CAD carries the same lines a DXF would. What it does not
carry is layers or a drawing scale, and both absences change how the pipeline
has to be driven — which is what these pin down.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from importer import ImportOptions, ImporterError, import_file
from importer.calibration import from_two_points
from importer.layers import LayerMapping
from importer.pdf_vector import PDF_LAYER

from make_fixture import write_all

FIXTURES = Path(__file__).parent / "fixtures"


@pytest.fixture(scope="session", autouse=True)
def _fixtures() -> None:
    write_all(FIXTURES)


def pdf_options() -> ImportOptions:
    # 20 m of wall drawn at 1:100 spans 566.93 points. Nothing in the file says
    # so, which is exactly why two-point calibration is the PDF path's normal
    # route rather than a fallback.
    return ImportOptions(
        layers=LayerMapping.from_pairs([(PDF_LAYER, "wall")]),
        scale=from_two_points((100.0, 100.0), (666.93, 100.0), 20.0),
    )


def test_a_vector_pdf_imports_to_walls_in_metres() -> None:
    r = import_file(FIXTURES / "hall.pdf", pdf_options())

    assert r.wall_count > 0, r.warnings
    xs = [p.root[0] for w in r.venue.floors[0].walls for p in w.polyline]
    ys = [p.root[1] for w in r.venue.floors[0].walls for p in w.polyline]
    assert max(xs) - min(xs) == pytest.approx(20.0, abs=0.1)
    assert max(ys) - min(ys) == pytest.approx(12.0, abs=0.1)


def test_the_page_scale_is_not_the_drawing_scale() -> None:
    """A PDF's own units are points on the page, not metres in the building.

    `trust_file_units` reads 1/72 inch and produces a hall 20 cm across — a
    building that is plausibly shaped and two orders of magnitude wrong, which
    is the exact failure the calibration stage exists to prevent. The importer
    must not present that as a successful import.
    """
    opts = ImportOptions(
        layers=LayerMapping.from_pairs([(PDF_LAYER, "wall")]),
        trust_file_units=True,
    )
    r = import_file(FIXTURES / "hall.pdf", opts)

    xs = [p.root[0] for w in r.venue.floors[0].walls for p in w.polyline]
    width = max(xs) - min(xs)
    assert width < 1.0, "page units happened to give a sane size; rewrite this test"
    assert any("scale looks wrong" in w for w in r.warnings), r.warnings


def test_a_pdf_says_it_has_no_layers() -> None:
    # A user has to know why every segment landed in one bucket, or they will
    # think the layer mapping is broken.
    r = import_file(FIXTURES / "hall.pdf", pdf_options())
    assert any("no layers" in w for w in r.warnings), r.warnings


def test_asking_for_a_page_that_is_not_there_says_so() -> None:
    opts = ImportOptions(
        layers=LayerMapping.from_pairs([(PDF_LAYER, "wall")]),
        scale=from_two_points((100.0, 100.0), (666.93, 100.0), 20.0),
        page=7,
    )
    with pytest.raises(ImporterError, match="page"):
        import_file(FIXTURES / "hall.pdf", opts)


def test_a_raster_only_file_is_refused_with_the_right_advice() -> None:
    # A scanned plan has no vector line work. Saying "no geometry" without
    # saying *why* sends a user looking for a bug in their file.
    from importer import NoGeometryError
    from reportlab.pdfgen import canvas

    empty = FIXTURES / "text-only.pdf"
    c = canvas.Canvas(str(empty))
    c.drawString(100, 100, "SCANNED PLAN")
    c.showPage()
    c.save()

    with pytest.raises(NoGeometryError, match="raster"):
        import_file(empty, pdf_options())


def test_the_same_pdf_imports_identically_twice() -> None:
    a = import_file(FIXTURES / "hall.pdf", pdf_options())
    b = import_file(FIXTURES / "hall.pdf", pdf_options())
    assert a.venue.model_dump_json() == b.venue.model_dump_json()
