"""CrowdFlow Studio floorplan import pipeline.

Turns a professional drawing into a draft **Venue document** (`cfs.venue/1.0`).

The vector path implemented here is deterministic end to end — no ML, no randomness,
no wall-clock. The same file with the same options produces a byte-identical document,
which is what makes it testable and what makes it a trustworthy baseline to measure the
later AI raster path against (`docs/03-track-a-venue-designer.md` A4/A5).

Stages, in order:

1. **read** (`dxf`, `pdf_vector`) — source file to a flat bag of :class:`~importer.linework.Segment`
   in *drawing units*, tagged with the layer they came from.
2. **calibrate** (`calibration`) — establish drawing-units-per-metre. Never guessed
   silently; an unconfirmed scale is a hard error, because a wrong scale produces a venue
   that is plausibly shaped and completely the wrong size.
3. **map layers** (`layers`) — decide which layers are walls, doors, outlines, junk.
4. **repair** (`topology`) — the part real drawings need: dedupe, snap, merge collinear,
   chain into polylines, close loops, infer openings from gaps.
5. **emit** (`emit`) — Venue document with per-element provenance and confidence.

`pipeline.import_file` runs all five.
"""

from importer.errors import (
    ImporterError,
    NoGeometryError,
    ScaleUnknownError,
    UnsupportedFileError,
)
from importer.pipeline import (
    FloorSpec,
    ImportOptions,
    ImportResult,
    StairSpec,
    import_building,
    import_file,
)

__all__ = [
    "FloorSpec",
    "ImportOptions",
    "ImportResult",
    "ImporterError",
    "NoGeometryError",
    "ScaleUnknownError",
    "StairSpec",
    "UnsupportedFileError",
    "import_building",
    "import_file",
]
