//! Scenario execution — turning an authored [`ScenarioDoc`] into agents.
//!
//! # Why this lives in `cf-wasm` and not in `cf-sim`
//!
//! `cf-sim` deliberately carries no `serde` and no dependency on `cf-schema`:
//! it consumes a compiled `NavGraph` and knows nothing about documents. That is
//! what keeps the wasm bundle small and the determinism surface narrow. So the
//! translation from *document* to *agents* belongs on the document side of that
//! line, which is here.
//!
//! The consequence, stated plainly because it will matter later: a native
//! runner (`cf-native`, when it exists) must share this module rather than
//! reimplement it, or the browser preview and the server report will disagree
//! about what an arrival curve means. Nothing enforces that yet.
//!
//! # Determinism
//!
//! Every random draw goes through [`Rng`] keyed by `(stream, agent key,
//! attempt)`. A retry re-jitters *that* agent without disturbing anyone else's
//! draws, so the crowd is reproducible from the seed regardless of how
//! congested an entrance happened to be.
//!
//! # What is honoured and what is not
//!
//! [`ScenarioRunner::notes`] lists, in prose, every authored field this runner
//! could not act on. The panel shows that list verbatim. A control that looks
//! real but does nothing is worse than no control, so anything unimplemented is
//! reported rather than silently dropped.

use cf_compile::FloorMesh;
use cf_geom::Vec2;
use cf_schema::scenario::{Arrival, EventKind, Goal, Population, ScenarioDoc};
use cf_schema::venue::VenueDoc;
use cf_schema::Polygon;
use cf_sim::world::{AgentState, SpawnParams};
use cf_sim::{Rng, Sim, Stream};
use std::collections::HashMap;

/// How far inside a doorway an arriving agent is placed, metres.
///
/// Must exceed [`cf_sim::SimParams::exit_radius`], or an agent would be counted
/// as having left through the very door it just came in by.
const ENTRY_SETBACK_M: f64 = 1.30;

/// Extra depth the staging band extends beyond the setback, metres. Arrivals
/// spread over a shallow band rather than a line so a busy door does not stall
/// on one occupied point.
const ENTRY_DEPTH_M: f64 = 1.20;

/// Fraction of a doorway's width usable for lateral placement. Less than 1 so
/// bodies do not start clipped into the jambs.
const ENTRY_SPAN_FRAC: f64 = 0.80;

/// Attempts before an arrival is abandoned.
///
/// An arrival whose spot is occupied is retried on later ticks — a queue
/// building outside a door is the physically meaningful behaviour. The cap
/// exists so a venue with genuinely no room terminates instead of holding a
/// growing backlog forever.
const MAX_ATTEMPTS: u32 = 48;

/// Where an agent is to be placed. Both variants re-sample on retry, so a
/// rejected arrival tries a different spot rather than the same occupied one.
#[derive(Clone, Copy, Debug)]
enum Spot {
    /// Somewhere just inside doorway `usize`.
    Entry(usize),
    /// Somewhere inside area sampler `usize`.
    Area(usize),
}

/// Where an agent is heading. Resolved once per population.
#[derive(Clone, Copy, Debug)]
enum PlannedGoal {
    /// Whichever exit is nearest by walkable distance, decided at spawn time.
    NearestExit,
    Point(Vec2),
}

/// One agent, fully planned before the run starts.
#[derive(Clone, Debug)]
struct Planned {
    at_s: f64,
    spot: Spot,
    goal: PlannedGoal,
    radius_m: f32,
    speed: f32,
    population: u16,
    /// RNG entity key. Unique per agent across the whole scenario.
    key: u64,
    attempts: u32,
}

/// A doorway used as an entrance.
#[derive(Clone, Copy, Debug)]
struct EntrySite {
    /// Doorway midpoint.
    mid: Vec2,
    /// Unit vector along the doorway span.
    along: Vec2,
    /// Unit vector pointing into walkable space.
    inward: Vec2,
    half_width: f64,
}

/// Area-weighted sampler over a set of walkable triangles.
///
/// Sampling by *area* rather than by triangle keeps placement uniform however
/// the mesh happens to be cut up — a rectangular hall is two triangles, and
/// picking triangles uniformly would put half the crowd in each regardless of
/// their size.
#[derive(Clone, Debug)]
struct AreaSampler {
    /// `(vertices, cumulative area)`, ascending.
    tris: Vec<([Vec2; 3], f64)>,
    total: f64,
}

