//! WebAssembly bindings for the CrowdFlow engine.
//!
//! A thin shell. All the work happens in `cf-compile` and `cf-sim`; this crate
//! only marshals across the JS boundary. Keeping it thin is what makes the
//! browser preview and the server report agree — they run the same code.
//!
//! # The boundary rule
//!
//! Per-frame data crosses as **typed array views into wasm memory**, never as
//! serialised JSON. At 25,000 agents, serialising positions every frame would
//! cost more than simulating them. Structured data that changes rarely —
//! documents, warnings, statistics — crosses as JSON, where clarity is worth
//! more than the microseconds.
//!
//! `positions()` returns a `Float32Array` that *aliases wasm memory directly*.
//! It is valid only until the next allocation inside wasm, so the renderer must
//! upload it to the GPU immediately rather than holding it across ticks. This is
//! the standard wasm-bindgen caveat and it is the reason the API hands out a
//! fresh view each frame instead of caching one.

mod scenario;

use cf_compile::{compile, CompileWarning, NavGraph};
use cf_geom::Vec2;
use cf_schema::{ScenarioDoc, VenueDoc};
use cf_sim::building::{Building, Link as BuildingLink};
use cf_sim::world::{AgentState, SpawnParams};
use cf_sim::{ExitSpan, Sim, SimParams};
use scenario::ScenarioRunner;
use wasm_bindgen::prelude::*;

/// Persons/m² mapped to a density byte of 255.
///
/// 8 rather than the 6 p/m² crush threshold, so the critical band occupies the
/// top quarter of the range and remains visually distinct instead of
/// saturating the moment a venue becomes dangerous.
const DENSITY_SCALE: f32 = 8.0;

/// Install a panic hook so a Rust panic surfaces as a readable console error
/// rather than `RuntimeError: unreachable executed`.
#[wasm_bindgen(start)]
pub fn start() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// Engine build identity, shown in the UI and stamped into every report.
#[wasm_bindgen]
pub fn engine_version() -> String {
    format!(
        "cf-wasm/{} {}",
        env!("CARGO_PKG_VERSION"),
        cf_compile::COMPILER_VERSION
    )
}

// ---------------------------------------------------------------------------
// Compiled venue
// ---------------------------------------------------------------------------

/// A compiled venue: mesh, walls, doors and diagnostics.
///
/// The source document is kept alongside the compiled graph. Scenario
/// authoring needs it: zone polygons and routing waypoints are destinations an
/// agent can be sent to, and the `NavGraph` deliberately does not carry either
/// — it is geometry for walking on, not a record of what the author called
/// things.
#[wasm_bindgen]
pub struct CompiledVenue {
    graph: NavGraph,
    doc: VenueDoc,
}

#[wasm_bindgen]
impl CompiledVenue {
    /// Compile a venue document.
    ///
    /// Returns a compiled venue even when the document has problems; read
    /// [`CompiledVenue::warnings`] to find out. A partial result plus
    /// diagnostics is more useful to an editor than an error that loses
    /// everything.
    #[wasm_bindgen(js_name = fromJson)]
    pub fn from_json(json: &str) -> Result<CompiledVenue, JsError> {
        let doc: VenueDoc = serde_json::from_str(json)
            .map_err(|e| JsError::new(&format!("venue document is not valid: {e}")))?;
        Ok(CompiledVenue {
            graph: compile(&doc),
            doc,
        })
    }

    /// Can this venue be simulated?
    #[wasm_bindgen(js_name = isSimulable)]
    pub fn is_simulable(&self) -> bool {
        self.graph.is_simulable()
    }

    /// Walkable floor area in m², excluding obstacles.
    #[wasm_bindgen(js_name = walkableArea)]
    pub fn walkable_area(&self) -> f64 {
        self.graph.total_walkable_area()
    }

    #[wasm_bindgen(js_name = floorCount)]
    pub fn floor_count(&self) -> usize {
        self.graph.floors.len()
    }

