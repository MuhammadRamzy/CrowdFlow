"""Scale calibration — deciding how many drawing units make a metre.

This is the step that stops the pipeline producing a venue that is plausibly shaped and
completely the wrong size. Everything downstream — occupant load, egress capacity, every
NFPA and Green Guide number in the dossier — is linear in this one float, and nothing
downstream can detect that it is wrong. A hall at 1/25th scale still triangulates, still
simulates, and still prints a compliance report.

`docs/03-track-a-venue-designer.md` A4 gives the order of preference:

1. File header (`$INSUNITS`) — trusted, still confirmed.
2. OCR'd dimension strings — the AI path (A5); not implemented here.
3. Door-width prior (modal door ~0.9 m) — implemented as a *cross-check*, never as the
   primary source, because it is circular: it infers the scale from the very geometry
   whose size is in question.
4. Manual two-point — the user clicks two points and types the real distance.

and one rule above all of them: **never proceed on an unconfirmed scale.** That is
enforced by :func:`resolve`, which raises rather than guessing.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Literal

from importer.errors import CalibrationError, ScaleUnknownError
from importer.geometry import distance
from importer.linework import LineWork, Point
from importer.units import insunits_name, metres_per_unit

#: The `CalibrationSource` variants the Venue schema accepts.
CalibrationSourceName = Literal["fileHeader", "ocrDimension", "doorWidthPrior", "manualTwoPoint"]

#: Plausible extent of a real venue, in metres. A drawing that lands outside this after
#: scaling is almost certainly a units mistake, so we say so loudly rather than emit it.
MIN_PLAUSIBLE_EXTENT_M = 2.0
MAX_PLAUSIBLE_EXTENT_M = 5_000.0

#: Modal single-leaf door width used by the sanity cross-check (`docs/03` A4).
DOOR_WIDTH_PRIOR_M = 0.9


@dataclass(frozen=True, slots=True)
class Scale:
    """A decided mapping from drawing units to metres, and the story behind it.

    ``units_per_metre`` is what the Venue schema calls ``sourcePxPerMeter``: the number of
    source drawing units in one real metre. For a millimetre drawing that is 1000.0.
    """

    units_per_metre: float
    source: CalibrationSourceName
    confidence: float
    confirmed: bool
    #: Human-readable justification, shown next to the confirmation control.
    note: str = ""

    def __post_init__(self) -> None:
        """Reject a scale that cannot describe a real drawing."""
        if not math.isfinite(self.units_per_metre) or self.units_per_metre <= 0.0:
            raise CalibrationError(
                f"units per metre must be finite and positive, got {self.units_per_metre!r}"
            )

    @property
    def metres_per_unit(self) -> float:
        """Metres in one source drawing unit — the factor applied to coordinates."""
        return 1.0 / self.units_per_metre

    def to_metres(self, value: float) -> float:
        """Convert a length in drawing units to metres."""
        return value * self.metres_per_unit

    def apply(self, work: LineWork) -> LineWork:
        """Return *work* with every coordinate converted to metres."""
        return work.scaled(self.metres_per_unit)


def from_unit_name(name: str, *, confirmed: bool = True) -> Scale:
    """Scale from an explicitly stated unit, e.g. the CLI's ``--units mm``.

    Confirmed by default: the user typed it, which *is* the confirmation.
    """
    m_per_unit = metres_per_unit(name)
    return Scale(
        units_per_metre=1.0 / m_per_unit,
        source="fileHeader",
        confidence=1.0 if confirmed else 0.9,
        confirmed=confirmed,
        note=f"units stated as {name}",
    )


def from_file_header(metres_per_source_unit: float, unit_code: int | None = None) -> Scale:
    """Scale from the file's own units declaration (DXF ``$INSUNITS``).

    Never `confirmed`. The header is right most of the time and catastrophically wrong
    the rest — a template saved in millimetres and then drawn in metres is a genuinely
    common failure — so it seeds the confirmation dialog rather than skipping it.
    """
    return Scale(
        units_per_metre=1.0 / metres_per_source_unit,
        source="fileHeader",
        confidence=0.9,
        confirmed=False,
        note=f"file header declares {insunits_name(unit_code)}",
    )


def from_two_points(a: Point, b: Point, real_distance_m: float) -> Scale:
    """Scale from two picked points and the real distance between them.

    The user's own measurement, so it is confirmed and fully confident by construction.
    Both degenerate inputs are hard errors rather than clamped values: silently
    accepting a zero-length pick is how you get an infinite scale factor and a venue
    the size of a continent.
    """
    if not math.isfinite(real_distance_m) or real_distance_m <= 0.0:
        raise CalibrationError(
            f"real distance must be finite and positive, got {real_distance_m!r} m"
        )
    drawn = distance(a, b)
    if drawn <= 0.0:
        raise CalibrationError("the two calibration points coincide in the drawing")
    return Scale(
        units_per_metre=drawn / real_distance_m,
        source="manualTwoPoint",
        confidence=1.0,
        confirmed=True,
        note=f"{drawn:.6g} drawing units measured as {real_distance_m:g} m",
    )


def from_drawing_ratio(ratio: float, page_unit: str = "pt") -> Scale:
    """Scale for a drawing plotted at ``1:ratio`` on a page measured in *page_unit*.

    The normal case for a vector PDF: the page is real paper, the plan on it is at
    1:100, so one page millimetre is 100 real millimetres. Confirmed, because the ratio
    came from the user reading it off the title block.
    """
    if not math.isfinite(ratio) or ratio <= 0.0:
        raise CalibrationError(f"drawing ratio must be finite and positive, got {ratio!r}")
    page_m_per_unit = metres_per_unit(page_unit)
    return Scale(
        units_per_metre=1.0 / (page_m_per_unit * ratio),
        source="manualTwoPoint",
        confidence=1.0,
        confirmed=True,
        note=f"plotted at 1:{ratio:g} on a page measured in {page_unit}",
    )


@dataclass(frozen=True, slots=True)
class PlausibilityCheck:
    """Result of sanity-checking a candidate scale against the drawing's own extent."""

    ok: bool
    width_m: float
    height_m: float
    message: str


