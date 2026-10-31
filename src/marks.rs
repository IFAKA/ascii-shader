//! Marks layer (DESIGN.md section 3): anchor points after Garcia et al. 2021, section 4.
//!
//! Every pixel finds the nearest jittered grid point in object space, at a few grid levels chosen from the
//! on-screen size of the surface (fractalization). The point's hash is a stable id that moves with the surface.
//! Pixels vote per id; an id with enough covered area becomes a mark, drawn by a dither test pinned to the id.
use crate::raster::{GBuffer, NO_OBJ};
use crate::scene::Camera;
use glam::Vec3;
use rayon::prelude::*;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum MarkStyle {
    Stipple,
    Hatch,
}

/// Per-anchor memory (hysteresis).
#[derive(Clone, Copy, Default)]
struct AnchorMem {
    last_seen: u32,
    drawn: bool,
    big: bool,
    band: u8,
    orient: u8,
    /// Cell chosen last time it was a candidate (x, y).
    cell: (i32, i32),
    cell_frame: u32,
}

/// Memory across frames (per-anchor hysteresis: threshold state, glyph band, cell owner).
#[derive(Default)]
pub struct MarkState {
    frame: u32,
    anchors: HashMap<u64, AnchorMem, IdHash>,
    /// Anchor id that owned each cell last frame (`None` if empty).
    owner: Vec<Option<u64>>,
}

#[derive(Clone, Debug)]
pub struct MarkParams {
    /// Base grid frequency of the anchor grid (object-space units^-1, before the 2^level scaling).
    pub f: f32,
    /// Levels k in [-n, n].
    pub n_levels: i32,
    /// Maximum anchor weight after the Hermite step.
    pub w_max: f32,
    /// Overall density multiplier (anchor draws if darkness * density > r).
    pub density: f32,
    pub style: MarkStyle,
    /// Use every `stride`-th G-buffer pixel in x and y.
    pub stride: usize,
}

impl Default for MarkParams {
    fn default() -> Self {
        MarkParams { f: 0.4, n_levels: 1, w_max: 1.0, density: 2.0, style: MarkStyle::Stipple, stride: 1 }
    }
}

pub struct MarkOutput {
    /// `cols * rows` mark glyph per cell, or None.
    pub glyphs: Vec<Option<char>>,
    /// Object index per mark cell (`NO_OBJ` where none).
    pub obj: Vec<u16>,
    /// `(anchor id, cell index, glyph)` for the respell metric.
    pub ids: Vec<(u64, usize, char)>,
    /// Anchor weight 0..1 per cell (0 where none).
    pub weight: Vec<f32>,
}

/// Ids are already well mixed hashes, so the map hasher passes them through.
#[derive(Default)]
struct IdHasher(u64);
impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, b: &[u8]) {
        for &x in b {
            self.0 = (self.0 << 8) ^ x as u64;
        }
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = v;
    }
}
type IdHash = BuildHasherDefault<IdHasher>;

#[inline]
fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58476d1ce4e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

#[inline]
fn cell_hash(obj: u16, level: i32, c: [i32; 3]) -> u64 {
    let mut h = mix(obj as u64 ^ ((level as i64 as u64 & 0xffff) << 16) ^ 0x9e37_79b9_0000_0000);
    for v in c {
        h = mix(h ^ (v as u32 as u64).wrapping_mul(0x9e3779b97f4a7c15));
    }
    h
}

/// Jitter in [0,1)^3 from a cell hash.
#[inline]
fn jitter(h: u64) -> [f32; 3] {
    let m = (1u64 << 21) - 1;
    let s = 1.0 / (1u64 << 21) as f32;
    [(h & m) as f32 * s, ((h >> 21) & m) as f32 * s, ((h >> 42) & m) as f32 * s]
}

/// Stable random in [0,1) for an anchor id (independent of the jitter bits).
#[inline]
fn id_rand(id: u64) -> f32 {
    (mix(id ^ 0x5851_f42d_4c95_7f2d) >> 40) as f32 / (1u64 << 24) as f32
}