impl AreaSampler {
    /// Build over every walkable triangle whose centroid passes `accept`.
    fn build(mesh: &FloorMesh, accept: impl Fn(Vec2) -> bool) -> Self {
        let mut tris = Vec::new();
        let mut total = 0.0;
        let m = &mesh.mesh;
        for (idx, t) in m.tri.live() {
            if !m.is_walkable(idx) {
                continue;
            }
            let p = [
                m.tri.points[t.v[0]],
                m.tri.points[t.v[1]],
                m.tri.points[t.v[2]],
            ];
            let centroid = (p[0] + p[1] + p[2]) * (1.0 / 3.0);
            if !accept(centroid) {
                continue;
            }
            let area = ((p[1] - p[0]).cross(p[2] - p[0])).abs() * 0.5;
            if area <= 0.0 {
                continue;
            }
            total += area;
            tris.push((p, total));
        }
        Self { tris, total }
    }

    fn is_empty(&self) -> bool {
        self.tris.is_empty() || self.total <= 0.0
    }

    /// A uniform point over the sampled area.
    fn sample(&self, rng: &Rng, key: u64, attempt: u64) -> Option<Vec2> {
        if self.is_empty() {
            return None;
        }
        let target = rng.uniform01(Stream::SpawnPoint, key, attempt) * self.total;
        let i = self
            .tris
            .partition_point(|(_, cum)| *cum < target)
            .min(self.tris.len() - 1);
        let p = self.tris[i].0;

        // Uniform barycentric sample. Folding `u + v > 1` back into the
        // triangle keeps the distribution uniform instead of biased to a corner.
        let mut u = rng.uniform01(Stream::SpawnJitter, key, attempt * 2);
        let mut v = rng.uniform01(Stream::SpawnJitter, key, attempt * 2 + 1);
        if u + v > 1.0 {
            u = 1.0 - u;
            v = 1.0 - v;
        }
        Some(p[0] + (p[1] - p[0]) * u + (p[2] - p[0]) * v)
    }
}

/// Schedules a scenario's arrivals against a running [`Sim`].
pub struct ScenarioRunner {
    planned: Vec<Planned>,
    /// Index of the first arrival not yet due.
    cursor: usize,
    /// Arrivals whose spot was occupied, retried next tick.
    deferred: Vec<Planned>,
    entries: Vec<EntrySite>,
    areas: Vec<AreaSampler>,
    notes: Vec<String>,
    rng: Rng,
    /// Arrivals abandoned because there was no room for them.
    unplaced: u32,
    /// Doorways this scenario uses as entrances, by index into the floor's
    /// door list. They are excluded from the exit set — a door being used as
    /// an entrance is not an egress route in this scenario, and leaving it in
    /// would delete arrivals the instant they appeared.
    entry_doors: Vec<usize>,
    /// Doorways due to be shut, sorted by time, with a cursor into them.
    ///
    /// Held as the doorway's *endpoints* rather than an index into `Sim::exits`,
    /// because closing one removes it from that list and renumbers everything
    /// after it. Two closures in a run would then shut the wrong door, and the
    /// run would still look plausible.
    closures: Vec<Closure>,
    closure_cursor: usize,
}

/// A doorway scheduled to shut, identified by where it is.
#[derive(Clone, Copy, Debug)]
struct Closure {
    at_s: f64,
    a: Vec2,
    b: Vec2,
}

