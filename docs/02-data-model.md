# CrowdFlow Studio — Data Model

This is the contract between Track A and Track B. It is written first, in Phase 0, and
changes to it are governed by CI gate G1 (`01-architecture.md` §6).

---

## 1. Three documents, three lifecycles

| Document | Lifecycle | Owner | Store |
|---|---|---|---|
| **Venue** | Authored, versioned, immutable-per-version | Track A | Postgres `JSONB` + S3 |
| **Scenario** | Authored, versioned, references a venue version | Track A | Postgres `JSONB` |
| **Run** | Generated, immutable, content-addressed | Track B | S3 + Postgres index row |

Geometry and scenario are split so one venue supports many scenarios (Optimal Ops / Peak Load /
Evacuation) without duplicating geometry — this is Core Capability 8 from the source plan,
and separating them at the schema level is what makes it cheap.

---

## 2. Venue Document (`cfs.venue/1.0`)

```jsonc
{
  "schemaVersion": "cfs.venue/1.0",
  "id": "vnu_01JQZ8...",
  "name": "Chennai Trade Centre — Hall 3",
  "units": { "length": "m", "angle": "deg" },
  "georef": { "epsg": 32644, "origin": [0, 0], "rotationDeg": 0 },   // optional
  "scale": { "sourcePxPerMeter": 37.79, "calibration": "ocr_dimension", "confidence": 0.94 },

  "layers": [
    { "id": "lay_struct", "name": "Structure", "visible": true, "locked": false, "z": 0 },
    { "id": "lay_ai",     "name": "AI proposals", "visible": true, "locked": false, "z": 90,
      "kind": "proposal" }
  ],

  "floors": [
    {
      "id": "f0", "name": "Ground", "elevationM": 0.0, "ceilingM": 6.5,

      "walls": [
        { "id": "w_001", "layer": "lay_struct",
          "polyline": [[0,0],[42.5,0]],          // metres, floor-local
          "thicknessM": 0.23,
          "kind": "structural",                  // structural | partition | barrier | temporary
          "permeable": false,
          "provenance": { "source": "import.dxf", "confidence": 1.0 } }
      ],

      "openings": [
        { "id": "op_001", "wall": "w_001", "t": 0.345,   // param along wall polyline [0,1]
          "widthM": 1.80, "kind": "double_door",         // door|double_door|gap|gate|revolving
          "swing": "both",
          "isFireExit": true,
          "capacityFactor": 1.0,                          // multiplies Green Guide rate
          "schedule": []                                  // see §2.4
        }
      ],

      "zones": [
        { "id": "z_hall", "polygon": [[0,0],[42.5,0],[42.5,30],[0,30]],
          "kind": "assembly_concentrated",   // drives NFPA OLF; see §5
          "name": "Main hall",
          "olfOverride": null,               // m²/person, null → from `kind`
          "access": ["general", "staff", "vip"],
          "speedMultiplier": 1.0,
          "attractors": [ { "kind": "stage", "point": [21, 28], "weight": 1.0 } ],
          "isVoid": false }
      ],

      "obstacles": [
        { "id": "ob_001", "polygon": [[10,10],[10.6,10],[10.6,10.6],[10,10.6]],
          "kind": "pillar", "heightM": 6.5, "traversable": false }
      ],

      "components": [
        { "id": "cmp_001", "type": "turnstile",
          "transform": { "p": [12.0, 3.0], "rotDeg": 90 },
          "params": {
            "lanes": 2, "laneWidthM": 0.60, "direction": "in",
            "serviceTime": { "dist": "lognormal", "muLn": 0.10, "sigmaLn": 0.35,
                             "minS": 0.8, "maxS": 8.0 },
            "maxThroughputPph": 660,
            "failureRate": 0.0
          },
          "queueArea": "z_queue_n",
          "servesAccess": ["general"] }
      ]
    }
  ],

  "links": [
    { "id": "lnk_001", "kind": "stair",
      "ends": [ { "floor": "f0", "footprint": [[...]] },
                { "floor": "f1", "footprint": [[...]] } ],
      "widthM": 2.40, "clearWidthM": 2.20,
      "steps": 22, "riserM": 0.170, "goingM": 0.280,
      "direction": "both",
      "flowRatePpmm": 66,               // Green Guide stepped
      "speedMultiplierUp": 0.52, "speedMultiplierDown": 0.65 }
  ],

  "routing": {
    "waypoints": [ { "id": "wp_01", "floor": "f0", "p": [5, 5], "radiusM": 1.5 } ],
    "edges":     [ { "from": "wp_01", "to": "wp_02", "direction": "forward", "costMult": 1.0 } ],
    "flowConstraints": [
      { "zone": "z_corr_a", "kind": "one_way", "headingDeg": 90, "strength": 0.85 }
    ]
  },

  "annotations": [ { "id":"an_1","kind":"text","p":[3,3],"text":"North entry cluster" } ],

  "provenance": {
    "source": "import",
    "importJob": "imp_01JQ...",
    "sourceFile": "s3://cf-uploads/...",
    "reviewedBy": "usr_01J...",
    "reviewedAt": "2026-09-14T10:22:11Z"
  }
}
```

