"""Layer mapping — the semantic signal in a professional drawing.

A CAD drawing already contains the answer to "which of these lines are walls". It is
in the layer names, and it follows conventions (AIA CAD Layer Guidelines, ISO 13567)
often enough that a regex table gets most of the way there. What it never does is get
*all* the way there, so the design is: heuristics propose, the user disposes.

`docs/03-track-a-venue-designer.md` A4 describes the UI — a table of detected layers with
entity counts, each mapped to a role. :func:`summarise` produces exactly that table's rows,
and :class:`LayerMapping` is what the table's state serialises to. Persisting a mapping
per-organisation is what makes the second drawing from the same architect one click; that
persistence lives in the API layer, but the serialisable shape is here.
"""

from __future__ import annotations

import re
from collections.abc import Iterable, Mapping
from dataclasses import dataclass, field
from enum import StrEnum

from importer.linework import LineWork, Segment, total_length


class LayerRole(StrEnum):
    """What a source layer's geometry means to us.

    Deliberately smaller than the Venue schema's vocabulary. A layer says "this is a
    wall"; it does not say whether that wall is `structural` or `partition` in the
    NFPA sense with any reliability, so the two wall roles below are the only
    distinction we let a layer name make.
    """

    #: Load-bearing / permanent construction. Becomes a `structural` wall.
    WALL = "wall"
    #: Non-structural division. Becomes a `partition` wall.
    PARTITION = "partition"
    #: Door leaves, swings and frames. Becomes an `Opening` on the nearest wall.
    DOOR = "door"
    #: Window openings. Recorded as openings that are *not* fire exits and start closed.
    WINDOW = "window"
    #: The building or hall footprint. Closed loops become zones.
    OUTLINE = "outline"
    #: Furniture, fittings, equipment. Closed loops become obstacles.
    FURNITURE = "furniture"
    #: Dimension lines, leaders, hatching, grid bubbles. Discarded.
    DIMENSION = "dimension"
    #: Text and annotation. Discarded by the vector path; the AI path will read it.
    TEXT = "text"
    #: Explicitly not wanted.
    IGNORE = "ignore"


#: Roles whose geometry becomes walls in the emitted document.
WALL_ROLES: frozenset[LayerRole] = frozenset({LayerRole.WALL, LayerRole.PARTITION})

#: Roles that contribute geometry at all. Everything else is dropped before repair.
CARRIED_ROLES: frozenset[LayerRole] = frozenset(
    {
        LayerRole.WALL,
        LayerRole.PARTITION,
        LayerRole.DOOR,
        LayerRole.WINDOW,
        LayerRole.OUTLINE,
        LayerRole.FURNITURE,
    }
)

# Ordered most-specific first: the first pattern that matches wins. `A-WALL-PRHT` is a
# partition in the AIA scheme, so partitions must be tested before the generic wall rule,
# and door layers before wall layers because `A-WALL-DOOR` exists in the wild.
_HEURISTICS: tuple[tuple[re.Pattern[str], LayerRole], ...] = (
    (re.compile(r"(?:^|[-_ ])(?:door|dr|porte|tuer|puerta)(?:s)?(?:[-_ ]|$)"), LayerRole.DOOR),
    (re.compile(r"a[-_]?door"), LayerRole.DOOR),
    (re.compile(r"(?:^|[-_ ])(?:window|glaz|glass|fenster|fenetre)"), LayerRole.WINDOW),
    (re.compile(r"a[-_]?glaz"), LayerRole.WINDOW),
    (re.compile(r"(?:partition|prht|part|stud|drywall|cloison)"), LayerRole.PARTITION),
    (re.compile(r"(?:^|[-_ ])(?:wall|mur|wand|muro|a-wall|s-wall)"), LayerRole.WALL),
    (re.compile(r"a[-_]?wall"), LayerRole.WALL),
    (re.compile(r"(?:outline|footprint|perimeter|boundary|site|contour|envelope)"), LayerRole.OUTLINE),  # noqa: E501
    (re.compile(r"(?:furn|equip|fixt|casework|seat|table|chair|mobilier)"), LayerRole.FURNITURE),
    (re.compile(r"(?:dim|anno[-_]?dim|hatch|patt|grid|axis|centerline|centreline)"), LayerRole.DIMENSION),  # noqa: E501
    (re.compile(r"(?:text|anno|label|note|title|tblk|legend)"), LayerRole.TEXT),
    (re.compile(r"(?:defpoint|xref|viewport|scratch)"), LayerRole.IGNORE),
)