impl ScenarioRunner {
    /// Plan every arrival in `doc` against a compiled floor.
    ///
    /// All the work happens here rather than during the run: an arrival curve
    /// is inverted once, body radii and walking speeds are drawn once, and the
    /// step loop is left with nothing to do but place agents whose time has
    /// come.
    pub fn plan(doc: &ScenarioDoc, venue: &VenueDoc, mesh: &FloorMesh) -> Self {
        let rng = Rng::new(doc.seed);
        let mut notes = Vec::new();
        let mut entries: Vec<EntrySite> = Vec::new();
        let mut entry_doors: Vec<usize> = Vec::new();
        let mut areas: Vec<AreaSampler> = Vec::new();
        let mut planned: Vec<Planned> = Vec::new();

        // Door lookup by opening id. The compiled floor keeps the opening id on
        // every door node, which is the only link back to the authored document.
        let mut door_of: HashMap<&str, usize> = HashMap::new();
        for (i, d) in mesh.doors.iter().enumerate() {
            door_of.insert(d.opening.as_str(), i);
        }

        let floor_zones: Vec<(&str, &Polygon)> = venue
            .floors
            .iter()
            .flat_map(|f| f.zones.iter())
            .filter(|z| !z.is_void)
            .map(|z| (z.id.as_str(), &z.polygon))
            .collect();

        // Timed events. Only doorway closures are acted on; the rest still get
        // reported, because a field that round-trips and is quietly ignored
        // makes the control that edits it a lie.
        let mut closures: Vec<Closure> = Vec::new();
        let mut unhandled = 0usize;
        for ev in &doc.events {
            match &ev.event {
                EventKind::CloseOpening { target } => match door_of.get(target.as_str()) {
                    Some(&i) => {
                        let d = &mesh.doors[i];
                        closures.push(Closure {
                            at_s: ev.at_s,
                            a: d.a,
                            b: d.b,
                        });
                    }
                    None => notes.push(format!(
                        "event at {:.0} s closes opening '{}', which is not on this floor.",
                        ev.at_s, target
                    )),
                },
                _ => unhandled += 1,
            }
        }
        closures.sort_by(|x, y| x.at_s.total_cmp(&y.at_s));
        if unhandled > 0 {
            notes.push(format!(
                "{unhandled} timed event(s) are not applied — only doorway closures are modelled."
            ));
        }

        for (pi, pop) in doc.populations.iter().enumerate() {
            let pop_idx = pi as u16;
            let goal = resolve_goal(pop, venue, mesh, &door_of, &mut notes);
            note_unmodelled_profile(pop, &mut notes);

            // Where this population comes from, as a list of (spot, weight).
            let mut sources: Vec<(Spot, f64)> = Vec::new();
            match &pop.arrival {
                Arrival::Curve { entries: ew, .. } | Arrival::Uniform { entries: ew } => {
                    for e in ew {
                        let Some(&di) = door_of.get(e.opening.as_str()) else {
                            notes.push(format!(
                                "{}: entry '{}' is not a doorway on this floor — ignored.",
                                pop.label, e.opening
                            ));
                            continue;
                        };
                        let site = entry_site(mesh, di);
                        if !entry_doors.contains(&di) {
                            entry_doors.push(di);
                        }
                        entries.push(site);
                        sources.push((Spot::Entry(entries.len() - 1), e.weight.max(0.0)));
                    }
                }
                Arrival::Preplaced { zones } => {
                    for z in zones {
                        let Some((_, poly)) =
                            floor_zones.iter().find(|(id, _)| *id == z.zone.as_str())
                        else {
                            notes.push(format!(
                                "{}: zone '{}' is not on this floor — ignored.",
                                pop.label, z.zone
                            ));
                            continue;
                        };
                        let sampler = AreaSampler::build(mesh, |c| poly.contains(c));
                        if sampler.is_empty() {
                            notes.push(format!(
                                "{}: zone '{}' covers no walkable floor — ignored.",
                                pop.label, z.zone
                            ));
                            continue;
                        }
                        areas.push(sampler);
                        sources.push((Spot::Area(areas.len() - 1), z.weight.max(0.0)));
                    }
                }
            }

            // A population with no usable source still has to go somewhere, or
            // the count the author typed silently becomes zero. Scattering over
            // the whole floor is the honest fallback and it is called out.
            if sources.is_empty() {
                let all = AreaSampler::build(mesh, |_| true);
                if all.is_empty() {
                    notes.push(format!("{}: nowhere to place anyone.", pop.label));
                    continue;
                }
                notes.push(format!(
                    "{}: no usable entry or zone, so agents are scattered over the whole floor.",
                    pop.label
                ));
                areas.push(all);
                sources.push((Spot::Area(areas.len() - 1), 1.0));
            }

            let weights: Vec<f64> = sources.iter().map(|(_, w)| *w).collect();
            let total_w: f64 = weights.iter().sum();
            let weights = if total_w > 0.0 {
                weights
            } else {
                vec![1.0; sources.len()]
            };

            for i in 0..pop.count as u64 {
                // Distinct per agent across the whole scenario, so two
                // populations never share a random draw.
                let key = ((pi as u64) << 40) | i;
                let at_s = arrival_time(&pop.arrival, i, pop.count as u64, doc.duration_s);
                let si = rng.weighted(Stream::SpawnPoint, key, 0, &weights);
                let radius =
                    pop.profile
                        .radius_m
                        .sample_icdf(rng.uniform01(Stream::Radius, key, 0));
                let speed = pop.profile.desired_speed.sample_icdf(rng.uniform01(
                    Stream::DesiredSpeed,
                    key,
                    0,
                ));

                planned.push(Planned {
                    at_s,
                    spot: sources[si.min(sources.len() - 1)].0,
                    goal,
                    radius_m: radius as f32,
                    speed: speed as f32,
                    population: pop_idx,
                    key,
                    attempts: 0,
                });
            }
        }

        // Ascending by time, ties broken by key so the order is reproducible.
        planned.sort_by(|a, b| {
            a.at_s
                .partial_cmp(&b.at_s)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.key.cmp(&b.key))
        });