/// Nearest jittered point of the 3x3x3 neighbourhood for grid frequency `inv = 1/cellsize`.
/// Returns the winning integer cell.
#[inline]
fn nearest_cell(p: Vec3, inv: f32, obj: u16, level: i32) -> [i32; 3] {
    let q = p * inv;
    let base = [q.x.floor() as i32, q.y.floor() as i32, q.z.floor() as i32];
    let mut best = f32::MAX;
    let mut win = base;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let c = [base[0] + dx, base[1] + dy, base[2] + dz];
                let j = jitter(cell_hash(obj, level, c));
                let ex = c[0] as f32 + j[0] - q.x;
                let ey = c[1] as f32 + j[1] - q.y;
                let ez = c[2] as f32 + j[2] - q.z;
                let d = ex * ex + ey * ey + ez * ez;
                if d < best {
                    best = d;
                    win = c;
                }
            }
        }
    }
    win
}

/// Per-anchor accumulator over all pixels voting for it.
#[derive(Clone, Copy, Default)]
struct Acc {
    obj: u16,
    /// Sum of level weight * (pixel object area / cell area): covered fraction of the grid cell.
    cov: f32,
    /// Sum of level weights.
    w: f32,
    x: f32,
    y: f32,
    light: f32,
    c2: f32,
    s2: f32,
}

type AccMap = HashMap<u64, Acc, IdHash>;

#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

const TANGENT_A: Vec3 = Vec3::new(0.80, 0.35, 0.49);
const TANGENT_B: Vec3 = Vec3::new(-0.30, 0.55, 0.78);
/// Coverage fraction at which an anchor reaches full weight (and where it starts).
const COV_LO: f32 = 0.08;
const COV_HI: f32 = 0.45;
/// Anchors below this weight are not drawn (hysteresis band around it).
const W_DRAW: f32 = 0.12;
const BAND: f32 = 0.05;
/// Score bonus for last frame's owner of a cell.
const OWNER_BONUS: f32 = 0.2;
/// How far (in cells) a mark point may leave its previous cell before it moves to a new cell.
const CELL_MARGIN: f32 = 0.3;
/// Frames an unseen anchor stays in the state.
const KEEP_FRAMES: u32 = 45;

fn vote_row(gb: &GBuffer, params: &MarkParams, cy: usize) -> AccMap {
    let mut map = AccMap::default();
    let n = params.n_levels.max(0);
    let sigma = (0.4 * n as f32).max(0.2);
    let st = params.stride.max(1);
    let y0 = cy * gb.sy;
    let mut y = y0;
    while y < y0 + gb.sy {
        let mut x = 0;
        while x < gb.w {
            let i = gb.idx(x, y);
            let o = gb.obj[i];
            if o != NO_OBJ
                && let Some((dpx, dpy)) = gradients(gb, x, y, i)
            {
                vote_pixel(gb, params, &mut map, i, o, (dpx, dpy), n, sigma, (x, y), st);
            }
            x += st;
        }
        y += st;
    }
    map
}

