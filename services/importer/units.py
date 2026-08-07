"""Length units, and the DXF `$INSUNITS` table.

All stored venue coordinates are metres (`docs/02-data-model.md` — `Units.length` is a
*display* preference only). Everything in this module exists to get from whatever a
drawing was authored in to metres exactly once, at a single point in the pipeline.

The conversions are exact rationals where the definition is exact (an inch *is*
25.4 mm), so a drawing in inches round-trips without accumulating float drift.
"""

from __future__ import annotations

from typing import Final

#: Metres per unit, for the unit names a human might type on the command line.
#: Survey units (the US survey foot) are deliberately absent — they differ from the
#: international foot by 2 ppm, which is far below any tolerance here, and offering
#: both invites picking the wrong one.
METRES_PER_UNIT: Final[dict[str, float]] = {
    "m": 1.0,
    "metre": 1.0,
    "meter": 1.0,
    "mm": 0.001,
    "millimetre": 0.001,
    "millimeter": 0.001,
    "cm": 0.01,
    "centimetre": 0.01,
    "centimeter": 0.01,
    "dm": 0.1,
    "km": 1000.0,
    "in": 0.0254,
    "inch": 0.0254,
    "ft": 0.3048,
    "foot": 0.3048,
    "feet": 0.3048,
    "yd": 0.9144,
    "yard": 0.9144,
    "mi": 1609.344,
    "mile": 1609.344,
    #: A PostScript/PDF point. Only meaningful for a PDF at 1:1 paper scale.
    "pt": 0.0254 / 72.0,
    "point": 0.0254 / 72.0,
}

#: DXF header `$INSUNITS` code to metres per drawing unit.
#:
#: Codes come from the DXF reference. Code 0 means "unitless" — the single most common
#: value in real files, and the reason manual calibration exists. It maps to ``None``,
#: not to 1.0: a drawing that declines to say what its units are has not said "metres".
INSUNITS_METRES: Final[dict[int, float | None]] = {
    0: None,  # Unitless — no claim made. Must be resolved by the user.
    1: 0.0254,  # Inches
    2: 0.3048,  # Feet
    3: 1609.344,  # Miles
    4: 0.001,  # Millimetres
    5: 0.01,  # Centimetres
    6: 1.0,  # Metres
    7: 1000.0,  # Kilometres
    8: 0.0254e-6,  # Microinches
    9: 0.0254e-3,  # Mils
    10: 0.9144,  # Yards
    11: 1e-10,  # Angstroms
    12: 1e-9,  # Nanometres
    13: 1e-6,  # Microns
    14: 0.1,  # Decimetres
    15: 10.0,  # Decametres
    16: 100.0,  # Hectometres
    17: 1e9,  # Gigametres
    18: 1.495978707e11,  # Astronomical units
    19: 9.4607304725808e15,  # Light years
    20: 3.0856775814913673e16,  # Parsecs
    21: 0.3048006096012192,  # US survey feet
    22: 0.0254000508001016,  # US survey inch
    23: 0.9144018288036576,  # US survey yard
    24: 1609.3472186944373,  # US survey mile
}

#: Human-readable names for `$INSUNITS`, for the layer/scale review table.
INSUNITS_NAMES: Final[dict[int, str]] = {
    0: "unitless",
    1: "inches",
    2: "feet",
    3: "miles",
    4: "millimetres",
    5: "centimetres",
    6: "metres",
    7: "kilometres",
    10: "yards",
    14: "decimetres",
    21: "US survey feet",
}


def metres_per_unit(name: str) -> float:
    """Return metres per drawing unit for a unit *name*.

    Raises ``KeyError`` with the accepted names listed, because a typo here silently
    scales an entire venue.
    """
    key = name.strip().lower()
    try:
        return METRES_PER_UNIT[key]
    except KeyError:
        accepted = ", ".join(sorted(METRES_PER_UNIT))
        raise KeyError(f"unknown length unit {name!r}; expected one of: {accepted}") from None


def insunits_metres(code: int | None) -> float | None:
    """Return metres per drawing unit for a DXF `$INSUNITS` code.

    ``None`` for unitless, unset, or a code we do not recognise — all three mean the
    same thing to the caller: *the file did not tell us*.
    """
    if code is None:
        return None
    return INSUNITS_METRES.get(code)


def insunits_name(code: int | None) -> str:
    """Return a display name for an `$INSUNITS` code, for the review UI."""
    if code is None:
        return "absent"
    return INSUNITS_NAMES.get(code, f"code {code}")