def suggest_role(layer_name: str) -> tuple[LayerRole, float]:
    """Guess a role for a layer name, with a confidence.

    Returns ``(role, confidence)``. A heuristic hit is 0.8 — good enough to preselect in
    the UI, never good enough to skip the confirmation step. A miss is
    ``(IGNORE, 0.0)``: the safe default is to import nothing, because a stray hatch
    layer read as walls produces a venue full of imaginary obstructions and the user has
    no way to tell which lines were real.
    """
    name = layer_name.strip().lower()
    for pattern, role in _HEURISTICS:
        if pattern.search(name):
            return role, 0.8
    return LayerRole.IGNORE, 0.0


@dataclass(frozen=True, slots=True)
class LayerSummary:
    """One row of the layer-mapping table shown to the user."""

    name: str
    segment_count: int
    total_length: float
    suggested: LayerRole
    confidence: float


def summarise(work: LineWork) -> list[LayerSummary]:
    """Build the layer-mapping table for a read file.

    Sorted by descending total length: the layers carrying the most line work are the
    ones the user must classify correctly, and they should not have to scroll to find
    them.
    """
    by_layer: dict[str, list[Segment]] = {}
    for seg in work.segments:
        by_layer.setdefault(seg.layer, []).append(seg)
    rows = []
    for name in sorted(by_layer):
        role, conf = suggest_role(name)
        rows.append(
            LayerSummary(
                name=name,
                segment_count=len(by_layer[name]),
                total_length=total_length(by_layer[name]),
                suggested=role,
                confidence=conf,
            )
        )
    rows.sort(key=lambda r: (-r.total_length, r.name))
    return rows


@dataclass(slots=True)
class LayerMapping:
    """Source layer name to role. Explicit entries win; unlisted layers fall to heuristics.

    Layer names are matched case-insensitively and with surrounding whitespace stripped,
    because "A-WALL" and "a-wall " are the same layer to everyone except a dict.
    """

    explicit: dict[str, LayerRole] = field(default_factory=dict)
    #: Applied to layers with no explicit entry. ``None`` means "use the heuristics".
    fallback: LayerRole | None = None

    @classmethod
    def from_pairs(cls, pairs: Iterable[tuple[str, str]]) -> LayerMapping:
        """Build from ``(layer, role)`` string pairs — the CLI's ``--layer NAME=ROLE``."""
        explicit: dict[str, LayerRole] = {}
        for name, role in pairs:
            explicit[name.strip().lower()] = LayerRole(role.strip().lower())
        return cls(explicit=explicit)

    @classmethod
    def suggested_for(cls, work: LineWork) -> LayerMapping:
        """Seed a mapping from the heuristics alone — the pre-filled state of the UI table."""
        return cls(explicit={row.name.strip().lower(): row.suggested for row in summarise(work)})

    def role_for(self, layer_name: str) -> LayerRole:
        """Resolve one layer name to a role."""
        key = layer_name.strip().lower()
        if key in self.explicit:
            return self.explicit[key]
        if self.fallback is not None:
            return self.fallback
        return suggest_role(layer_name)[0]

    def to_dict(self) -> Mapping[str, str]:
        """Serialisable form, for persisting a mapping per organisation."""
        return {k: str(v) for k, v in sorted(self.explicit.items())}


def partition_by_role(
    work: LineWork, mapping: LayerMapping
) -> dict[LayerRole, list[Segment]]:
    """Split a file's segments into role buckets, dropping roles we do not carry.

    Bucket contents are sorted into canonical order so every downstream stage sees the
    same sequence for the same input regardless of the order entities happened to appear
    in the file. Determinism is cheaper to buy here than to debug later.
    """
    buckets: dict[LayerRole, list[Segment]] = {}
    for seg in work.segments:
        role = mapping.role_for(seg.layer)
        if role not in CARRIED_ROLES:
            continue
        buckets.setdefault(role, []).append(seg)
    for segs in buckets.values():
        segs.sort()
    return buckets