def check_plausible(work: LineWork, scale: Scale) -> PlausibilityCheck:
    """Sanity-check a scale by asking how big the drawing becomes under it.

    Catches the two mistakes that actually happen — a millimetre drawing read as metres
    (a 60 m hall becomes 60 km) and the reverse (it becomes 6 cm). This is a *check*,
    not a decision: it produces a message for the confirmation dialog and never
    overrules the user.
    """
    min_x, min_y, max_x, max_y = work.bounds()
    w = scale.to_metres(max_x - min_x)
    h = scale.to_metres(max_y - min_y)
    extent = max(w, h)
    if extent < MIN_PLAUSIBLE_EXTENT_M:
        return PlausibilityCheck(
            False, w, h, f"drawing is only {w:.3g} x {h:.3g} m at this scale — units too large?"
        )
    if extent > MAX_PLAUSIBLE_EXTENT_M:
        return PlausibilityCheck(
            False, w, h, f"drawing is {w:.4g} x {h:.4g} m at this scale — units too small?"
        )
    return PlausibilityCheck(True, w, h, f"drawing measures {w:.4g} x {h:.4g} m")


def door_width_prior(door_widths_units: list[float], scale: Scale) -> str | None:
    """Cross-check a scale against detected door widths, returning a warning or ``None``.

    A single-leaf door is 0.9 m almost everywhere. If the median detected door comes out
    at 9 cm or 9 m under the proposed scale, the scale is wrong by an order of magnitude
    and this says so. Used only to *doubt* a scale, never to set one — inferring the
    scale from the geometry whose size is unknown is circular reasoning, and the one
    place in this pipeline where being clever would be actively harmful.
    """
    if not door_widths_units:
        return None
    widths_m = sorted(scale.to_metres(w) for w in door_widths_units)
    median = widths_m[len(widths_m) // 2]
    if median <= 0.0:
        return None
    factor = median / DOOR_WIDTH_PRIOR_M
    if 0.4 <= factor <= 2.5:
        return None
    return (
        f"median detected door is {median:.3g} m, expected around "
        f"{DOOR_WIDTH_PRIOR_M} m — the scale may be off by roughly {factor:.3g}x"
    )


def resolve(
    work: LineWork,
    *,
    override: Scale | None = None,
    assume_file_header: bool = False,
) -> Scale:
    """Decide the scale for a read file, or refuse.

    *override* is whatever the user supplied (two-point, explicit units, drawing ratio)
    and always wins. Failing that, the file's own header is used **only** when
    *assume_file_header* is set — an explicit, auditable opt-in that the API layer sets
    once the user has clicked through the confirmation dialog.

    Raises :class:`ScaleUnknownError` otherwise. That is the whole point of the module:
    the failure mode of guessing is invisible, so guessing is not offered.
    """
    if override is not None:
        return override
    if work.unit_metres is not None and assume_file_header:
        return from_file_header(work.unit_metres)
    if work.unit_metres is None:
        raise ScaleUnknownError(
            f"{work.source} declares no units ({work.unit_note}); supply a scale "
            "(two-point calibration, explicit units, or a drawing ratio)"
        )
    raise ScaleUnknownError(
        f"{work.source} declares {work.unit_note}, but the scale has not been confirmed; "
        "pass --assume-header-units to accept it or supply a two-point calibration"
    )