        Self {
            planned,
            cursor: 0,
            deferred: Vec::new(),
            entries,
            areas,
            notes,
            rng,
            unplaced: 0,
            entry_doors,
            closures,
            closure_cursor: 0,
        }
    }

    /// Doorway indices this scenario uses as entrances.
    pub fn entry_doors(&self) -> &[usize] {
        &self.entry_doors
    }

    /// Everything the runner could not honour, in prose, for the UI.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// Record something the *caller* could not honour. Used by the binding for
    /// conditions it discovers after planning, such as a venue whose every door
    /// is an entrance.
    pub fn push_note(&mut self, note: String) {
        self.notes.push(note);
    }

    /// Agents authored but not yet in the venue.
    pub fn pending(&self) -> u32 {
        (self.planned.len() - self.cursor + self.deferred.len()) as u32
    }

    /// Agents abandoned because their entrance never cleared.
    pub fn unplaced(&self) -> u32 {
        self.unplaced
    }

    /// Total agents this scenario asks for.
    pub fn total(&self) -> u32 {
        self.planned.len() as u32
    }

    /// Place everyone whose arrival time has passed. Returns how many entered.
    ///
    /// An arrival whose spot is occupied is *deferred*, not dropped: a crowd
    /// arriving faster than a doorway can absorb should queue outside it, which
    /// is exactly the bottleneck a planner is looking for.
    /// Shut any doorway whose closure time has passed.
    ///
    /// Separate from arrivals because it is not one: `pump` returns how many
    /// people were admitted, and folding a structural change to the venue into
    /// that count would hide it. Returns how many doorways closed.
    pub fn apply_events(&mut self, sim: &mut Sim, now: f64) -> u32 {
        let mut closed = 0;
        while self.closure_cursor < self.closures.len()
            && self.closures[self.closure_cursor].at_s <= now
        {
            let c = self.closures[self.closure_cursor];
            self.closure_cursor += 1;

            // Match by position: the index into `Sim::exits` shifts as doors
            // close, but a doorway does not move.
            let found = sim
                .exits()
                .iter()
                .position(|e| e.a.distance(c.a) < 1e-6 && e.b.distance(c.b) < 1e-6);
            match found {
                Some(i) => {
                    if sim.close_exit(i) {
                        closed += 1;
                    }
                }
                None => self.notes.push(format!(
                    "a doorway due to close at {:.0} s was not in the exit set — \
                     it may already be shut, or be in use as an entrance.",
                    c.at_s
                )),
            }
        }
        closed
    }

    pub fn pump(&mut self, sim: &mut Sim, now: f64) -> u32 {
        let mut due: Vec<Planned> = std::mem::take(&mut self.deferred);
        while self.cursor < self.planned.len() && self.planned[self.cursor].at_s <= now {
            due.push(self.planned[self.cursor].clone());
            self.cursor += 1;
        }
        if due.is_empty() {
            return 0;
        }

        // One pass over the crowd to index it, rather than one per arrival.
        // Bucketed by metre so the clearance test stays local.
        let mut buckets: HashMap<(i64, i64), Vec<(Vec2, f64)>> = HashMap::new();
        let w = &sim.world;
        for i in 0..w.len() {
            if !w.active[i] {
                continue;
            }
            let p = Vec2::new(w.pos_x[i] as f64, w.pos_y[i] as f64);
            buckets
                .entry((p.x.floor() as i64, p.y.floor() as i64))
                .or_default()
                .push((p, w.radius[i] as f64));
        }

        let mut spawned = 0;
        for mut a in due {
            let radius = a.radius_m as f64;
            let mut placed = None;
            // A few spots per tick: enough to find room beside a busy door,
            // bounded so a full venue does not spin.
            for k in 0..4u32 {
                let attempt = (a.attempts + k) as u64;
                let Some(p) = self.sample_spot(sim, a.spot, a.key, attempt) else {
                    continue;
                };
                if is_clear(&buckets, p, radius) {
                    placed = Some(p);
                    break;
                }
            }

            match placed {
                Some(p) => {
                    let params = SpawnParams {
                        position: p,
                        radius_m: a.radius_m,
                        desired_speed: a.speed,
                        goal: 0,
                        population: a.population,
                        entry: 0,
                        state: AgentState::Walking,
                    };
                    match a.goal {
                        PlannedGoal::NearestExit => {
                            sim.spawn_to_nearest_exit(params);
                        }
                        PlannedGoal::Point(g) => {
                            sim.spawn(params, g);
                        }
                    }
                    buckets
                        .entry((p.x.floor() as i64, p.y.floor() as i64))
                        .or_default()
                        .push((p, radius));
                    spawned += 1;
                }
                None => {
                    a.attempts += 4;
                    if a.attempts >= MAX_ATTEMPTS {
                        self.unplaced += 1;
                    } else {
                        self.deferred.push(a);
                    }
                }
            }
        }

        if spawned > 0 {
            // So a crowd that just walked in is visible before the next tick.
            sim.refresh_density();
        }
        spawned
    }

    /// A candidate position for `spot`, or `None` if it is off the mesh.
    fn sample_spot(&self, sim: &Sim, spot: Spot, key: u64, attempt: u64) -> Option<Vec2> {
        let p = match spot {
            Spot::Area(i) => self.areas.get(i)?.sample(&self.rng, key, attempt)?,
            Spot::Entry(i) => {
                let e = self.entries.get(i)?;
                let u = self.rng.uniform01(Stream::SpawnJitter, key, attempt * 2);
                let v = self
                    .rng
                    .uniform01(Stream::SpawnJitter, key, attempt * 2 + 1);
                let lateral = (u - 0.5) * 2.0 * e.half_width * ENTRY_SPAN_FRAC;
                let depth = ENTRY_SETBACK_M + v * ENTRY_DEPTH_M;
                e.mid + e.along * lateral + e.inward * depth
            }
        };
        match sim.mesh() {
            Some(m) if m.locate(p).is_some() => Some(p),
            Some(m) => m.nearest_walkable_point(p),
            None => Some(p),
        }
    }
}