### 2.1 Why openings are parametric on walls

`opening.t ∈ [0,1]` along the parent wall rather than absolute coordinates. Moving or
re-snapping a wall carries its doors with it automatically — the single most common editing
operation stays correct without a constraint solver. The compiler resolves `t` → world
coordinates.

### 2.2 Zone `kind` is the compliance hook

`kind` is an enum mapped to NFPA 101 Table 7.3.1.2 occupant load factors (§5). Authors pick a
semantic label; the compliance engine derives the number. `olfOverride` exists for
jurisdictions/AHJ rulings that differ, and always records a justification note.

### 2.3 Component types (v1 library)

| `type` | Key params | Simulation role |
|---|---|---|
| `turnstile` | lanes, laneWidthM, direction, serviceTime, maxThroughputPph | Hard capacity gate; primary bottleneck source |
| `security_checkpoint` | stations, serviceTime, secondaryRate, secondaryTime, footprint | Multi-stage service with probabilistic re-screen |
| `registration_desk` | positions, serviceTime, queueDiscipline (single/serpentine/parallel) | Variable dwell; compares queue geometries |
| `ticket_counter` | as above + paymentTime | |
| `barricade` | polyline, heightM, permeable | Non-wall obstacle, fast to place/move |
| `seating_block` | rows, cols, seatPitchM, rowPitchM, aisles[], entryNodes[] | Constrained micro-movement, seat-finding, row egress |
| `stall` / `booth` | footprint, serviceTime, attractorWeight | Dwell attractor + obstacle |
| `stair` / `ramp` / `escalator` / `lift` | see `links` | Vertical transit with speed/flow reduction |
| `sign` | headingDeg, radiusM, complianceRate | Modifies wayfinding weight for unfamiliar agents |

Each is a data file in `web/src/components-library/` **and** a Rust handler in `cf-sim`.
Adding one is a two-file change plus a fixture — this pairing is enforced by a codegen test.

### 2.4 Schedules

Any opening, component, or link may carry a schedule, giving Core Capability 3's time windows:

```jsonc
"schedule": [
  { "fromS": 0,    "state": "closed" },
  { "fromS": 1800, "state": "open" },
  { "fromS": 7200, "state": "exit_only" }
]
```

---

## 3. Scenario Document (`cfs.scenario/1.0`)

