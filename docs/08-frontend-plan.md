# Frontend: design direction and build order

**Re-prioritised 2026-07-29.** Reviews are coming, and the frontend must be
*functional* — real components driving the real engine, not a mockup. This
document is the design brief and the build sequence.

---

## 1. Why this can be functional now, not faked

The engine already does the work:

| Capability | Crate | State |
|---|---|---|
| Venue + scenario documents | `cf-schema` | done |
| Exact geometry, offsetting | `cf-geom` | done |
| CDT, regions, portals, pathfinding | `cf-navmesh` | done |
| `VenueDoc` → `NavGraph`, warnings | `cf-compile` | done |
| Agents, SFM+PBD, step loop, exits | `cf-sim` | done |

So the frontend does not need to simulate anything itself, or pretend to. It
needs to (a) render a venue, (b) hand it to the engine, (c) draw what comes
back. Everything on screen will be a real computed result. That is the whole
argument for doing the frontend now rather than later: the expensive half is
already true.

---

## 2. Design brief

**Subject.** A venue editor and crowd-simulation workspace.
**Audience.** Safety officers and event planners, who will sit in this for hours
iterating on a layout — not visitors skimming a landing page.
**The page's one job.** Show a planner where their venue fails, and let them
change it.

### The thesis: the workspace is an instrument; the report is the blueprint

The pitch deck established a cream-and-navy drafting look. That is right for the
*deliverable* — a PDF dossier handed to a licensing authority should look like a
drawing. It is wrong for the *workspace*: a cream field fights a density heatmap,
and nobody wants to stare at high-key paper while reading data for three hours.

So the two surfaces are deliberately different, and the difference means
something:

- **Workspace** — dark, quiet, no colour of its own.
- **Exported report** — the cream blueprint from the deck.

### Colour: the interface has no colour, because every coloured pixel means something

The density ramp is *scientific data* — 0–2 / 2–4 / 4–6 / 6+ persons per m² are
defined thresholds, not styling. If the chrome is also colourful, the chrome
competes with the readout and the readout stops being legible at a glance.

So all UI saturation is spent on state, and the accents are borrowed from the
equipment that actually lives in these buildings — **life-safety annunciator
panels**, which use a fixed and universally-read vocabulary:

```
--void      #0B0E13   outside the venue; the workspace field
--surface   #141A22   panels, rails
--raised    #1C242F   inputs, cards, hover
--line      #263140   rules, borders, grid
--chalk     #C9D3DF   primary text — silver pencil on drafting film
--muted     #7B8A9B   secondary text, units, disabled

--normal    #3DD68C   system normal        (annunciator green)
--supervise #FFB020   warning / attention  (annunciator amber)
--alarm     #FF4D4D   violation / critical (annunciator red)

--select    #58C4E8   selection and active tool only — never decorative
```

The density ramp is separate, and belongs to `dataviz`, not to the chrome.

### Type: mono-first, because everything here is a measured quantity

Widths, clear widths, occupant loads, egress times, densities, throughputs.
Nearly every number in this product is a measurement with a unit, and
measurements want tabular figures and a drafting hand.

So the unusual call: **IBM Plex Mono is the primary interface face** for labels,
values, and controls — not a code-only afterthought. **IBM Plex Sans Condensed**
takes headings and prose, where condensed widths buy horizontal room in narrow
inspector panels. Both are Open Font Licence and self-hosted, which the
cross-origin isolation requirement demands anyway (`docs/01-architecture.md` §7).

Scale is small and dense — this is an instrument panel, not a marketing page:
`11 / 12 / 13 / 15 / 18 / 24`, with `12` as the workhorse.

### Layout

The four-region CAD convention — tool rail, canvas, inspector, timeline — is
what users of Pathfinder, Revit and OnePlan already know. Fighting it would be
novelty at the expense of usability, so it stays.

```
┌──────────────────────────────────────────────────────────────┐
│  LIFE SAFETY STATUS  ← the signature; always live            │
├───┬──────────────────────────────────────────┬───────────────┤
│ T │                                          │  INSPECTOR    │
│ O │             CANVAS                       │  selection    │
│ O │             venue · mesh · agents        │  properties   │
│ L │             heatmap                      │               │
│   │                                          ├───────────────┤
│ R │                                          │  VALIDATION   │
│ A │                                          │  compile      │
│ I │                                          │  warnings     │
│ L │                                          │               │
├───┴──────────────────────────────────────────┴───────────────┤
│  ◀ ▶  t=00:41  ×1  ────────●───────────  1,284 in · 216 out  │
└──────────────────────────────────────────────────────────────┘
```

### Signature: the life-safety status bar

The one thing this product should be remembered by is **the moment it tells you
your venue fails**. So that moment is not buried in a report — it is a permanent
strip across the top, modelled on a fire alarm annunciator, showing live:

```
 ● NORMAL     OCCUPANCY  1,284 / 1,962      EGRESS  4:12 / 8:00      PEAK  3.1 p/m²
 ▲ SUPERVISE  OCCUPANCY  1,890 / 1,962      EGRESS  7:41 / 8:00      PEAK  4.4 p/m²
 ■ ALARM      OCCUPANCY  2,140 / 1,962 ✕    EGRESS  9:03 / 8:00 ✕    PEAK  6.2 p/m² ✕
```

It updates as you drag a wall, and again as the simulation runs. The venue is
always being audited and you can always see the verdict — which is the product's
actual proposition, made into a component rather than a claim.

Everything else stays quiet so this can be loud.

### What was deliberately not chosen

- **Cream + serif + terracotta** — the deck's look, right for the report, wrong
  for a workspace you stare at for hours.
- **Near-black + one acid accent** — would collide with the density ramp, which
  needs to own saturation.
- **Numbered step markers (01/02/03)** — nothing here is a sequence.

---

## 3. Build order

Each step ends with something demonstrable. Nothing is a placeholder.

| # | Step | Delivers |
|---|---|---|
| **F1** | `cf-wasm` bindings | Browser can compile a venue and step a sim |
| **F2** | Vite + React + TS scaffold, design tokens, COOP/COEP | The shell, with the type and colour system real |
| **F3** | PixiJS canvas: viewport, grid, venue render | Pan/zoom a real venue from `venue.json` |
| **F4** | Compile on edit → validation panel | Real `CompileWarning`s, clickable |
| **F5** | Agent rendering + transport controls | **500 agents walk out of the hall — M1** |
| **F6** | Life-safety status bar | The signature, live |
| **F7** | Wall/zone drawing tools + command undo stack | Author a venue from scratch |
| **F8** | Density heatmap overlay | The headline visual |

F1–F5 is the reviewable demo. F6 makes it memorable. F7–F8 make it a product.

## 4. Non-negotiables

- **No fake data.** Every number shown is computed by the engine. If something
  is not implemented, it is absent — not mocked.
- **Canvas stays out of React.** React owns chrome; PixiJS owns the scene.
- **Every document mutation goes through a `Command`** (`docs/01-architecture.md`).
- Keyboard focus visible, `prefers-reduced-motion` respected, responsive down to
  a laptop screen.