/// Is there room for a body of `radius` at `p`?
fn is_clear(buckets: &HashMap<(i64, i64), Vec<(Vec2, f64)>>, p: Vec2, radius: f64) -> bool {
    let (cx, cy) = (p.x.floor() as i64, p.y.floor() as i64);
    for dx in -1..=1 {
        for dy in -1..=1 {
            let Some(list) = buckets.get(&(cx + dx, cy + dy)) else {
                continue;
            };
            for (q, r) in list {
                // Two body radii plus a little, so they start apart rather than
                // exactly touching — a crowd that begins overlapping is not a
                // physical initial condition and the density field latches it.
                if q.distance(p) < radius + r + 0.02 {
                    return false;
                }
            }
        }
    }
    true
}

/// The staging geometry just inside a doorway.
fn entry_site(mesh: &FloorMesh, door: usize) -> EntrySite {
    let d = &mesh.doors[door];
    let mid = d.midpoint();
    let along = (d.b - d.a).normalized().unwrap_or(Vec2::new(1.0, 0.0));
    let n = along.perp();
    // Whichever side of the threshold is walkable is "inside". Probing rather
    // than assuming a winding order: the compiler does not promise one.
    let inward = match mesh.mesh.locate(mid + n * 0.15) {
        Some(_) => n,
        None => -n,
    };
    EntrySite {
        mid,
        along,
        inward,
        half_width: d.width_m * 0.5,
    }
}

/// When agent `i` of `count` arrives, in seconds.
///
/// The midpoint `(i + 0.5) / count` rather than `i / count` so the first agent
/// does not arrive at exactly the start and the last not exactly at the end —
/// a curve that puts 20% of a crowd through a door in the first minute should
/// spread them through that minute, not stack one on the threshold.
fn arrival_time(arrival: &Arrival, i: u64, count: u64, duration_s: f64) -> f64 {
    if count == 0 {
        return 0.0;
    }
    let f = (i as f64 + 0.5) / count as f64;
    match arrival {
        Arrival::Preplaced { .. } => 0.0,
        Arrival::Uniform { .. } => f * duration_s.max(0.0),
        Arrival::Curve { points, .. } => invert_curve(points, f),
    }
}

/// Read a time off a cumulative arrival curve.
///
/// `points` are `(t, cumulative fraction)`. Inverting rather than integrating:
/// the author draws "60% are in by minute 20" and the engine needs "when does
/// agent 600 of 1000 arrive", which is the same statement read the other way.
/// Segments where the fraction does not advance are skipped — a flat stretch is
/// a pause in arrivals, not a moment when everyone appears at once.
fn invert_curve(points: &[[f64; 2]], f: f64) -> f64 {
    if points.is_empty() {
        return 0.0;
    }
    if points.len() == 1 {
        return points[0][0].max(0.0);
    }
    let f = f.clamp(0.0, 1.0);
    for w in points.windows(2) {
        let (t0, f0) = (w[0][0], w[0][1]);
        let (t1, f1) = (w[1][0], w[1][1]);
        if f1 <= f0 {
            continue;
        }
        if f <= f1 {
            let local = ((f - f0) / (f1 - f0)).clamp(0.0, 1.0);
            return (t0 + (t1 - t0) * local).max(0.0);
        }
    }
    // Past the end of the curve: everyone remaining arrives at its last point.
    points[points.len() - 1][0].max(0.0)
}

