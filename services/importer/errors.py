"""Import failures, as a typed hierarchy.

The Rust side uses `thiserror` so a caller can match on the failure rather than parse a
string; the Python side gets the same courtesy. The review UI needs to distinguish
"this file has no scale, ask the user" from "this file is not a drawing at all" —
those are different screens, not different messages.
"""

from __future__ import annotations


class ImporterError(Exception):
    """Base class for every failure raised by the import pipeline."""


class UnsupportedFileError(ImporterError):
    """The file is not a format this pipeline reads, or is corrupt beyond recovery.

    Raised before any geometry work, so the caller can offer a different upload path
    (e.g. "this looks like a raster PDF — the AI import is not built yet").
    """


class ScaleUnknownError(ImporterError):
    """The drawing's real-world scale could not be established, and none was supplied.

    Deliberately fatal. `docs/03-track-a-venue-designer.md` A4 is explicit: *never
    proceed on an unconfirmed scale*. A venue at 1/1000th of its real size still
    compiles, still simulates, and every compliance number it produces is nonsense.
    """


class NoGeometryError(ImporterError):
    """Nothing survived reading and layer mapping.

    Usually means the layer mapping selected nothing (all layers left as `ignore`),
    or the drawing's line work is in paper space / an xref we did not resolve.
    """


class CalibrationError(ImporterError):
    """A two-point calibration was impossible to satisfy.

    Zero real distance, negative distance, or two points that coincide in the drawing.
    """
