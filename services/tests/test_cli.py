"""The command line.

The import track is useless to anyone who would have to write Python to reach
it, so the CLI is part of the deliverable rather than a convenience. These check
the things a script depends on: exit codes, and that a refusal explains itself.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from importer.__main__ import main

from make_fixture import write_all

FIXTURES = Path(__file__).parent / "fixtures"


@pytest.fixture(scope="session", autouse=True)
def _fixtures() -> None:
    write_all(FIXTURES)


def test_it_writes_a_loadable_venue(tmp_path: Path) -> None:
    out = tmp_path / "v.json"
    code = main(
        [
            str(FIXTURES / "hall-mm.dxf"),
            "--scale",
            "mm",
            "--layer",
            "A-WALL=wall",
            "-o",
            str(out),
        ]
    )
    assert code == 0
    doc = json.loads(out.read_text())
    assert doc["schemaVersion"] == "cfs.venue/1.0"
    assert doc["floors"][0]["walls"]


def test_an_unconfirmed_scale_exits_nonzero(tmp_path: Path) -> None:
    # Composability: a script that pipes this must be able to tell that the
    # import did not happen, rather than reading an empty venue as success.
    code = main(
        [str(FIXTURES / "hall-mm.dxf"), "--layer", "A-WALL=wall", "-o", str(tmp_path / "v.json")]
    )
    assert code == 1
    assert not (tmp_path / "v.json").exists()


def test_a_drawing_with_no_geometry_exits_nonzero(tmp_path: Path) -> None:
    code = main(
        [str(FIXTURES / "text-only.dxf"), "--scale", "mm", "-o", str(tmp_path / "v.json")]
    )
    assert code == 1


def test_listing_layers_does_not_need_a_scale(capsys: pytest.CaptureFixture[str]) -> None:
    # Deciding which layer is the wall layer is how a user gets far enough to
    # answer the scale question. Demanding the scale first is a loop with no
    # way in.
    code = main([str(FIXTURES / "hall-mm.dxf"), "--list-layers"])
    assert code == 0
    out = capsys.readouterr().out
    assert "A-WALL" in out
    assert "millimetres" in out


def test_a_malformed_layer_argument_is_rejected(tmp_path: Path) -> None:
    code = main(
        [str(FIXTURES / "hall-mm.dxf"), "--scale", "mm", "--layer", "nonsense", "-o", str(tmp_path / "v.json")]
    )
    assert code == 2


def test_two_point_calibration_works(tmp_path: Path) -> None:
    # The south wall runs 0..20000 drawing units and is 20 m long.
    out = tmp_path / "v.json"
    code = main(
        [
            str(FIXTURES / "hall-mm.dxf"),
            "--calibrate",
            "0",
            "0",
            "20000",
            "0",
            "20",
            "--layer",
            "A-WALL=wall",
            "-o",
            str(out),
        ]
    )
    assert code == 0
    doc = json.loads(out.read_text())
    xs = [p[0] for w in doc["floors"][0]["walls"] for p in w["polyline"]]
    assert max(xs) == pytest.approx(20.0, abs=0.05)