/// Resolve a population's destination, reporting anything unsupported.
fn resolve_goal(
    pop: &Population,
    venue: &VenueDoc,
    mesh: &FloorMesh,
    door_of: &HashMap<&str, usize>,
    notes: &mut Vec<String>,
) -> PlannedGoal {
    if pop.itinerary.len() > 1 {
        notes.push(format!(
            "{}: only the first itinerary step is followed — multi-leg plans need per-agent \
             goal chaining, which the engine does not have yet.",
            pop.label
        ));
    }
    let Some(step) = pop.itinerary.first() else {
        return PlannedGoal::NearestExit;
    };
    if step.dwell.is_some() {
        notes.push(format!(
            "{}: dwell time is not simulated — agents hold at their goal instead of moving on.",
            pop.label
        ));
    }
    if step.probability < 1.0 {
        notes.push(format!(
            "{}: step probability {:.2} is not applied — every agent takes this step.",
            pop.label, step.probability
        ));
    }

    match &step.goal {
        Goal::NearestExit => PlannedGoal::NearestExit,
        Goal::Opening { id } => match door_of.get(id.as_str()) {
            Some(&di) => {
                let site = entry_site(mesh, di);
                PlannedGoal::Point(site.mid + site.inward * 0.15)
            }
            None => {
                notes.push(format!(
                    "{}: goal doorway '{id}' is not on this floor — using the nearest exit.",
                    pop.label
                ));
                PlannedGoal::NearestExit
            }
        },
        Goal::Zone { id } => {
            let zone = venue
                .floors
                .iter()
                .flat_map(|f| f.zones.iter())
                .find(|z| z.id.as_str() == id.as_str());
            match zone.and_then(|z| walkable_point_in(mesh, &z.polygon)) {
                Some(p) => PlannedGoal::Point(p),
                None => {
                    notes.push(format!(
                        "{}: goal zone '{id}' has no walkable floor — using the nearest exit.",
                        pop.label
                    ));
                    PlannedGoal::NearestExit
                }
            }
        }
        Goal::Waypoint { id } => {
            match venue
                .routing
                .waypoints
                .iter()
                .find(|w| w.id.as_str() == id.as_str())
            {
                Some(w) => PlannedGoal::Point(w.p),
                None => {
                    notes.push(format!(
                        "{}: goal waypoint '{id}' is not in this venue — using the nearest exit.",
                        pop.label
                    ));
                    PlannedGoal::NearestExit
                }
            }
        }
        Goal::Component { id } => {
            notes.push(format!(
                "{}: component '{id}' cannot be a goal — components are not simulated yet, so \
                 these agents head for the nearest exit.",
                pop.label
            ));
            PlannedGoal::NearestExit
        }
    }
}

/// A point on walkable floor inside `poly`, area-weighted.
///
/// The centroid is not used: an L-shaped zone's centroid can sit in the notch,
/// outside the zone and possibly inside a wall.
fn walkable_point_in(mesh: &FloorMesh, poly: &Polygon) -> Option<Vec2> {
    let s = AreaSampler::build(mesh, |c| poly.contains(c));
    if s.is_empty() {
        return None;
    }
    // The area-weighted median triangle's centroid: deterministic, and inside
    // the zone by construction.
    let half = s.total * 0.5;
    let i = s
        .tris
        .partition_point(|(_, cum)| *cum < half)
        .min(s.tris.len() - 1);
    let p = s.tris[i].0;
    Some((p[0] + p[1] + p[2]) * (1.0 / 3.0))
}

