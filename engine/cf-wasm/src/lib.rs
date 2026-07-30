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

use cf_compile::{compile, CompileWarning, NavGraph};
use cf_geom::Vec2;
use cf_schema::VenueDoc;
use cf_sim::world::{AgentState, SpawnParams};
use cf_sim::{ExitSpan, Sim, SimParams};
use wasm_bindgen::prelude::*;

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
#[wasm_bindgen]
pub struct CompiledVenue {
    graph: NavGraph,
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
        })
    }

    /// Scatter `count` agents across the walkable floor, each routed to its
    /// nearest exit.
    ///
    /// Placement is deterministic from the seed, so the same call twice gives
    /// the same crowd.
    #[wasm_bindgen(js_name = spawnScattered)]
    pub fn spawn_scattered(&mut self, count: u32) -> u32 {
        let Some(mesh) = self.sim.mesh() else {
            return 0;
        };

        // Candidate points: triangle centroids, plus interpolations toward each
        // vertex so a large crowd spreads out rather than stacking on centroids.
        let mut spots: Vec<Vec2> = Vec::new();
        for (idx, t) in mesh.tri.live() {
            if !mesh.is_walkable(idx) {
                continue;
            }
            let c = mesh.centroids[idx];
            spots.push(c);
            for v in t.v {
                spots.push(c.lerp(mesh.tri.points[v], 0.55));
            }
        }
        if spots.is_empty() {
            return 0;
        }

        let seed = self.sim.rng.seed();
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
        let mut planned: Vec<(Vec2, f64, f64)> = Vec::with_capacity(count as usize);
        for i in 0..count as u64 {
            let pick = rng.below(cf_sim::Stream::SpawnPoint, i, 0, spots.len() as u64) as usize;
            let base = spots[pick];
            // Jitter so agents sharing a spot do not start exactly coincident.
            let jx = rng.uniform_range(cf_sim::Stream::SpawnJitter, i, 0, -0.35, 0.35);
            let jy = rng.uniform_range(cf_sim::Stream::SpawnJitter, i, 1, -0.35, 0.35);
            let pos = Vec2::new(base.x + jx, base.y + jy);
            if mesh.locate(pos).is_none() {
                continue;
            }

            // Sampled through cf-schema's Distribution rather than a local
            // copy: ADR 0002's point is that there is exactly one definition of
            // what "normal" means, shared by the editor's preview and the
            // engine. A second implementation here could drift from it.
            let speed = speed_dist.sample_icdf(rng.uniform01(cf_sim::Stream::DesiredSpeed, i, 0));
            let radius = radius_dist.sample_icdf(rng.uniform01(cf_sim::Stream::Radius, i, 0));
            planned.push((pos, speed, radius));
        }

        let mut spawned = 0;
        for (pos, speed, radius) in planned {
            self.sim.spawn_to_nearest_exit(SpawnParams {
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
        spawned
    }

    /// Advance one physics tick.
    pub fn step(&mut self) {
        self.sim.step();
    }

    /// Advance `n` ticks. Cheaper than `n` calls across the JS boundary.
    #[wasm_bindgen(js_name = stepMany)]
    pub fn step_many(&mut self, n: u32) {
        for _ in 0..n {
            self.sim.step();
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