/// Object-space position change to a neighbour pixel in x and y (same surface only).
#[inline]
fn gradients(gb: &GBuffer, x: usize, y: usize, i: usize) -> Option<(Vec3, Vec3)> {
    let d0 = gb.depth[i];
    let ok = |j: usize| gb.obj[j] == gb.obj[i] && (gb.depth[j] - d0).abs() < 0.08 * d0;
    let jx = if x + 1 < gb.w && ok(i + 1) {
        i + 1
    } else if x > 0 && ok(i - 1) {
        i - 1
    } else {
        return None;
    };
    let jy = if y + 1 < gb.h && ok(i + gb.w) {
        i + gb.w
    } else if y > 0 && ok(i - gb.w) {
        i - gb.w
    } else {
        return None;
    };
    let dx = (gb.pos[jx] - gb.pos[i]) * if jx > i { 1.0 } else { -1.0 };
    let dy = (gb.pos[jy] - gb.pos[i]) * if jy > i { 1.0 } else { -1.0 };
    Some((dx, dy))
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn vote_pixel(gb: &GBuffer, params: &MarkParams, map: &mut AccMap, i: usize, o: u16, d: (Vec3, Vec3), n: i32, sigma: f32, xy: (usize, usize), st: usize) {
    let (dpx, dpy) = d;
    let area = dpx.cross(dpy).length() * (st * st) as f32;
    // Object-space size of one terminal cell, as the geometric mean of both directions.
    let g = (dpx.length() * gb.sx as f32 * dpy.length() * gb.sy as f32).sqrt();
    if g <= 1e-9 || area <= 0.0 {
        return;
    }
    let lr = g.log2();
    let l0 = lr.floor() as i32;
    // Fixed object-space tangent, projected into the surface plane, then to screen: solve J d = t.
    let nrm = dpx.cross(dpy).normalize_or_zero();
    let mut t = TANGENT_A - nrm * TANGENT_A.dot(nrm);
    if t.length_squared() < 0.15 {
        t = TANGENT_B - nrm * TANGENT_B.dot(nrm);
    }
    let (a, b, c) = (dpx.dot(dpx), dpx.dot(dpy), dpy.dot(dpy));
    let det = a * c - b * b;
    let (c2, s2) = if det > 1e-20 {
        let (r0, r1) = (dpx.dot(t), dpy.dot(t));
        let (ux, uy) = ((c * r0 - b * r1) / det, (a * r1 - b * r0) / det);
        let th = uy.atan2(ux) * 2.0;
        (th.cos(), th.sin())
    } else {
        (0.0, 0.0)
    };
    let p = gb.pos[i];
    let light = gb.light[i];
    let mut ws = [0.0f32; 9];
    let mut sum = 0.0;
    for k in -n..=n {
        let dl = (l0 + k) as f32 - lr;
        let w = (-dl * dl / (2.0 * sigma * sigma)).exp();
        ws[(k + n) as usize % 9] = w;
        sum += w;
    }
    for k in -n..=n {
        let w = ws[(k + n) as usize % 9] / sum;
        if w < 0.01 {
            continue;
        }
        let l = (l0 + k).clamp(-14, 14);
        let size = (l as f32).exp2() / params.f;
        let cell = nearest_cell(p, 1.0 / size, o, l);
        let id = cell_hash(o, l, cell);
        let e = map.entry(id).or_default();
        e.obj = o;
        e.cov += w * area / (size * size);
        e.w += w;
        e.x += w * (xy.0 as f32 + 0.5);
        e.y += w * (xy.1 as f32 + 0.5);
        e.light += w * light;
        e.c2 += w * c2;
        e.s2 += w * s2;
    }
}

struct Cand {
    id: u64,
    cell: usize,
    obj: u16,
    weight: f32,
    darkness: f32,
    angle: f32,
}

pub fn render_marks(gb: &GBuffer, _cam: &Camera, params: &MarkParams, state: &mut MarkState, cols: usize, rows: usize) -> MarkOutput {
    let ncell = cols * rows;
    let mut out = MarkOutput { glyphs: vec![None; ncell], obj: vec![NO_OBJ; ncell], ids: vec![], weight: vec![0.0; ncell] };
    state.frame += 1;
    let frame = state.frame;
    if state.owner.len() != ncell {
        state.owner = vec![None; ncell];
    }

    // 1. Vote: parallel over cell rows, then merge.
    let maps: Vec<AccMap> = (0..gb.rows).into_par_iter().map(|cy| vote_row(gb, params, cy)).collect();
    let mut all = AccMap::default();
    for m in maps {
        for (id, a) in m {
            let e = all.entry(id).or_default();
            e.obj = a.obj;
            e.cov += a.cov;
            e.w += a.w;
            e.x += a.x;
            e.y += a.y;
            e.light += a.light;
            e.c2 += a.c2;
            e.s2 += a.s2;
        }
    }

    // 2. Anchors -> candidates (sorted by id so everything is deterministic).
    let mut cands: Vec<Cand> = Vec::new();
    let mut keys: Vec<u64> = all.keys().copied().collect();
    keys.sort_unstable();
    for id in keys {
        let a = all[&id];
        if a.w <= 0.0 {
            continue;
        }
        let weight = smoothstep(COV_LO, COV_HI, a.cov) * params.w_max;
        let mem = state.anchors.entry(id).or_default();
        let prev_seen = mem.last_seen;
        mem.last_seen = frame;
        let thr = if mem.drawn { W_DRAW - BAND * 0.5 } else { W_DRAW + BAND * 0.5 };
        if weight < thr {
            mem.drawn = false;
            continue;
        }
        let (px, py) = (a.x / a.w, a.y / a.w);
        let (fx, fy) = (px / gb.sx as f32, py / gb.sy as f32);
        let (mut cx, mut cy) = ((fx as i32).min(cols as i32 - 1), (fy as i32).min(rows as i32 - 1));
        // Cell hysteresis: stay in last frame's cell while the point is within a margin of it.
        if prev_seen + 1 == frame && mem.cell_frame + 1 == frame {
            let (pcx, pcy) = mem.cell;
            if fx > pcx as f32 - CELL_MARGIN && fx < (pcx + 1) as f32 + CELL_MARGIN && fy > pcy as f32 - CELL_MARGIN && fy < (pcy + 1) as f32 + CELL_MARGIN {
                (cx, cy) = (pcx, pcy);
            }
        }
        mem.cell = (cx, cy);
        mem.cell_frame = frame;
        let (cx, cy) = (cx as usize, cy as usize);
        let darkness = 1.0 - a.light / a.w;
        // Dither pinned to the anchor, with hysteresis.
        let t = darkness * params.density;
        let r = id_rand(id);
        let draw = if mem.drawn { t > r - BAND } else { t > r + BAND };
        mem.drawn = draw;
        if !draw {
            continue;
        }
        cands.push(Cand { id, cell: cy * cols + cx, obj: a.obj, weight, darkness, angle: 0.5 * a.s2.atan2(a.c2) });
    }

    // 3. Cell conflict: weight + bonus for last frame's owner.
    let mut best: HashMap<usize, (f32, usize)> = HashMap::new();
    for (ci, c) in cands.iter().enumerate() {
        let score = c.weight + if state.owner[c.cell] == Some(c.id) { OWNER_BONUS } else { 0.0 };
        let e = best.entry(c.cell).or_insert((f32::MIN, ci));
        if score > e.0 {
            *e = (score, ci);
        }
    }
    let mut owner = vec![None; ncell];
    let mut winners: Vec<usize> = best.values().map(|v| v.1).collect();
    winners.sort_unstable();

    // 4. Glyphs.
    const BANDS: [f32; 3] = [0.0, 0.40, 0.65];
    for ci in winners {
        let c = &cands[ci];
        let mem = state.anchors.get_mut(&c.id).unwrap();
        let big = if mem.big { c.weight > 0.5 - BAND } else { c.weight > 0.5 + BAND };
        mem.big = big;
        let ch = match params.style {
            MarkStyle::Stipple => {
                if !big {
                    '.'
                } else {
                    // Darkness bands with hysteresis: 0 '.', 1 ':', 2 ';'.
                    let mut b = mem.band as usize;
                    if b < 2 && c.darkness > BANDS[b + 1] + BAND {
                        b += 1;
                    }
                    if b > 0 && c.darkness < BANDS[b] - BAND {
                        b -= 1;
                    }
                    mem.band = b as u8;
                    ['.', ':', ';'][b]
                }
            }
            MarkStyle::Hatch => {
                if !big {
                    '.'
                } else {
                    let deg = c.angle.to_degrees().rem_euclid(180.0);
                    let prev = mem.orient as f32 * 45.0;
                    let dist = (deg - prev + 90.0).rem_euclid(180.0) - 90.0;
                    if dist.abs() > 22.5 + 7.0 || mem.orient > 3 {
                        mem.orient = ((deg / 45.0).round() as u8) % 4;
                    }
                    ['-', '\\', '|', '/'][mem.orient as usize]
                }
            }
        };
        out.glyphs[c.cell] = Some(ch);
        out.obj[c.cell] = c.obj;
        out.weight[c.cell] = c.weight;
        out.ids.push((c.id, c.cell, ch));
        owner[c.cell] = Some(c.id);
    }
    state.owner = owner;

    // 5. Forget anchors not seen for a while.
    if frame.is_multiple_of(16) {
        state.anchors.retain(|_, m| frame - m.last_seen < KEEP_FRAMES);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_point_same_id() {
        let p = Vec3::new(1.23, 0.4, -2.7);
        let a = nearest_cell(p, 2.0, 3, 1);
        let b = nearest_cell(p, 2.0, 3, 1);
        assert_eq!(a, b);
        assert_eq!(cell_hash(3, 1, a), cell_hash(3, 1, b));
        assert_ne!(cell_hash(3, 1, a), cell_hash(4, 1, a));
        assert_ne!(cell_hash(3, 1, a), cell_hash(3, 2, a));
    }

    #[test]
    fn translated_camera_keeps_ids_of_static_scene() {
        use crate::raster::rasterize;
        let scene = crate::scene::Scene::shapes();
        let params = MarkParams { density: 3.0, ..Default::default() };
        let cam = scene.bench_camera(120);
        let mut cam2 = cam;
        cam2.pos += cam.right() * 0.02;
        let ids = |c: &Camera| {
            let gb = rasterize(&scene, c, 80, 25, 4, 8);
            let mut st = MarkState::default();
            let o = render_marks(&gb, c, &params, &mut st, 80, 25);
            o.ids.iter().map(|x| x.0).collect::<std::collections::HashSet<u64>>()
        };
        let (a, b) = (ids(&cam), ids(&cam2));
        assert!(a.len() > 20);
        let common = a.intersection(&b).count() as f32;
        assert!(common / a.len() as f32 > 0.6, "only {} of {} ids kept", common, a.len());
    }

    #[test]
    fn hysteresis_stops_blinking() {
        // A test value hovering +-0.02 around the threshold must not toggle once drawn.
        let r = 0.5;
        for start in [false, true] {
            let mut drawn = start;
            let mut toggles = 0;
            for f in 0..100 {
                let t = 0.5 + if f % 2 == 0 { 0.02 } else { -0.02 };
                let d = if drawn { t > r - BAND } else { t > r + BAND };
                if d != drawn {
                    toggles += 1;
                }
                drawn = d;
            }
            assert!(toggles <= 1);
        }
    }

    #[test]
    fn fractalization_keeps_mark_density() {
        use crate::raster::rasterize;
        use crate::scene::{Camera, Scene, plane};
        let scene = Scene::from_obj(plane(2.5, 2.5, 4));
        let params = MarkParams { density: 20.0, ..Default::default() };
        let mut dens = vec![];
        for dist in [0.5f32, 1.0, 2.0, 3.0] {
            let cam = Camera::look_at(Vec3::new(0.0, dist * 0.7, dist), Vec3::ZERO);
            let gb = rasterize(&scene, &cam, 160, 50, 4, 8);
            let mut st = MarkState::default();
            let o = render_marks(&gb, &cam, &params, &mut st, 160, 50);
            let full = |x: usize, y: usize| gb.cell_coverage(x, y) > 0.99;
            let covered = (0..50).flat_map(|y| (0..160).map(move |x| (x, y))).filter(|&(x, y)| full(x, y)).count();
            let marks = (0..50 * 160).filter(|&i| o.glyphs[i].is_some() && full(i % 160, i / 160)).count();
            dens.push(marks as f32 / covered as f32);
        }
        let (lo, hi) = (dens.iter().cloned().fold(f32::MAX, f32::min), dens.iter().cloned().fold(0.0, f32::max));
        assert!(hi / lo < 2.0, "marks per covered cell {:?}", dens);
        eprintln!("marks per covered cell at distance 0.5, 1, 2, 3: {:?}", dens);
    }
}