```jsonc
{
  "schemaVersion": "cfs.scenario/1.0",
  "id": "scn_01JQ...",
  "venueVersion": "ver_01JQ...",
  "name": "Peak load — doors 19:00",
  "mode": "event_flow",              // event_flow | peak_load | evacuation
  "durationS": 5400,
  "timestepS": 0.05,
  "seed": 20260729,

  "populations": [
    {
      "id": "pop_ga", "label": "General admission", "count": 18000,
      "profile": {
        "desiredSpeed": { "dist": "normal", "mean": 1.34, "sd": 0.26, "min": 0.60, "max": 2.20 },
        "radiusM":      { "dist": "normal", "mean": 0.23, "sd": 0.02, "min": 0.18, "max": 0.30 },
        "massKg":       { "dist": "normal", "mean": 72, "sd": 12 },
        "groupSize":    { "dist": "categorical", "p": { "1":0.45, "2":0.30, "3":0.15, "4":0.10 } },
        "patienceS":    { "dist": "exponential", "lambda": 0.004 },
        "familiarity": 0.60,                 // 0 → follows signage only, 1 → knows all routes
        "mobilityImpairedFrac": 0.03,
        "reactionTimeS": { "dist": "lognormal", "muLn": 2.3, "sigmaLn": 0.6 }   // evac only
      },
      "arrival": {
        "kind": "curve",
        "points": [[0,0.00],[1800,0.35],[3000,0.90],[3600,1.00]],  // (t, cumulative fraction)
        "entries": [ { "opening": "op_001", "weight": 0.6 },
                     { "opening": "op_014", "weight": 0.4 } ]
      },
      "itinerary": [
        { "goal": "component:cmp_001" },
        { "goal": "zone:z_hall", "dwell": { "dist": "lognormal", "muLn": 6.0, "sigmaLn": 0.8 } },
        { "goal": "zone:z_food", "probability": 0.42,
          "dwell": { "dist": "lognormal", "muLn": 5.4, "sigmaLn": 0.7 } },
        { "goal": "exit:nearest" }
      ]
    }
  ],

  "events": [
    { "atS": 300,  "kind": "close_opening",  "target": "op_004" },
    { "atS": 4800, "kind": "alarm",          "scope": "all", "egressPolicy": "nearest_available" },
    { "atS": 4830, "kind": "block_link",     "target": "lnk_003" }
  ],

  "compliance": {
    "codes": ["NFPA101", "GreenGuide"],
    "targetEgressS": 480,
    "occupancyBasis": "simulated"      // simulated | declared
  },

  "output": {
    "densityGridM": 0.5,
    "densityBucketS": 5,
    "trajectorySampleRate": 0.02,      // fraction of agents recorded at full rate
    "trajectoryHz": 2
  }
}
```

**Distributions** are a closed tagged union (`normal|lognormal|exponential|uniform|categorical|
constant|empirical`) with a shared Rust/TS sampler so authoring UI and engine can never
disagree about what `lognormal` means.

---

## 4. Compiled artifact — NavGraph

Not authored, not user-visible, not hand-editable. Binary (`bincode` + `zstd`), content-addressed
by `blake3(venue_doc_canonical_json ++ compiler_version)`.

```rust
pub struct NavGraph {
    pub version: CompilerVersion,
    pub source_hash: [u8; 32],
    pub floors: Vec<FloorMesh>,
    pub links: Vec<LinkPortal>,          // cross-floor transitions
    pub components: Vec<CompiledComponent>,
    pub goals: Vec<GoalDef>,             // named destinations flow fields can be built for
    pub bounds: Aabb,
    pub warnings: Vec<CompileWarning>,   // surfaced in the editor's validation panel
}

pub struct FloorMesh {
    pub verts: Vec<[f32; 2]>,
    pub tris: Vec<[u32; 3]>,
    pub tri_adj: Vec<[i32; 3]>,          // -1 = boundary
    pub portals: Vec<Portal>,            // shared edges, with clear width
    pub tri_zone: Vec<u16>,              // → zone attributes (speed mult, access mask)
    pub obstacle_bvh: Bvh,               // for SFM wall repulsion nearest-surface queries
    pub grid_index: UniformGrid,         // tri lookup from position, O(1)
}
```

Flow fields are computed lazily per goal and cached alongside:
`FlowField { potential: Vec<f32> /* per tri */, gradient: Vec<[f32;2]> }`.

`CompileWarning` is the mechanism by which the engine tells the editor what's wrong with the
geometry — unreachable zone, unclosed room, opening narrower than 0.85 m, zone with no exit,
component with no queue area. These render as clickable items in the editor validation panel.
**This feedback channel is what makes the compile step feel like a feature rather than a wall.**

---

## 5. Compliance reference data

`cf-compliance/rules/` — data, not code.

### NFPA 101 Table 7.3.1.2 occupant load factors (subset shipped in v1)

| Zone `kind` | ft²/person (net) | m²/person (net) |
|---|---|---|
| `assembly_concentrated` (standing, dance floor, no fixed seats) | 7 | 0.65 |
| `assembly_less_concentrated` (dining, exhibition) | 15 | 1.4 |
| `assembly_standing_space` | 5 | 0.46 |
| `library_reading` / `exercise` | 50 | 4.6 |
| `business` | 100 (150 in newer editions — configurable) | 9.3 |
| `mercantile_street_floor` | 30 | 2.8 |
| `industrial_general` | 100 | 9.3 |
| `storage` | 500 | 46.5 |

### UK Green Guide rates of passage