    /// Diagnostics for the validation panel, as JSON.
    ///
    /// `[{ "code": "openingTooNarrow", "fatal": false, "message": "..." }]`
    pub fn warnings(&self) -> Result<JsValue, JsError> {
        let items: Vec<WarningView> = self.graph.warnings.iter().map(WarningView::from).collect();
        serde_wasm_bindgen::to_value(&items).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Wall segments as a flat `[x0,y0,x1,y1, ...]` array, in metres.
    ///
    /// Flat rather than nested so the renderer can build a geometry buffer
    /// without walking a JS object graph.
    #[wasm_bindgen(js_name = wallSegments)]
    pub fn wall_segments(&self, floor: usize) -> Vec<f32> {
        let Some(f) = self.graph.floors.get(floor) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut keys: Vec<_> = f.mesh.tri.constraints.iter().copied().collect();
        keys.sort_unstable();
        for (a, b) in keys {
            let (pa, pb) = (f.mesh.tri.points[a], f.mesh.tri.points[b]);
            out.extend_from_slice(&[pa.x as f32, pa.y as f32, pb.x as f32, pb.y as f32]);
        }
        out
    }

    /// Walkable triangles as a flat `[ax,ay,bx,by,cx,cy, ...]` array.
    ///
    /// Solid triangles are omitted: they are not floor, and drawing them would
    /// show the inside of pillars as navigable.
    #[wasm_bindgen(js_name = walkableTriangles)]
    pub fn walkable_triangles(&self, floor: usize) -> Vec<f32> {
        let Some(f) = self.graph.floors.get(floor) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (idx, t) in f.mesh.tri.live() {
            if !f.mesh.is_walkable(idx) {
                continue;
            }
            for v in t.v {
                let p = f.mesh.tri.points[v];
                out.extend_from_slice(&[p.x as f32, p.y as f32]);
            }
        }
        out
    }

    /// Doorways as a flat `[ax,ay,bx,by, ...]` array.
    pub fn doors(&self, floor: usize) -> Vec<f32> {
        let Some(f) = self.graph.floors.get(floor) else {
            return Vec::new();
        };
        f.doors
            .iter()
            .flat_map(|d| [d.a.x as f32, d.a.y as f32, d.b.x as f32, d.b.y as f32])
            .collect()
    }

    /// Bounding box as `[minX, minY, maxX, maxY]`, for fitting the viewport.
    pub fn bounds(&self, floor: usize) -> Vec<f32> {
        let Some(f) = self.graph.floors.get(floor) else {
            return vec![0.0, 0.0, 1.0, 1.0];
        };
        match cf_geom::Aabb::of(f.mesh.tri.points.iter().copied()) {
            Some(b) => vec![
                b.min.x as f32,
                b.min.y as f32,
                b.max.x as f32,
                b.max.y as f32,
            ],
            None => vec![0.0, 0.0, 1.0, 1.0],
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WarningView {
    code: String,
    fatal: bool,
    message: String,
}

impl From<&CompileWarning> for WarningView {
    fn from(w: &CompileWarning) -> Self {
        // A stable machine-readable code so the UI can group and filter without
        // parsing prose.
        let code = match w {
            CompileWarning::DegenerateWall { .. } => "degenerateWall",
            CompileWarning::OpeningOrphaned { .. } => "openingOrphaned",
            CompileWarning::OpeningOverflowsWall { .. } => "openingOverflowsWall",
            CompileWarning::OpeningTooNarrow { .. } => "openingTooNarrow",
            CompileWarning::OpeningsOverlap { .. } => "openingsOverlap",
            CompileWarning::NoWalkableArea { .. } => "noWalkableArea",
            CompileWarning::UnclosedOutline { .. } => "unclosedOutline",
            CompileWarning::DisconnectedRegion { .. } => "disconnectedRegion",
            CompileWarning::ZoneNotOnFloor { .. } => "zoneNotOnFloor",
            CompileWarning::ZoneSpeedNotApplied { .. } => "zoneSpeedNotApplied",
            CompileWarning::LinkNotUsable { .. } => "linkNotUsable",
            CompileWarning::NoFireExit { .. } => "noFireExit",
            CompileWarning::TriangulationFailed { .. } => "triangulationFailed",
            CompileWarning::EmptyFloor { .. } => "emptyFloor",
        };
        WarningView {
            code: code.to_string(),
            fatal: w.is_fatal(),
            message: w.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Simulation
// ---------------------------------------------------------------------------

/// A running simulation, driven one tick at a time from JS.
#[wasm_bindgen]
pub struct Simulation {
    sim: Sim,
    /// Interleaved `[x, y, x, y, ...]`, refilled each time positions are read.
    /// Reused so reading positions does not allocate every frame.
    xy: Vec<f32>,
    states: Vec<u8>,
    /// Present only for a scenario-driven run. `None` means the caller is
    /// placing agents itself through [`Simulation::spawn_scattered`].
    runner: Option<ScenarioRunner>,
}

#[wasm_bindgen]
impl Simulation {
    /// Build a simulation over a compiled venue.
    ///
    /// Every doorway becomes an exit. Agents leave through whichever is nearest
    /// *by walkable distance* — not straight-line distance, which would route
    /// them through walls and under-report egress time.
    #[wasm_bindgen(constructor)]
    pub fn new(venue: &CompiledVenue, floor: usize, seed: f64) -> Result<Simulation, JsError> {
        let f = venue
            .graph
            .floors
            .get(floor)
            .ok_or_else(|| JsError::new("floor index out of range"))?;

        let exits: Vec<ExitSpan> = f
            .doors
            .iter()
            .map(|d| ExitSpan { a: d.a, b: d.b })
            .collect();

        Ok(Simulation {
            sim: Sim::new(f.mesh.clone(), exits, SimParams::default(), seed as u64),
            xy: Vec::new(),
            states: Vec::new(),
            runner: None,
        })
    }

    /// Build a simulation driven by an authored scenario document.
    ///
    /// Unlike [`Simulation::new`], agents are **not** placed up front. They
    /// arrive over time according to each population's arrival curve, entering
    /// through the doorways the scenario names, carrying body radii and walking
    /// speeds drawn from its distributions. Call [`Simulation::step`] and they
    /// appear as their time comes.
    ///
    /// # Entrances are not exits
    ///
    /// Any doorway a population arrives through is removed from the exit set.
    /// Otherwise an agent placed just inside its own entrance would be counted
    /// as having left through it on the first tick, and a scenario with an
    /// arrival curve would simulate an empty room. A door that no population
    /// enters by remains an exit. If every door is an entrance the exit set
    /// would be empty, so in that case they all stay exits and the caller is
    /// told through [`Simulation::scenario_notes`].
    #[wasm_bindgen(js_name = fromScenario)]
    pub fn from_scenario(
        venue: &CompiledVenue,
        floor: usize,
        scenario_json: &str,
    ) -> Result<Simulation, JsError> {
        Self::plan_with_seed(venue, floor, scenario_json, None)
    }

    /// Run the same scenario across `runs` seeds and report how long each took.
    ///
    /// # Why this is not one run
    ///
    /// A single run is one draw from the distributions the scenario specifies —
    /// one set of walking speeds, one set of body sizes, one arrival order. It
    /// is a sample, not an answer, and the dossier already tells a reader that
    /// a submission should quote a spread rather than a figure. Until now the
    /// product could not produce one, which made that sentence an instruction
    /// the tool could not follow.
    ///
    /// Seeds are `scenario.seed + i`, so the set is reproducible from the
    /// document and run 0 is exactly what `fromScenario` gives — the number on
    /// screen is in the distribution rather than beside it.
    ///
    /// A run that has not cleared within `max_ticks` reports `f64::NAN` rather
    /// than its truncated time. An evacuation that did not finish has no
    /// evacuation time, and averaging in the moment we gave up would quietly
    /// improve the mean the longer the tool waited.
    #[wasm_bindgen(js_name = egressDistribution)]
    pub fn egress_distribution(
        venue: &CompiledVenue,
        floor: usize,
        scenario_json: &str,
        runs: u32,
        max_ticks: u32,
    ) -> Result<Vec<f64>, JsError> {
        let mut out = Vec::with_capacity(runs as usize);
        for i in 0..runs {
            let mut sim = Self::plan_with_seed(venue, floor, scenario_json, Some(i as u64))?;
            let mut cleared = f64::NAN;
            for _ in 0..max_ticks {
                sim.step();
                if sim.active_count() == 0 && sim.pending_count() == 0 {
                    cleared = sim.time();
                    break;
                }
            }
            out.push(cleared);
        }
        Ok(out)
    }

    fn plan_with_seed(
        venue: &CompiledVenue,
        floor: usize,
        scenario_json: &str,
        seed_offset: Option<u64>,
    ) -> Result<Simulation, JsError> {
        let f = venue
            .graph
            .floors
            .get(floor)
            .ok_or_else(|| JsError::new("floor index out of range"))?;
        let mut doc: ScenarioDoc = serde_json::from_str(scenario_json)
            .map_err(|e| JsError::new(&format!("scenario document is not valid: {e}")))?;

        // Shift the *document* seed, not just the simulation's. The planner
        // draws walking speeds, body sizes and placement from `doc.seed`;
        // varying only the physics seed would give every run the same crowd and
        // report a spread far narrower than the scenario actually specifies.
        doc.seed = doc.seed.wrapping_add(seed_offset.unwrap_or(0));

        let mut runner = ScenarioRunner::plan(&doc, &venue.doc, f);
        let entries = runner.entry_doors().to_vec();

        let mut exits: Vec<ExitSpan> = f
            .doors
            .iter()
            .enumerate()
            .filter(|(i, _)| !entries.contains(i))
            .map(|(_, d)| ExitSpan { a: d.a, b: d.b })
            .collect();
        if exits.is_empty() && !f.doors.is_empty() {
            runner.push_note(
                "Every doorway is an entrance, so there is nowhere to leave by. All doors are \
                 being treated as exits as well, which will delete arrivals as they appear."
                    .to_string(),
            );
            exits = f
                .doors
                .iter()
                .map(|d| ExitSpan { a: d.a, b: d.b })
                .collect();
        }

        // A timestep outside this range is a typo, not an intent: below 10 ms
        // the contact solve costs more than it buys, above 100 ms it stops
        // converging. Clamp rather than refuse, and let the caller read back
        // what was used through `timestep`.
        let dt = if doc.timestep_s.is_finite() {
            doc.timestep_s.clamp(0.01, 0.10)
        } else {
            SimParams::default().dt
        };
        let params = SimParams {
            dt,
            ..SimParams::default()
        };

        let mut out = Simulation {
            sim: Sim::new(f.mesh.clone(), exits, params, doc.seed),
            xy: Vec::new(),
            states: Vec::new(),
            runner: Some(runner),
        };
        // Anyone due at t = 0 — a preplaced crowd — is on the floor before the
        // first tick, so the venue is not briefly and misleadingly empty.
        out.admit_arrivals();
        Ok(out)
    }

    /// Apply anything the scenario schedules for now: doorway closures first,
    /// then arrivals.
    ///
    /// Closures first so nobody is admitted through a door that shut this same
    /// tick, and so an agent placed this tick plans against the geometry it
    /// will actually walk through.
    fn admit_arrivals(&mut self) {
        let now = self.sim.world.time;
        if let Some(r) = self.runner.as_mut() {
            r.apply_events(&mut self.sim, now);
            r.pump(&mut self.sim, now);
        }
    }

    /// Agents the scenario has authored but not yet admitted.
    ///
    /// A run is finished when this and the active count are both zero — an
    /// arrival curve means an empty venue at t = 0 is the *start*, not the end.
    #[wasm_bindgen(js_name = pendingCount)]
    pub fn pending_count(&self) -> u32 {
        self.runner.as_ref().map(|r| r.pending()).unwrap_or(0)
    }

    /// Agents abandoned because their entrance never cleared.
    #[wasm_bindgen(js_name = unplacedCount)]
    pub fn unplaced_count(&self) -> u32 {
        self.runner.as_ref().map(|r| r.unplaced()).unwrap_or(0)
    }

    /// Total agents the scenario asks for, across all populations.
    #[wasm_bindgen(js_name = scenarioTotal)]
    pub fn scenario_total(&self) -> u32 {
        self.runner.as_ref().map(|r| r.total()).unwrap_or(0)
    }

    /// Everything in the scenario document this engine could not act on, in
    /// prose, as a JSON array of strings.
    ///
    /// The authoring panel shows this verbatim. A scenario field that is stored
    /// and round-tripped but not simulated has to say so somewhere, or the
    /// control that edits it is a lie.
    #[wasm_bindgen(js_name = scenarioNotes)]
    pub fn scenario_notes(&self) -> Result<JsValue, JsError> {
        let notes: &[String] = self.runner.as_ref().map(|r| r.notes()).unwrap_or(&[]);
        serde_wasm_bindgen::to_value(notes).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Scatter `count` agents across the walkable floor, each routed to its
    /// nearest exit.
    ///
    /// Placement is deterministic from the seed, so the same call twice gives
    /// the same crowd.
    #[wasm_bindgen(js_name = spawnScattered)]
    pub fn spawn_scattered(&mut self, count: u32) -> u32 {
        spawn_scattered_into(&mut self.sim, count)
    }
}

/// Scatter `count` agents over a `Sim`'s walkable floor.
///
/// A free function rather than a method because `BuildingSim` needs it per
/// floor, and duplicating placement — with its area weighting and its
/// no-overlap rule — is exactly how two code paths come to disagree about what
/// a crowd looks like.
fn spawn_scattered_into(sim: &mut Sim, count: u32) -> u32 {
    let placed_count = {
        let Some(mesh) = sim.mesh() else {
            return 0;
        };

        // Sample uniformly over walkable *area*, by picking a triangle in
        // proportion to its area and then a uniform point inside it.
        //
        // An earlier version used triangle centroids plus a few interpolations
        // as candidate spots. That fails badly on simple geometry: a
        // rectangular hall triangulates into two triangles, giving eight spawn
        // points for 240 m² of floor, and only 95 of 500 agents could be
        // placed. Area-weighted sampling is independent of how the mesh
        // happens to be cut up.
        let mut tris: Vec<([Vec2; 3], f64)> = Vec::new();
        let mut total_area = 0.0;
        for (idx, t) in mesh.tri.live() {
            if !mesh.is_walkable(idx) {
                continue;
            }
            let p = [
                mesh.tri.points[t.v[0]],
                mesh.tri.points[t.v[1]],
                mesh.tri.points[t.v[2]],
            ];
            let area = ((p[1] - p[0]).cross(p[2] - p[0])).abs() * 0.5;
            total_area += area;
            tris.push((p, total_area));
        }
        if tris.is_empty() || total_area <= 0.0 {
            return 0;
        }

        let seed = sim.rng.seed();
        let rng = cf_sim::Rng::new(seed);

        // Weidmann (1993): mean free walking speed 1.34 m/s, sd 0.26.
        let speed_dist = cf_schema::Distribution::Normal {
            mean: 1.34,
            sd: 0.26,
            min: Some(0.60),
            max: Some(2.20),
        };
        let radius_dist = cf_schema::Distribution::Normal {
            mean: 0.23,
            sd: 0.02,
            min: Some(0.18),
            max: Some(0.30),
        };

        // Plan every placement while the mesh is borrowed, then spawn. Spawning
        // needs `&mut self.sim`, which cannot coexist with the `&mesh` the
        // walkability check reads.
        //
        // Placements must not overlap. A crowd that starts with bodies inside
        // one another is not a physical initial condition, and the density
        // field latches that artefact as a peak before the contact solve has
        // had a tick to separate them — which is how a 20 x 12 hall came to
        // report 26 p/m², five times the densest packing that can exist.
        let mut planned: Vec<(Vec2, f64, f64)> = Vec::with_capacity(count as usize);
        let mut placed: Vec<Vec2> = Vec::with_capacity(count as usize);
        // Bucketed by metre so the separation check stays local rather than
        // quadratic over the whole crowd.
        let mut buckets: std::collections::HashMap<(i64, i64), Vec<usize>> =
            std::collections::HashMap::new();

        for i in 0..count as u64 {
            // A few attempts per agent: enough to fill a hall evenly, bounded
            // so a venue with no room left terminates instead of spinning.
            let mut chosen: Option<(Vec2, f64, f64)> = None;
            for attempt in 0..12u64 {
                let k = i * 16 + attempt;
                // Pick a triangle in proportion to its area.
                let target = rng.uniform01(cf_sim::Stream::SpawnPoint, k, 0) * total_area;
                let ti = tris
                    .partition_point(|(_, cum)| *cum < target)
                    .min(tris.len() - 1);
                let p = tris[ti].0;

                // Uniform barycentric sample. Folding u+v > 1 back into the
                // triangle keeps the distribution uniform rather than biased
                // toward one corner.
                let mut u = rng.uniform01(cf_sim::Stream::SpawnJitter, k, 0);
                let mut v = rng.uniform01(cf_sim::Stream::SpawnJitter, k, 1);
                if u + v > 1.0 {
                    u = 1.0 - u;
                    v = 1.0 - v;
                }
                let pos = p[0] + (p[1] - p[0]) * u + (p[2] - p[0]) * v;

                let radius = radius_dist.sample_icdf(rng.uniform01(cf_sim::Stream::Radius, i, 0));
                let cx = pos.x.floor() as i64;
                let cy = pos.y.floor() as i64;
                let mut clear = true;
                'search: for dx in -1..=1 {
                    for dy in -1..=1 {
                        let Some(list) = buckets.get(&(cx + dx, cy + dy)) else {
                            continue;
                        };
                        for &j in list {
                            // Two body radii, plus a little so they start apart
                            // rather than exactly touching.
                            if placed[j].distance(pos) < radius * 2.0 + 0.02 {
                                clear = false;
                                break 'search;
                            }
                        }
                    }
                }
                if !clear {
                    continue;
                }

                let speed =
                    speed_dist.sample_icdf(rng.uniform01(cf_sim::Stream::DesiredSpeed, i, 0));
                buckets.entry((cx, cy)).or_default().push(placed.len());
                placed.push(pos);
                chosen = Some((pos, speed, radius));
                break;
            }

            let Some(plan) = chosen else { continue };
            planned.push(plan);
        }
        planned
    };

    let mut spawned = 0;
    for (pos, speed, radius) in placed_count {
        sim.spawn_to_nearest_exit(SpawnParams {
            position: pos,
            radius_m: radius as f32,
            desired_speed: speed as f32,
            goal: 0,
            population: 0,
            entry: 0,
            state: AgentState::Walking,
        });
        spawned += 1;
    }
    // Populate the density field now so a placed crowd is visible before
    // playback starts.
    sim.refresh_density();
    spawned
}

#[wasm_bindgen]
impl Simulation {
    /// The density field as bytes, one per cell, row-major.
    ///
    /// Quantised to `0..=255` over `0..maxDensity` persons/m², because this is
    /// uploaded as a texture every frame and a `Float32Array` would cost four
    /// times the bandwidth for precision no display can show. `maxDensity` is
    /// reported by [`Simulation::density_scale`] so the caller can label the
    /// legend correctly.
    #[wasm_bindgen(js_name = densityBytes)]
    pub fn density_bytes(&mut self, peak: bool) -> Vec<u8> {
        let g = self.sim.density();
        let src = if peak { g.peak() } else { g.current() };
        let scale = DENSITY_SCALE;
        src.iter()
            .map(|d| ((d / scale).clamp(0.0, 1.0) * 255.0) as u8)
            .collect()
    }

    /// Persons/m² represented by a density byte of 255.
    #[wasm_bindgen(js_name = densityScale)]
    pub fn density_scale(&self) -> f32 {
        DENSITY_SCALE
    }

    /// Density grid shape as `[cols, rows]`.
    #[wasm_bindgen(js_name = densityDims)]
    pub fn density_dims(&self) -> Vec<u32> {
        let (c, r) = self.sim.density().dims();
        vec![c as u32, r as u32]
    }

    /// Grid placement as `[originX, originY, cellSize]`, in metres.
    #[wasm_bindgen(js_name = densityPlacement)]
    pub fn density_placement(&self) -> Vec<f32> {
        let g = self.sim.density();
        let (ox, oy) = g.origin();
        vec![ox as f32, oy as f32, g.cell_size() as f32]
    }

    /// Highest density reached anywhere during the run, persons/m².
    #[wasm_bindgen(js_name = peakDensity)]
    pub fn peak_density(&self) -> f32 {
        self.sim.density().max_peak()
    }

    /// Floor area, m², that reached or exceeded the crush threshold at any
    /// point. The figure that answers "how much of my venue became dangerous".
    #[wasm_bindgen(js_name = criticalArea)]
    pub fn critical_area(&self) -> f64 {
        self.sim.density().peak_area_above(cf_sim::BAND_CRITICAL)
    }

    /// Advance one physics tick.
    ///
    /// Arrivals are admitted *before* the physics, so an agent that arrives on
    /// this tick is steered on this tick rather than standing still for one.
    pub fn step(&mut self) {
        self.admit_arrivals();
        self.sim.step();
    }

    /// Advance `n` ticks. Cheaper than `n` calls across the JS boundary.
    #[wasm_bindgen(js_name = stepMany)]
    pub fn step_many(&mut self, n: u32) {
        for _ in 0..n {
            self.step();
        }
    }

    /// Interleaved `[x, y, ...]` for every **active** agent, in metres.
    ///
    /// The returned view aliases wasm memory and is invalidated by the next
    /// allocation inside wasm. Upload it to the GPU now; do not hold it.
    pub fn positions(&mut self) -> Vec<f32> {
        self.xy.clear();
        let w = &self.sim.world;
        for i in 0..w.len() {
            if w.active[i] {
                self.xy.push(w.pos_x[i]);
                self.xy.push(w.pos_y[i]);
            }
        }
        std::mem::take(&mut self.xy)
    }

    /// One [`AgentState`] discriminant per active agent, matching the order of
    /// [`Simulation::positions`]. Used as a colour index by the renderer.
    pub fn states(&mut self) -> Vec<u8> {
        self.states.clear();
        let w = &self.sim.world;
        for i in 0..w.len() {
            if w.active[i] {
                self.states.push(w.state[i] as u8);
            }
        }
        std::mem::take(&mut self.states)
    }

    #[wasm_bindgen(js_name = activeCount)]
    pub fn active_count(&self) -> u32 {
        self.sim.world.active_count()
    }

    #[wasm_bindgen(js_name = exitedCount)]
    pub fn exited_count(&self) -> u32 {
        self.sim.world.exited_count()
    }

    /// Agents recovered from outside the mesh on the most recent tick. Should
    /// be zero; anything else means the physics is leaking.
    #[wasm_bindgen(js_name = escapedCount)]
    pub fn escaped_count(&self) -> u32 {
        self.sim.stats().escaped
    }

    #[wasm_bindgen(js_name = spawnedCount)]
    pub fn spawned_count(&self) -> u32 {
        self.sim.world.spawned_count()
    }

    /// Simulation time in seconds. Derived from the tick count, so it does not
    /// drift over a long run.
    pub fn time(&self) -> f64 {
        self.sim.world.time
    }

    /// Physics timestep actually in use, in seconds. A scenario may ask for a
    /// different one from the default, and the playback clock has to agree with
    /// the engine about how much time a tick is worth.
    pub fn timestep(&self) -> f64 {
        self.sim.params.dt
    }

    pub fn tick(&self) -> f64 {
        self.sim.world.tick as f64
    }

    /// Live counters as JSON, for the status bar.
    pub fn stats(&self) -> Result<JsValue, JsError> {
        let s = self.sim.stats();
        let view = StatsView {
            tick: s.tick as f64,
            time: s.time,
            active: s.active,
            exited: s.exited,
            spawned: s.spawned,
            blocked: s.blocked,
            max_overlap: s.max_overlap,
            escaped: s.escaped,
        };
        serde_wasm_bindgen::to_value(&view).map_err(|e| JsError::new(&e.to_string()))
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StatsView {
    tick: f64,
    time: f64,
    active: u32,
    exited: u32,
    spawned: u32,
    blocked: u32,
    max_overlap: f32,
    escaped: u32,
}

// ---------------------------------------------------------------------------
// Multi-floor
// ---------------------------------------------------------------------------

/// A whole building: one simulation per floor, joined by its stairs.
///
/// Separate from [`Simulation`] rather than folded into it. A single-floor venue
/// is every fixture in this repo and most venues anywhere, and it should not pay
/// for floors it does not have — neither in the step, where agents on different
/// floors must not appear in each other's neighbour queries, nor in the API,
/// where every call would grow a floor argument that is always zero.
///
/// The editor draws one floor at a time, so positions come back per floor.
#[wasm_bindgen]
pub struct BuildingSim {
    inner: Building,
    xy: Vec<f32>,
    states: Vec<u8>,
}

#[wasm_bindgen]
impl BuildingSim {
    /// Build from a compiled venue, one `Sim` per floor plus its links.
    ///
    /// Each floor's own doorways are its exits. A floor with none — an upper
    /// storey, typically — has no way out except the stairs, which is both
    /// correct and the reason `routeToStairs` exists.
    #[wasm_bindgen(constructor)]
    pub fn new(venue: &CompiledVenue, seed: u64) -> Result<BuildingSim, JsError> {
        if venue.graph.floors.is_empty() {
            return Err(JsError::new("venue has no compiled floors"));
        }

        let sims: Vec<Sim> = venue
            .graph
            .floors
            .iter()
            .map(|f| {
                let exits = f
                    .doors
                    .iter()
                    .map(|d| ExitSpan { a: d.a, b: d.b })
                    .collect();
                Sim::new(f.mesh.clone(), exits, SimParams::default(), seed)
            })
            .collect();

        let links = venue
            .graph
            .links
            .iter()
            .map(|l| BuildingLink {
                floor_a: l.ends[0].floor,
                point_a: l.ends[0].point,
                floor_b: l.ends[1].floor,
                point_b: l.ends[1].point,
                clear_width_m: l.clear_width_m,
                // Time on the stair, from its own speed multiplier and a
                // nominal flight length. Descending is what an evacuation
                // does, so `speed_down` is the one that counts.
                traverse_s: stair_seconds(l.speed_down),
            })
            .collect();

        Ok(BuildingSim {
            inner: Building::new(sims, links),
            xy: Vec::new(),
            states: Vec::new(),
        })
    }

    #[wasm_bindgen(js_name = floorCount)]
    pub fn floor_count(&self) -> usize {
        self.inner.floor_count()
    }

    /// Scatter `count` agents across a floor, each routed to its nearest exit.
    #[wasm_bindgen(js_name = spawnOnFloor)]
    pub fn spawn_on_floor(&mut self, floor: usize, count: u32) -> u32 {
        match self.inner.floor_mut(floor) {
            Some(sim) => spawn_scattered_into(sim, count),
            None => 0,
        }
    }

    /// Send everyone on a floor to its nearest staircase.
    ///
    /// What "evacuate" means on a storey with no doors of its own.
    #[wasm_bindgen(js_name = routeToStairs)]
    pub fn route_to_stairs(&mut self, floor: usize) -> u32 {
        self.inner.route_to_stairs(floor)
    }

    pub fn step(&mut self) {
        self.inner.step();
    }

    #[wasm_bindgen(js_name = stepMany)]
    pub fn step_many(&mut self, n: u32) {
        for _ in 0..n {
            self.inner.step();
        }
    }

    /// Interleaved `[x, y, ...]` for the agents on one floor, in metres.
    #[wasm_bindgen(js_name = positionsOnFloor)]
    pub fn positions_on_floor(&mut self, floor: usize) -> Vec<f32> {
        self.xy.clear();
        if let Some(sim) = self.inner.floor(floor) {
            let w = &sim.world;
            for i in 0..w.len() {
                if w.active[i] {
                    self.xy.push(w.pos_x[i]);
                    self.xy.push(w.pos_y[i]);
                }
            }
        }
        self.xy.clone()
    }

    /// One `AgentState` per active agent on that floor, matching `positionsOnFloor`.
    #[wasm_bindgen(js_name = statesOnFloor)]
    pub fn states_on_floor(&mut self, floor: usize) -> Vec<u8> {
        self.states.clear();
        if let Some(sim) = self.inner.floor(floor) {
            let w = &sim.world;
            for i in 0..w.len() {
                if w.active[i] {
                    self.states.push(w.state[i] as u8);
                }
            }
        }
        self.states.clone()
    }

    /// People on a staircase: off one floor and not yet on the next.
    ///
    /// A run is over when the building is empty **and** this is zero. Without
    /// it a building reports itself evacuated while a stair is still full.
    #[wasm_bindgen(js_name = inTransit)]
    pub fn in_transit(&self) -> usize {
        self.inner.in_transit()
    }

    /// How many have used each staircase, in the order `links` were compiled.
    pub fn crossings(&self) -> Vec<u32> {
        self.inner.crossings().to_vec()
    }

    pub fn time(&self) -> f64 {
        self.inner.time()
    }

    /// Counters across every floor, including people on the stairs.
    pub fn stats(&self) -> Result<JsValue, JsError> {
        let s = self.inner.stats();
        let view = StatsView {
            tick: s.tick as f64,
            time: s.time,
            active: s.active,
            exited: s.exited,
            spawned: s.spawned,
            blocked: s.blocked,
            max_overlap: s.max_overlap,
            escaped: s.escaped,
        };
        serde_wasm_bindgen::to_value(&view).map_err(|e| JsError::new(&e.to_string()))
    }
}

/// Seconds to walk a flight, from its speed multiplier.
///
/// A storey is about 3.5 m of rise, and a stair at the Green Guide's 66
/// persons/m/min is walked at roughly 0.8 of level pace. That gives a flight of
/// about 12 s, which is the figure hydraulic calculations use. Derived rather
/// than hardcoded so a document that states its own multiplier changes it.
fn stair_seconds(speed_multiplier: f64) -> f64 {
    const FLIGHT_LENGTH_M: f64 = 12.0;
    const LEVEL_SPEED: f64 = 1.34;
    let v = (LEVEL_SPEED * speed_multiplier.clamp(0.05, 2.0)).max(0.05);
    FLIGHT_LENGTH_M / v
}
