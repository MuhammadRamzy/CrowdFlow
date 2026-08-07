# services — floorplan import

Turns a professional drawing into a draft **Venue document** (`cfs.venue/1.0`)
that the Rust engine can compile and simulate.

## Setup

```bash
cd services
python3 -m venv .venv && source .venv/bin/activate
pip install -e ".[dev]"
```

## Import a drawing

```bash
python3 -m importer plan.dxf --list-layers          # what is in the file
python3 -m importer plan.dxf --scale mm \
        --layer A-WALL=wall --layer A-DOOR=door \
        -o venue.json                               # import it
```

The JSON loads straight into the editor. Exits non-zero on any refusal, so it
composes. `--list-layers` deliberately works *before* a scale is known: deciding
which layer is the wall layer is how a user gets far enough to answer the scale
question, and demanding it first is a loop with no way in.

If you do not know the units, measure something you do know:

```bash
python3 -m importer plan.dxf --calibrate 0 0 20000 0 20 --layer A-WALL=wall
```

## A building, not a floor

```bash
python3 -m importer --scale mm --layer A-WALL=wall \
    --floor f0=ground.dxf --floor f1=first.dxf@4.0 \
    --stair 15,6,1.4 -o building.venue.json
```

Each drawing goes through exactly the per-floor path, so a two-storey import
cannot drift from a one-storey import in how it repairs geometry. Element ids
are prefixed per floor — two storeys from the same drawing both call their first
wall `w_0000`, and unprefixed the compiler sees one venue with duplicate ids and
openings attaching to the wrong storey.

**Stairs are never inferred.** A drawing marks a stair with a symbol that varies
by office and by decade, and a stair invented in the wrong place gives a
building an escape route it does not have — an error in the optimistic
direction, on the figure a venue is approved against. You say where it is. A
stair naming a floor that is not in the set is reported rather than dropped
quietly, and a multi-storey building with no stairs at all says so.

## Commands

```bash
pytest                      # the suite
mypy importer               # strict; generated models are excluded
ruff check .                # style
python3 tests/make_fixture.py   # regenerate the DXF fixtures
```

## What works today

**DXF** — `LINE`, `LWPOLYLINE`, `POLYLINE`, `ARC` and `CIRCLE`. Anything else
is *counted and reported*, so a drawing whose walls are all splines says so
rather than importing as an empty venue.

**Vector PDF** — stroked and filled paths, Béziers subdivided. A PDF carries no
layers, so everything arrives on one bucket named `pdf` which you map wholesale.
It also carries no *drawing* scale: its units are points on the page, and a plan
at 1:100 puts 1 m of wall in 0.72 pt. `--trust-file-units` on a PDF therefore
produces a building two orders of magnitude too small — the importer warns, but
two-point calibration is the normal route here rather than a fallback.

```python
from importer import ImportOptions, import_file
from importer.calibration import from_unit_name
from importer.layers import LayerMapping

result = import_file(
    "plan.dxf",
    ImportOptions(
        layers=LayerMapping.from_pairs([("A-WALL", "wall"), ("A-DOOR", "door")]),
        scale=from_unit_name("mm"),
    ),
)
print(result.wall_count, result.opening_count, result.warnings)
open("venue.json", "w").write(result.venue.model_dump_json(exclude_none=True))
```

## The five stages

1. **read** (`dxf`) — file to a bag of segments in drawing units, tagged by layer.
2. **calibrate** (`calibration`) — decide drawing-units-per-metre.
3. **map layers** (`layers`) — which layers are walls, doors, furniture, junk.
4. **repair** (`topology`) — dedupe, snap, merge collinear, chain, close
   junctions, read openings from a door layer and infer the rest from gaps.
   Where both point at the same place the drawing wins, because two overlapping
   openings in one wall is a compiler warning caused entirely by the importer
   failing to spot its own duplicate.
5. **emit** (`emit`) — a Venue document with per-element provenance.

`ImportResult` carries the working from every stage, not just the document.
When an import comes out wrong the question is always *which stage*, and a
monolith cannot answer it.

## Two things worth knowing before changing anything

**Scale is never guessed silently.** A drawing that does not state its units, or
whose units have not been confirmed, raises `ScaleUnknownError`. A DXF's
`$INSUNITS` is frequently just whatever the template had, so `trust_file_units`
is off by default. This is deliberate and slightly annoying on purpose: a wrong
scale produces a venue that is plausibly shaped and completely the wrong size,
every downstream figure inherits it, and nothing about the result looks wrong.

**Everything after calibration is in metres.** `pipeline` converts once, right
after the scale is decided, and nothing downstream scales again. This is not
stylistic — repair's tolerances are metric (a 50 mm snapping radius, a 0.85–2.0 m
opening range) and they *are* its judgement. An earlier version scaled in `emit`
instead, which meant repair ran against raw millimetres: a 1 m doorway measured
1000 against a 2.0 ceiling and was never recognised, so a hall imported with no
way out and nothing said anything was wrong. `test_a_gap_in_a_wall_becomes_a_doorway`
guards it.

## Licensing

Every dependency in the product path must be permissive —
`docs/07-infrastructure-and-cost.md` §5.

| Package | Licence | Why |
|---|---|---|
| `ezdxf` | MIT | DXF reading |
| `pdfminer.six` | MIT | Vector PDF reading |
| `pydantic` | MIT | Generated document models |
| `pytest`, `mypy`, `ruff` | MIT | Dev only |

**PyMuPDF is AGPL and must not be used**, despite being the obvious tool for
this job. `pdfminer.six` is MIT and is what the PDF path uses.

## Not built yet

- **AI raster import** (track A5) — photographed and scanned plans.

## The cross-track test

`engine/cf-compile/tests/imported.rs` compiles this importer's real output in
the Rust engine and asserts it is simulable. Both sides generate their types
from `schema/` (ADR 0001), so they *should* agree — but that has been wrong
twice, and both times every test on each side passed while the boundary was
broken. `fixtures/unit/imported-hall.venue.json` is regenerated by running
`pytest`.