| Surface | persons / metre / minute |
|---|---|
| Level (flat) | 82 |
| Stepped / stairways | 66 |

Plus: 8-minute egress benchmark, minimum exit width 1.1 m, exit capacity =
`clear_width_m × rate × egress_minutes`.

### Additional v1 rules

- NFPA 101 §7.3.4 egress capacity factors (0.2 in/person level, 0.3 in/person stairs).
- NFPA 101 jam-point rule: assembly > 930 m² must not exceed 1 person / 0.65 m².
- NFPA 130: platform egress in 4 min, station egress in 6 min, crush-loaded train arrival.
- NBC India 2016 Part 4: travel distances, exit counts.
- Crowd-science density thresholds for heatmap banding: 2 / 4 / 6 persons/m²
  (comfortable / restricted / critical).

Every rule carries `{ id, code, edition, clause, description, severity, citation_url }` so the
generated report cites its source. Rules with a jurisdictional variant are namespaced
(`nfpa101.2024.business_olf`).

---

## 6. Run artifacts

The naive approach (store every agent position every tick) is 1.4 GB for a single 100k-agent
30-minute run. We don't do that. Layered storage:

```
runs/{run_id}/
  manifest.json          # venue_version, scenario_id, seed, engine_version, compiler_version,
                         # wall_time, host (wasm|native), determinism_hash
  metrics.json           # aggregate KPIs — the report reads this
  density/
    f0.zst               # [T][H][W] u8 quantized (0–255 → 0–8 persons/m²), 0.5 m cells, 5 s buckets
  flow/
    f0.zst               # [T][H][W] i8×2 mean velocity field, same grid
  dwell/
    f0.zst               # [H][W] u16 seconds, cumulative
  cohort.bin             # 2% sampled agents, 2 Hz, i16 mm offsets from bounds origin
  events.parquet         # agent_id, t, kind, target   (spawn|enqueue|service|exit|reroute|
                         #                              blocked|group_split|fall)
  components.parquet     # component_id, minute, served, queue_len, mean_wait_s, p95_wait_s
  compliance.json        # per-rule pass/fail with computed values and clause citations
```

Sizing for 100k agents / 30 min / 50k m²: density ~ 90 MB before zstd, ~12 MB after;
cohort ~ 29 MB; events ~ 60 MB. Total well under 150 MB — an order of magnitude better than raw,
and every visualization the product needs is derivable from it.

`determinism_hash` = blake3 of the final positions + event log. Two runs claiming to be the
same must have the same hash; the report prints it.

---

## 7. Versioning model

Venue versions form a **git-like DAG**, not a linear history:

```
ver_A (baseline venue)
  ├── ver_B "wider north exit"       ── scn_peak, scn_evac
  └── ver_C "alternate stage position"
        └── ver_D "C + extra gates"  ── scn_evac_v2
```

```sql
venue_versions (
  id            text primary key,        -- ver_01JQ...
  venue_id      text not null references venues,
  parent_id     text references venue_versions,
  doc           jsonb not null,
  doc_hash      bytea not null,          -- blake3 of canonical JSON
  message       text,
  author_id     text not null,
  created_at    timestamptz not null default now(),
  unique (venue_id, doc_hash)            -- dedupe no-op commits
);
create index on venue_versions using gin (doc jsonb_path_ops);
```

Rules:
- Versions are **immutable**. Editing creates a new version on save/checkpoint.
- Autosave writes to a mutable `working_copy` row; explicit "Save version" promotes it.
- A `Run` pins `(venue_version, scenario_id, engine_version, compiler_version, seed)` — so any
  report is reproducible years later, which is the whole point for an audit artifact.
- Compare view diffs two versions structurally (elements added/removed/modified) and their
  runs' metrics side-by-side. This is Core Capability 8 and it falls out of the model for free.

### Core tables

```
orgs, users, memberships(role)
projects(org_id, name, client, event_date)
venues(project_id, name)
venue_versions(...)                     -- above
working_copies(venue_id, user_id, doc, updated_at)
scenarios(venue_id, doc, name)
import_jobs(id, venue_id?, status, source_key, draft_doc, confidence, error)
runs(id, venue_version_id, scenario_id, seed, target, status, metrics, artifacts_prefix)
reports(id, run_id, template, status, pdf_key)
audit_log(actor, action, subject, at, payload)   -- required for the compliance story
```