/// Report profile fields the locomotion model does not read yet.
///
/// These are authored, stored and round-tripped — they are simply not acted on.
/// Saying so is the difference between an honest tool and a demo.
fn note_unmodelled_profile(pop: &Population, notes: &mut Vec<String>) {
    let p = &pop.profile;
    let mut unused: Vec<&str> = Vec::new();
    if p.mass_kg.is_some() {
        unused.push("mass");
    }
    if p.group_size.is_some() {
        unused.push("group size");
    }
    if p.patience_s.is_some() {
        unused.push("patience");
    }
    if p.reaction_time_s.is_some() {
        unused.push("reaction time");
    }
    if p.mobility_impaired_frac > 0.0 {
        unused.push("mobility impairment");
    }
    if !pop.access.is_empty() {
        unused.push("access tags");
    }
    if !unused.is_empty() {
        notes.push(format!(
            "{}: {} not simulated — body radius and walking speed are the only per-agent \
             parameters the locomotion model reads.",
            pop.label,
            unused.join(", ")
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cf_schema::scenario::{AgentProfile, EntryWeight, ItineraryStep, ZoneWeight};
    use cf_schema::Distribution;

    fn hall() -> (VenueDoc, cf_compile::NavGraph) {
        let json = include_str!("../../../fixtures/unit/hall-two-doors.venue.json");
        let doc: VenueDoc = serde_json::from_str(json).expect("fixture parses");
        let graph = cf_compile::compile(&doc);
        (doc, graph)
    }

    fn scenario(pop: Population, duration_s: f64) -> ScenarioDoc {
        ScenarioDoc {
            schema_version: cf_schema::SCENARIO_SCHEMA_VERSION.to_string(),
            id: "scn_test".into(),
            name: "test".into(),
            venue_version: "ver_test".into(),
            mode: Default::default(),
            duration_s,
            timestep_s: 0.05,
            seed: 20260801,
            populations: vec![pop],
            events: Vec::new(),
            compliance: None,
            output: Default::default(),
        }
    }

    fn population(count: u32, arrival: Arrival, itinerary: Vec<ItineraryStep>) -> Population {
        Population {
            id: "pop_t".into(),
            label: "Test".into(),
            count,
            profile: AgentProfile::default(),
            arrival,
            itinerary,
            access: Vec::new(),
        }
    }

    fn sim_for(graph: &cf_compile::NavGraph, exclude: &[usize]) -> Sim {
        let f = &graph.floors[0];
        let exits: Vec<cf_sim::ExitSpan> = f
            .doors
            .iter()
            .enumerate()
            .filter(|(i, _)| !exclude.contains(i))
            .map(|(_, d)| cf_sim::ExitSpan { a: d.a, b: d.b })
            .collect();
        Sim::new(
            f.mesh.clone(),
            exits,
            cf_sim::SimParams::default(),
            20260801,
        )
    }

    #[test]
    fn preplaced_agents_all_arrive_at_t_zero() {
        let (doc, graph) = hall();
        let scn = scenario(
            population(
                60,
                Arrival::Preplaced {
                    zones: vec![ZoneWeight {
                        zone: "z_hall".into(),
                        weight: 1.0,
                    }],
                },
                Vec::new(),
            ),
            300.0,
        );
        let mut runner = ScenarioRunner::plan(&scn, &doc, &graph.floors[0]);
        let mut sim = sim_for(&graph, &[]);
        assert_eq!(runner.pending(), 60);
        let n = runner.pump(&mut sim, 0.0);
        assert_eq!(n, 60, "everyone should be placed in a 240 m² hall");
        assert_eq!(runner.pending(), 0);
    }

    #[test]
    fn a_uniform_arrival_spreads_over_the_run() {
        let (doc, graph) = hall();
        let scn = scenario(
            population(
                40,
                Arrival::Uniform {
                    entries: vec![EntryWeight {
                        opening: "op_east_door".into(),
                        weight: 1.0,
                    }],
                },
                Vec::new(),
            ),
            100.0,
        );
        let runner = ScenarioRunner::plan(&scn, &doc, &graph.floors[0]);
        let first = runner.planned.first().expect("planned").at_s;
        let last = runner.planned.last().expect("planned").at_s;
        assert!(first > 0.0 && first < 5.0, "first arrival at {first}");
        assert!(last > 95.0 && last < 100.0, "last arrival at {last}");
    }

    #[test]
    fn nobody_arrives_before_their_time() {
        let (doc, graph) = hall();
        let scn = scenario(
            population(
                40,
                Arrival::Uniform {
                    entries: vec![EntryWeight {
                        opening: "op_east_door".into(),
                        weight: 1.0,
                    }],
                },
                Vec::new(),
            ),
            100.0,
        );
        let mut runner = ScenarioRunner::plan(&scn, &doc, &graph.floors[0]);
        let mut sim = sim_for(&graph, runner.entry_doors());
        runner.pump(&mut sim, 0.0);
        assert_eq!(
            sim.stats().spawned,
            0,
            "an arrival curve must not front-load"
        );
        // Half the run should have admitted roughly half the crowd. Stepping
        // matters: without it the first arrivals stand on the threshold and
        // block everyone behind them, which is real behaviour but not what this
        // test is measuring.
        for t in 1..=1000 {
            runner.pump(&mut sim, t as f64 * 0.05);
            sim.step();
        }
        let half = sim.stats().spawned;
        assert!((15..=25).contains(&half), "{half} arrived by t = 50 s");
    }

    #[test]
    fn an_entry_door_is_not_an_exit_so_arrivals_survive() {
        let (doc, graph) = hall();
        let scn = scenario(
            population(
                20,
                Arrival::Uniform {
                    entries: vec![EntryWeight {
                        opening: "op_east_door".into(),
                        weight: 1.0,
                    }],
                },
                vec![ItineraryStep {
                    goal: Goal::Zone {
                        id: "z_hall".into(),
                    },
                    probability: 1.0,
                    dwell: None,
                }],
            ),
            40.0,
        );
        let mut runner = ScenarioRunner::plan(&scn, &doc, &graph.floors[0]);
        assert_eq!(runner.entry_doors().len(), 1);
        let mut sim = sim_for(&graph, runner.entry_doors());
        for t in 1..=1200 {
            runner.pump(&mut sim, t as f64 * 0.05);
            sim.step();
        }
        assert_eq!(sim.stats().spawned, 20);
        // They were routed into the hall, not out of the other door.
        assert!(
            sim.stats().active >= 18,
            "only {} of 20 remained in the hall",
            sim.stats().active
        );
    }

    #[test]
    fn the_curve_is_inverted_at_its_control_points() {
        let pts = vec![[0.0, 0.0], [60.0, 0.5], [120.0, 1.0]];
        assert!((invert_curve(&pts, 0.0) - 0.0).abs() < 1e-9);
        assert!((invert_curve(&pts, 0.25) - 30.0).abs() < 1e-9);
        assert!((invert_curve(&pts, 0.5) - 60.0).abs() < 1e-9);
        assert!((invert_curve(&pts, 0.75) - 90.0).abs() < 1e-9);
        assert!((invert_curve(&pts, 1.0) - 120.0).abs() < 1e-9);
    }

    #[test]
    fn a_flat_stretch_is_a_pause_not_a_surge() {
        // Nobody arrives between 30 s and 90 s.
        let pts = vec![[0.0, 0.0], [30.0, 0.5], [90.0, 0.5], [120.0, 1.0]];
        assert!((invert_curve(&pts, 0.5) - 30.0).abs() < 1e-9);
        let just_after = invert_curve(&pts, 0.51);
        assert!(
            just_after > 90.0,
            "the first arrival after the pause was at {just_after}"
        );
    }

    #[test]
    fn planning_is_reproducible_from_the_seed() {
        let (doc, graph) = hall();
        let scn = scenario(
            population(
                50,
                Arrival::Preplaced {
                    zones: vec![ZoneWeight {
                        zone: "z_hall".into(),
                        weight: 1.0,
                    }],
                },
                Vec::new(),
            ),
            60.0,
        );
        let a = ScenarioRunner::plan(&scn, &doc, &graph.floors[0]);
        let b = ScenarioRunner::plan(&scn, &doc, &graph.floors[0]);
        for (x, y) in a.planned.iter().zip(&b.planned) {
            assert_eq!(x.at_s.to_bits(), y.at_s.to_bits());
            assert_eq!(x.radius_m.to_bits(), y.radius_m.to_bits());
            assert_eq!(x.speed.to_bits(), y.speed.to_bits());
        }

        let mut sa = sim_for(&graph, &[]);
        let mut sb = sim_for(&graph, &[]);
        let (mut ra, mut rb) = (a, b);
        ra.pump(&mut sa, 0.0);
        rb.pump(&mut sb, 0.0);
        for i in 0..sa.world.len() {
            assert_eq!(sa.world.pos_x[i].to_bits(), sb.world.pos_x[i].to_bits());
            assert_eq!(sa.world.pos_y[i].to_bits(), sb.world.pos_y[i].to_bits());
        }
    }

    #[test]
    fn body_radius_follows_the_authored_distribution() {
        let (doc, graph) = hall();
        let mut pop = population(
            200,
            Arrival::Preplaced {
                zones: vec![ZoneWeight {
                    zone: "z_hall".into(),
                    weight: 1.0,
                }],
            },
            Vec::new(),
        );
        pop.profile.radius_m = Distribution::Constant { value: 0.19 };
        pop.profile.desired_speed = Distribution::Constant { value: 0.85 };
        let scn = scenario(pop, 60.0);
        let runner = ScenarioRunner::plan(&scn, &doc, &graph.floors[0]);
        assert!(runner
            .planned
            .iter()
            .all(|p| (p.radius_m - 0.19).abs() < 1e-6 && (p.speed - 0.85).abs() < 1e-6));
    }

    #[test]
    fn unsupported_authoring_is_reported_rather_than_ignored() {
        let (doc, graph) = hall();
        let mut pop = population(
            10,
            Arrival::Preplaced {
                zones: vec![ZoneWeight {
                    zone: "z_hall".into(),
                    weight: 1.0,
                }],
            },
            vec![
                ItineraryStep {
                    goal: Goal::Zone {
                        id: "z_hall".into(),
                    },
                    probability: 1.0,
                    dwell: Some(Distribution::Constant { value: 30.0 }),
                },
                ItineraryStep {
                    goal: Goal::NearestExit,
                    probability: 1.0,
                    dwell: None,
                },
            ],
        );
        pop.profile.patience_s = Some(Distribution::Constant { value: 30.0 });
        let scn = scenario(pop, 60.0);
        let runner = ScenarioRunner::plan(&scn, &doc, &graph.floors[0]);
        let joined = runner.notes().join(" | ");
        assert!(joined.contains("first itinerary step"), "{joined}");
        assert!(joined.contains("dwell"), "{joined}");
        assert!(joined.contains("patience"), "{joined}");
    }

    #[test]
    fn an_unknown_entry_is_reported_and_the_count_is_still_honoured() {
        let (doc, graph) = hall();
        let scn = scenario(
            population(
                12,
                Arrival::Uniform {
                    entries: vec![EntryWeight {
                        opening: "op_does_not_exist".into(),
                        weight: 1.0,
                    }],
                },
                Vec::new(),
            ),
            30.0,
        );
        let runner = ScenarioRunner::plan(&scn, &doc, &graph.floors[0]);
        assert_eq!(runner.total(), 12);
        assert!(runner.notes().iter().any(|n| n.contains("not a doorway")));
    }
}
