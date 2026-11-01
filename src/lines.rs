//! Lines layer (DESIGN.md section 2): feature edges from the mesh, visibility against the depth buffer, per-cell
//! path segments, glyph cost over the line set, and the typist with spelling memory.
use crate::glyphs::{Atlas, GH, GW, Glyph, LineSet};
use crate::raster::{GBuffer, NO_OBJ};
use crate::scene::{Camera, Scene};
use glam::{Vec2, Vec3};
use rayon::prelude::*;
use std::collections::HashMap;

const NO_FACE: u32 = u32::MAX;
/// Sample spacing along an edge, in cells.
const STEP: f32 = 0.25;
/// Relative / absolute depth bias for the visibility test.
const BIAS_REL: f32 = 0.02;
const BIAS_ABS: f32 = 0.02;
/// Own-object slack.
const SLOPE: f32 = 0.08;
/// Distance clamp (glyph pixels) in the cost.
const CLAMP: f32 = 4.0;
/// Entries shorter than this (glyph pixels) in a cell are dropped.
const MIN_ENTRY_LEN: f32 = 2.0;
/// A cell needs an entry at least this long to get a glyph: `MIN_NEW_LEN` if it had none last frame,
/// `MIN_KEEP_LEN` if it had one (presence hysteresis).
const MIN_NEW_LEN: f32 = 4.0;
const MIN_KEEP_LEN: f32 = 2.0;
/// Segments are pulled this fraction of the way to where the font draws a centred line before costing, so lines
/// quantise to the cell grid (plain `│ ─ ╱ ╲`) instead of hopping between sub-cell glyphs.
const PULL: f32 = 0.85;
/// Distances up to this (glyph pixels) cost nothing: strokes are about 2 px wide.
const TOL: f32 = 1.25;
/// A new line cell whose ink lies within this many glyph pixels of a side border defers to a neighbour that had the line.
const BORDER_PX: f32 = 3.0;
/// Staircase merge test slack (glyph px): wider when exactly one of the two cells drew last frame (they were merged,
/// stay merged), tighter when both did (they were doubled, stay doubled) so the predicate does not toggle.
const SLACK: f32 = 1.5;
const TIGHT: f32 = -0.7;
/// Bonus (glyph px) in the staircase duel for a cell that drew a line last frame.
const STAIR_BONUS: f32 = 12.0;



/// One unique mesh edge with its (1 or 2) faces.
#[derive(Clone, Debug)]
struct Edge {
    a: u32,
    b: u32,
    f0: u32,
    f1: u32,
    /// Always drawn when visible (boundary, crease, or non-manifold); otherwise only as silhouette.
    hard: bool,
    id: u64,
}

struct ObjEdges {
    edges: Vec<Edge>,
    sig: (usize, usize),
}

/// Memory across frames (spelling memory: key -> last glyph, plus any per-mesh edge tables).
#[derive(Default)]
pub struct LineState {
    objs: Vec<ObjEdges>,
    crease_deg: f32,
    memory: HashMap<u64, char>,
    /// Run memory: `(edge id, column or row, orientation)` -> `(row or column, level)` chosen last frame.
    run_mem: RunMem,
    /// Cells that held a line glyph last frame (presence hysteresis).
    had: Vec<bool>,
    prev: Vec<Option<char>>,
    /// Edge ids that survived the culls last frame (hysteresis for LOD and the parallel cull).
    alive: std::collections::HashSet<u64>,
    /// Cells a stub was removed from last frame (hysteresis).
    stub_gone: std::collections::HashSet<usize>,
}

/// When the key misses (chain id or bucket changed at a junction), fall back to last frame's glyph in this cell with
/// this fraction of keep_margin.
const FALLBACK_MARGIN: f32 = 0.75;
/// A kept glyph may have at most this much more far-from-path ink (the ink part of its cost) than the best glyph.
/// Stops `┼` standing in for `─` after a crossing edge moved on; `╱` for a shallow `─` stays allowed.
const KEEP_INK_EXTRA: f32 = 1.0;

#[derive(Clone, Debug)]
pub struct LineParams {
    /// Dihedral angle (degrees) above which an edge is a crease. Default 35.
    pub crease_deg: f32,
    /// Keep last frame's glyph if its cost <= best cost + keep_margin (glyph pixel units).
    pub keep_margin: f32,
    /// A kept glyph may have at most this much more far-from-path ink than the best glyph (default `KEEP_INK_EXTRA`).
    pub keep_ink: f32,
    /// ... and its cost <= keep_tol.
    pub keep_tol: f32,
    pub lineset: LineSet,
    /// Edges whose screen slope (cell units) is at most this are spelled as runs: shallow ones one cell per column,
    /// steep ones one cell per row. 0 turns runs off.
    pub run_slope: f32,
    /// Scan-line glyphs of shallow runs.
    pub runset: RunSet,
    /// Hysteresis of a run level, in levels (keep last frame's level unless the line is more than this far from it).
    pub run_keep: f32,
    /// Also spell the slopes in between (0.5 to 2 in cell units) as runs: one cell per column or row, glyph by cost.
    pub diag_runs: bool,
    /// Remove a 1-2 cell piece that touches a longer line at one end only when its edge chain is shorter than this
    /// many cells on screen (0 = off).
    pub stub_chain: f32,
    /// Mask corner and tee glyphs in cells with one straight direction.
    pub straight_mask: bool,
    /// Draw only one of two nearly parallel edges that run closer than `PAR_DIST` cells.
    pub parallel_cull: bool,
    /// Skip edges whose chain is shorter than `LOD_ON` cells on screen.
    pub lod: bool,
}

/// Glyphs a shallow run uses, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum RunSet {
    /// `⎺ ⎻ ─ ⎼ ⎽` (5 heights; Menlo lacks them, the atlas draws them analytically)
    Scan,
    /// `‾ ─ _` (3 heights, in every font)
    Plain,
}

impl RunSet {
    fn chars(self) -> &'static [char] {
        match self {
            RunSet::Scan => &['⎺', '⎻', '─', '⎼', '⎽'],
            RunSet::Plain => &['‾', '─', '_'],
        }
    }
}

type RunMem = HashMap<(u64, i32, u8), (i32, u8)>;

/// A run claims a cell: the one cell of a shallow run's column (or of a steep run's row).
#[derive(Clone, Copy)]
struct Claim {
    cell: usize,
    obj: u16,
    id: u64,
    va: u32,
    vb: u32,
    glyph: char,
    key: u64,
    /// 1 shallow, 2 steep, 3 diagonal per column, 4 diagonal per row.
    orient: u8,
}

/// A visible stretch of one edge, in cell units. `elig`: 1 shallow, 2 steep, 3 diagonal one cell per column,
/// 4 diagonal one cell per row, 0 not a run.
#[derive(Clone, Copy)]
struct Piece {
    id: u64,
    obj: u16,
    va: u32,
    vb: u32,
    a: Vec2,
    b: Vec2,
    elig: u8,
}

/// One visible straight stretch of an edge, in cell units (any slope), for the culls.
#[derive(Clone, Copy)]
struct Span {
    id: u64,
    obj: u16,
    va: u32,
    vb: u32,
    a: Vec2,
    b: Vec2,
    /// Silhouette edge (soft edge drawn only because it turns away), not a crease.
    sil: bool,
}

/// Edges shorter than this on screen (chain length, cells) are not drawn when new ...
const LOD_ON: f32 = 2.5;
/// ... and a drawn one vanishes only below this.
const LOD_OFF: f32 = 1.8;
/// Two edges are chained for the LOD test when their directions differ by less than this (cos).
const CHAIN_COS: f32 = 0.82;
/// Parallel cull: angle limit (sine), distance limit (cells), and the least overlap as a fraction of the shorter edge.
const PAR_SIN: f32 = 0.17;
const PAR_DIST: f32 = 1.6;
/// Shallow run next to a slanted glyph: the run's target height moves this fraction of the way (at most JOIN_MAX rows)
/// toward the glyph's stroke end when that end is within JOIN_NEAR rows of the line at the column border.
const JOIN_PULL: f32 = 0.5;
const JOIN_MAX: f32 = 0.4;
const JOIN_NEAR: f32 = 0.6;
const PAR_OVERLAP: f32 = 0.35;
/// Passes of majority smoothing of the cull along a chain.
const CHAIN_SMOOTH: usize = 3;
/// Steep lines (unit |dy| above STEEP_DY): strands of one rib stagger along the axis, so overlap is measured against a's span
/// widened by PAR_SLIP of its length each end (only when more than PAR_SLIP_MIN_DIST apart: closer is the same line continuing),
/// and a wide rib near the camera gets PAR_STEEP extra distance.
const STEEP_DY: f32 = 0.6;
const PAR_SLIP: f32 = 1.0;
const PAR_SLIP_MIN_DIST: f32 = 0.7;
const PAR_STEEP: f32 = 1.4;
const PAR_MIN_OVERLAP: f32 = 0.5;
/// Score factors: silhouette over crease, and last frame's survivor over a challenger.
const SIL_BONUS: f32 = 1.5;
const KEEP_BONUS: f32 = 1.4;

struct EdgeVis {
    s: Span,
    len: f32,
}

/// Edge ids to drop this frame: short chains (LOD), then the weaker of two near-parallel edges of one object.
fn cull_edges(spans: &[Span], lod: bool, parallel: bool, alive: &std::collections::HashSet<u64>) -> (std::collections::HashSet<u64>, HashMap<u64, f32>) {
    let mut dead = std::collections::HashSet::new();
    // One entry per edge: its longest piece, total visible length.
    let mut by_id: HashMap<u64, EdgeVis> = HashMap::new();
    for &s in spans {
        let l = (s.b - s.a).length();
        let e = by_id.entry(s.id).or_insert(EdgeVis { s, len: 0.0 });
        if l > (e.s.b - e.s.a).length() {
            e.s = s;
        }
        e.len += l;
    }
    let mut edges: Vec<EdgeVis> = by_id.into_values().collect();
    edges.sort_by_key(|e| e.s.id);
    // Chains: edges of one object sharing a vertex and pointing the same way (a smooth silhouette is many short ones).
    let n = edges.len();
    let mut root: Vec<usize> = (0..n).collect();
    fn find(r: &mut [usize], mut i: usize) -> usize {
        while r[i] != i {
            r[i] = r[r[i]];
            i = r[i];
        }
        i
    }
    let mut at: HashMap<(u16, u32), Vec<usize>> = HashMap::new();
    for (i, e) in edges.iter().enumerate() {
        at.entry((e.s.obj, e.s.va)).or_default().push(i);
        at.entry((e.s.obj, e.s.vb)).or_default().push(i);
    }
    for v in at.values() {
        for (x, &i) in v.iter().enumerate() {
            for &j in &v[x + 1..] {
                let (di, dj) = ((edges[i].s.b - edges[i].s.a).normalize_or_zero(), (edges[j].s.b - edges[j].s.a).normalize_or_zero());
                if di.dot(dj).abs() >= CHAIN_COS {
                    let (ri, rj) = (find(&mut root, i), find(&mut root, j));
                    root[ri] = rj;
                }
            }
        }
    }
    let mut total = vec![0.0f32; n];
    for (i, e) in edges.iter().enumerate() {
        let r = find(&mut root, i);
        total[r] += e.len;
    }
    let chain_len: Vec<f32> = (0..n).map(|i| total[find(&mut root, i)]).collect();
    let chain_of: HashMap<u64, f32> = edges.iter().zip(&chain_len).map(|(e, &l)| (e.s.id, l)).collect();
    let mut keep = vec![true; n];
    if lod {
        for (i, e) in edges.iter().enumerate() {
            let thr = if alive.contains(&e.s.id) { LOD_OFF } else { LOD_ON };
            if chain_len[i] < thr {
                keep[i] = false;
                dead.insert(e.s.id);
            }
        }
    }
    if parallel {
        // Whole chains: a chain goes when most of its length runs alongside a stronger chain (per-edge culling left gaps).
        let roots: Vec<usize> = (0..n).map(|i| find(&mut root, i)).collect();
        let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in (0..n).filter(|&i| keep[i]) {
            members.entry(roots[i]).or_default().push(i);
        }
        let mut chains: Vec<(f32, usize)> = members
            .iter()
            .map(|(&r, m)| {
                let sil = m.iter().any(|&i| edges[i].s.sil);
                let was = m.iter().any(|&i| alive.contains(&edges[i].s.id));
                (chain_len[m[0]] * if sil { SIL_BONUS } else { 1.0 } * if was { KEEP_BONUS } else { 1.0 }, r)
            })
            .collect();
        chains.sort_by(|x, y| y.0.total_cmp(&x.0).then(edges[members[&x.1][0]].s.id.cmp(&edges[members[&y.1][0]].s.id)));
        let mut rank = vec![usize::MAX; n];
        for (x, c) in chains.iter().enumerate() {
            for &i in &members[&c.1] {
                rank[i] = x;
            }
        }
        // Raw decision per edge: lies alongside an edge of a stronger chain.
        let live: Vec<usize> = (0..n).filter(|&i| keep[i]).collect();
        let mut raw = vec![false; n];
        for &w in &live {
            raw[w] = live.iter().any(|&s| rank[s] < rank[w] && alongside(&edges[s].s, &edges[w].s));
        }
        // Smooth along the chain (majority of an edge and its chain neighbours) so a strand goes whole, not in dashes.
        let mut nbr: Vec<Vec<usize>> = vec![Vec::new(); n];
        for v in at.values() {
            for (x, &i) in v.iter().enumerate() {
                for &j in &v[x + 1..] {
                    if keep[i] && keep[j] && roots[i] == roots[j] {
                        nbr[i].push(j);
                        nbr[j].push(i);
                    }
                }
            }
        }
        for _ in 0..CHAIN_SMOOTH {
            let prev = raw.clone();
            for &i in &live {
                let d = prev[i] as u32 + nbr[i].iter().filter(|&&j| prev[j]).count() as u32;
                raw[i] = 2 * d >= 1 + nbr[i].len() as u32 + (nbr[i].len() as u32 & 1 == 0) as u32;
            }
        }
        for &i in &live {
            if raw[i] {
                dead.insert(edges[i].s.id);
            }
        }
    }
    (dead, chain_of)
}

/// True if `b` lies alongside `a`: same object, no shared vertex, nearly parallel, close, and overlapping.
fn alongside(a: &Span, b: &Span) -> bool {
    if a.obj != b.obj || a.va == b.va || a.va == b.vb || a.vb == b.va || a.vb == b.vb {
        return false;
    }
    let (da, db) = (a.b - a.a, b.b - b.a);
    let (la, lb) = (da.length(), db.length());
    if la < 1e-3 || lb < 1e-3 {
        return false;
    }
    let (ua, ub) = (da / la, db / lb);
    if ua.perp_dot(ub).abs() > PAR_SIN {
        return false;
    }
    // Along a's axis: a is [0, la]; b's ends.
    let (t0, t1) = ((b.a - a.a).dot(ua), (b.b - a.a).dot(ua));
    let steep = ua.y.abs() > STEEP_DY;
    let slip = if steep && ua.perp_dot(0.5 * (b.a + b.b) - a.a).abs() > PAR_SLIP_MIN_DIST { PAR_SLIP } else { 0.0 };
    let (lo, hi) = (t0.min(t1).max(-slip * la), t0.max(t1).min((1.0 + slip) * la));
    let ov = hi - lo;
    if ov < PAR_MIN_OVERLAP.min(la.min(lb)) || ov < PAR_OVERLAP * la.min(lb) {
        return false;
    }
    // Point of b at the middle of the overlap.
    let tm = 0.5 * (lo + hi);
    let u = if (t1 - t0).abs() < 1e-6 { 0.5 } else { (tm - t0) / (t1 - t0) };
    let p = b.a + (b.b - b.a) * u;
    ua.perp_dot(p - a.a).abs() < PAR_DIST + if steep { PAR_STEEP } else { 0.0 }
}

const STUB_CHAIN: f32 = 3.0;
/// A stub removed last frame stays removed up to this factor of the chain length threshold.
const STUB_HYST: f32 = 1.6;

/// A run keeps a cell it held last frame this far (cells) past its end.
const END_KEEP: f32 = 0.3;
/// Hysteresis for a run level: keep last frame's level unless the line is more than this many levels away.
const LEVEL_KEEP: f32 = 0.6;

impl Default for LineParams {
    fn default() -> Self {
        LineParams { crease_deg: 35.0, keep_margin: 2.0, keep_ink: KEEP_INK_EXTRA, keep_tol: 5.0, lineset: LineSet::Box, run_slope: 0.5, runset: RunSet::Scan, run_keep: LEVEL_KEEP, diag_runs: true, stub_chain: STUB_CHAIN, straight_mask: true, parallel_cull: true, lod: true }
    }
}

pub struct LineOutput {
    /// `cols * rows` line glyph per cell, or None.
    pub glyphs: Vec<Option<char>>,
    /// Object index owning each line cell (`NO_OBJ` where no line), for colour.
    pub obj: Vec<u16>,
    /// `(persistent memory key, cell index, glyph)` for the respell metric.
    pub ids: Vec<(u64, usize, char)>,
}

fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ceb9fe1a85ec53);
    x ^ (x >> 33)
}

fn edge_id(obj: usize, a: u32, b: u32) -> u64 {
    let (lo, hi) = (a.min(b) as u64, a.max(b) as u64);
    mix(mix(obj as u64 + 1) ^ (lo << 32 | hi))
}

fn build_edges(mesh: &crate::scene::Mesh, obj: usize, crease_deg: f32) -> Vec<Edge> {
    let mut map: HashMap<(u32, u32), (u32, u32, u32)> = HashMap::new();
    for (t, tri) in mesh.tris.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            let e = map.entry((a.min(b), a.max(b))).or_insert((t as u32, NO_FACE, 0));
            if e.0 != t as u32 {
                if e.1 == NO_FACE {
                    e.1 = t as u32;
                }
                e.2 += 1;
            }
        }
    }
    let cos_crease = crease_deg.to_radians().cos();
    let normals: Vec<Vec3> = (0..mesh.tris.len()).map(|t| mesh.face_normal(t).normalize_or_zero()).collect();
    let mut v: Vec<Edge> = map
        .into_iter()
        .map(|((a, b), (f0, f1, extra))| {
            let hard = f1 == NO_FACE || extra > 1 || normals[f0 as usize].dot(normals[f1 as usize]) < cos_crease;
            Edge { a, b, f0, f1, hard, id: edge_id(obj, a, b) }
        })
        .collect();
    v.sort_by_key(|e| e.id);
    v
}

/// A piece of one edge inside one cell.
#[derive(Clone, Copy)]
struct Entry {
    cell: u32,
    obj: u16,
    id: u64,
    /// Mesh vertices of the edge (entries of one object sharing a vertex are one chain).
    va: u32,
    vb: u32,
    /// Segment in glyph pixel coordinates.
    p0: Vec2,
    p1: Vec2,
    t_mid: f32,
    len_cells: f32,
    elig: u8,
}

impl Entry {
    fn len_px(&self) -> f32 {
        (self.p1 - self.p0).length()
    }
}

/// Liang-Barsky clip of `a -> b` against the unit square at `(ox, oy)`; returns the parameter range.
fn clip_cell(a: Vec2, b: Vec2, ox: f32, oy: f32) -> Option<(f32, f32)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-d.x, a.x - ox), (d.x, ox + 1.0 - a.x), (-d.y, a.y - oy), (d.y, oy + 1.0 - a.y)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, t1))
}

struct Vis<'a> {
    gb: &'a GBuffer,
}

impl Vis<'_> {
    /// Is a point at screen pixel `(px, py)` with linear depth `d` on top, for an edge of `obj`?
    fn visible(&self, px: f32, py: f32, d: f32, obj: u16) -> bool {
        let gb = self.gb;
        if px < 0.0 || py < 0.0 || px >= gb.w as f32 || py >= gb.h as f32 {
            return false;
        }
        let (ix, iy) = (px as usize, py as usize);
        let (x0, x1) = (ix.saturating_sub(1), (ix + 1).min(gb.w - 1));
        let (y0, y1) = (iy.saturating_sub(1), (iy + 1).min(gb.h - 1));
        // A pixel in front of the edge blocks it. Pixels of the edge's own object may be nearer by `SLOPE`
        // (grazing surfaces and silhouettes, where depth changes fast across a pixel).
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = y * gb.w + x;
                let pd = gb.depth[i];
                let rel = if gb.obj[i] == obj { SLOPE } else { BIAS_REL };
                if pd * (1.0 + rel) + BIAS_ABS < d {
                    return false;
                }
            }
        }
        true
    }
}

/// Entries of one edge (visible pieces clipped to cells).
#[allow(clippy::too_many_arguments)]
fn trace_edge(scene: &Scene, gb: &GBuffer, vis: &Vis, slope: f32, diag: bool, obj: usize, e: &Edge, sil: bool, out: &mut Vec<Entry>, pieces: &mut Vec<Piece>, spans: &mut Vec<Span>) {
    let o = &scene.objects[obj];
    let m = &o.mesh;
    let wa = o.model.transform_point3(m.verts[e.a as usize]);
    let wb = o.model.transform_point3(m.verts[e.b as usize]);
    let (ca, cb) = (gb.view_proj * wa.extend(1.0), gb.view_proj * wb.extend(1.0));
    let near = Camera::NEAR;
    if ca.w < near && cb.w < near {
        return;
    }
    // Near-plane clip in clip space (w is linear depth). u is the original edge parameter.
    let (mut ua, mut ub) = (0.0f32, 1.0f32);
    if ca.w < near {
        ua = (near - ca.w) / (cb.w - ca.w);
    } else if cb.w < near {
        ub = (near - ca.w) / (cb.w - ca.w);
    }
    let lerp4 = |u: f32| ca + (cb - ca) * u;
    let (qa, qb) = (lerp4(ua), lerp4(ub));
    let (w, h) = (gb.w as f32, gb.h as f32);
    let scr = |q: glam::Vec4| Vec2::new((q.x / q.w * 0.5 + 0.5) * w, (0.5 - q.y / q.w * 0.5) * h);
    let (sa, sb) = (scr(qa), scr(qb));
    let cell_scale = Vec2::new(1.0 / gb.sx as f32, 1.0 / gb.sy as f32);
    let len_cells = ((sb - sa) * cell_scale).length().min(1.0e4);
    let dc = ((sb - sa) * cell_scale).abs();
    let elig: u8 = if slope <= 0.0 {
        0
    } else if dc.y <= slope * dc.x {
        1
    } else if dc.x <= slope * dc.y {
        2
    } else if !diag {
        0
    } else if dc.y <= dc.x {
        3
    } else {
        4
    };
    let mut pstart: Option<Vec2> = None;
    let mut plast = Vec2::ZERO;
    let close = |pstart: &mut Option<Vec2>, plast: Vec2, pieces: &mut Vec<Piece>, spans: &mut Vec<Span>| {
        if let Some(a) = pstart.take()
            && (plast - a).length() > 0.05
        {
            spans.push(Span { id: e.id, obj: obj as u16, va: e.a, vb: e.b, a, b: plast, sil });
            if elig != 0 {
                pieces.push(Piece { id: e.id, obj: obj as u16, va: e.a, vb: e.b, a, b: plast, elig });
            }
        }
    };
    // Clip the screen segment to the buffer (screen-linear parameter s).
    let Some((s0, s1)) = clip_rect(sa, sb, w, h) else { return };
    let n = (((s1 - s0) * len_cells / STEP).ceil() as usize).clamp(1, 20000);
    let (iwa, iwb) = (1.0 / qa.w, 1.0 / qb.w);
    let o16 = obj as u16;
    let (cols, rows) = (gb.cols as i32, gb.rows as i32);
    let g = Vec2::new(GW as f32, GH as f32);
    // Previous visible sample: (position in cell units, edge parameter).
    let mut prev: Option<(Vec2, f32)> = None;
    // Entries of this edge so far plus the (first, last) edge parameter of each.
    let mut mine: Vec<(Entry, f32, f32)> = vec![];
    for k in 0..=n {
        let s = s0 + (s1 - s0) * k as f32 / n as f32;
        let u = s * qa.w / ((1.0 - s) * qb.w + s * qa.w);
        let d = 1.0 / (iwa + (iwb - iwa) * s);
        let sp = sa + (sb - sa) * s;
        let t = ua + (ub - ua) * u;
        let cur = sp * cell_scale;
        if !vis.visible(sp.x, sp.y, d, o16) {
            prev = None;
            close(&mut pstart, plast, pieces, spans);
            continue;
        }
        if pstart.is_none() {
            pstart = Some(cur);
        }
        plast = cur;
        if let Some((pp, pt)) = prev {
            let (lo, hi) = (pp.min(cur), pp.max(cur));
            for cy in (lo.y.floor() as i32)..=(hi.y.floor() as i32) {
                for cx in (lo.x.floor() as i32)..=(hi.x.floor() as i32) {
                    if cx < 0 || cy < 0 || cx >= cols || cy >= rows {
                        continue;
                    }
                    let Some((c0, c1)) = clip_cell(pp, cur, cx as f32, cy as f32) else { continue };
                    if c1 - c0 < 1e-4 {
                        continue;
                    }
                    let origin = Vec2::new(cx as f32, cy as f32);
                    let q0 = (pp + (cur - pp) * c0 - origin) * g;
                    let q1 = (pp + (cur - pp) * c1 - origin) * g;
                    let (t0, t1) = (pt + (t - pt) * c0, pt + (t - pt) * c1);
                    let cell = (cy * cols + cx) as u32;
                    // A step can touch several cells, so the matching entry is not always the last one.
                    match mine.iter_mut().rev().take(4).find(|(en, _, _)| en.cell == cell) {
                        Some((en, _, tl)) => {
                            en.p1 = q1;
                            *tl = t1;
                        }
                        None => mine.push((Entry { cell, obj: o16, id: e.id, va: e.a, vb: e.b, p0: q0, p1: q1, t_mid: 0.0, len_cells, elig }, t0, t1)),
                    }
                }
            }
        }
        prev = Some((cur, t));
    }
    close(&mut pstart, plast, pieces, spans);
    out.extend(mine.into_iter().map(|(mut en, tf, tl)| {
        en.t_mid = 0.5 * (tf + tl);
        en
    }));
}

fn clip_rect(a: Vec2, b: Vec2, w: f32, h: f32) -> Option<(f32, f32)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-d.x, a.x), (d.x, w - 0.01 - a.x), (-d.y, a.y), (d.y, h - 0.01 - a.y)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, t1))
}

fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let d = b - a;
    let l2 = d.length_squared();
    let t = if l2 < 1e-9 { 0.0 } else { ((p - a).dot(d) / l2).clamp(0.0, 1.0) };
    (p - a - d * t).length()
}

/// Preference for plain glyphs. Glyphs that only differ by where inside the cell the ink sits (`▏ ▕ ▁ ▔`) and
/// crowded ones (`╳`) get a large prior: a line near a cell border is then drawn as a plain `│` or `─` in the
/// cell (quantised), instead of hopping between cells' edge glyphs as the camera moves.
fn prior(ch: char) -> f32 {
    match ch {
        '─' | '│' | '/' | '\\' | '-' | '|' | '╱' | '╲' | '+' | '┼' => 0.0,
        '╳' => 1.0,
        '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '╭' | '╮' | '╯' | '╰' | 'L' | 'T' | '7' | 'J' => 0.1,
        '▁' | '▔' | '▏' | '▕' => 6.0,
        '_' | '‾' => 0.25,
        '=' | '~' | '(' | ')' | '[' | ']' | '<' | '>' => 1.0,
        _ => 1.5,
    }
}

/// Cost of every glyph for the segments of one cell.
fn glyph_costs(segs: &[(Vec2, Vec2)], glyphs: &[&Glyph], ink_w: &[f32], costs: &mut Vec<f32>, inks: &mut Vec<f32>) {
    // Distance field of the path, clamped.
    let mut dmap = [0.0f32; GW * GH];
    for y in 0..GH {
        for x in 0..GW {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            dmap[y * GW + x] = segs.iter().map(|&(a, b)| seg_dist(p, a, b)).fold(CLAMP, f32::min);
        }
    }
    // Path samples.
    let mut pts: Vec<Vec2> = vec![];
    for &(a, b) in segs {
        let n = (((b - a).length() / 1.5).ceil() as usize).clamp(1, 12);
        for i in 0..=n {
            pts.push(a + (b - a) * (i as f32 / n as f32));
        }
    }
    let pidx: Vec<usize> = pts
        .iter()
        .map(|p| (p.y.clamp(0.0, GH as f32 - 0.01) as usize) * GW + p.x.clamp(0.0, GW as f32 - 0.01) as usize)
        .collect();
    costs.clear();
    inks.clear();
        for (g, &wsum) in glyphs.iter().zip(ink_w) {
        if g.ink.is_empty() || wsum <= 0.0 {
            costs.push(f32::INFINITY);
            inks.push(f32::INFINITY);
            continue;
        }
        let ink: f32 = g.ink.iter().map(|&(x, y, c)| c * (dmap[y as usize * GW + x as usize] - TOL).max(0.0)).sum::<f32>() / wsum;
        let path: f32 = pidx.iter().map(|&i| (g.dist[i].min(CLAMP) - TOL).max(0.0)).sum::<f32>() / pidx.len() as f32;
        inks.push(ink);
        costs.push(ink + path + prior(g.ch));
    }
}

/// Chains of entries in one cell: entries of one object that share a mesh vertex (a smooth mesh's silhouette is a
/// zigzag of short edges). Returns `(total length in px, representative entry = smallest edge id)` per chain,
/// longest first.
fn chains(es: &[&Entry]) -> Vec<(f32, usize)> {
    let n = es.len();
    let root = chain_roots(es);
    let mut out: Vec<(f32, usize)> = vec![];
    let mut roots: Vec<usize> = vec![];
    for i in 0..n {
        let r = root[i];
        match roots.iter().position(|&x| x == r) {
            Some(k) => {
                out[k].0 += es[i].len_px();
                if es[i].id < es[out[k].1].id {
                    out[k].1 = i;
                }
            }
            None => {
                roots.push(r);
                out.push((es[i].len_px(), i));
            }
        }
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0).then(es[a.1].id.cmp(&es[b.1].id)));
    out
}

/// Chain root of every entry (entries of one object that share a mesh vertex are one chain).
fn chain_roots(es: &[&Entry]) -> Vec<usize> {
    let n = es.len();
    let mut root: Vec<usize> = (0..n).collect();
    fn find(r: &mut [usize], mut i: usize) -> usize {
        while r[i] != i {
            r[i] = r[r[i]];
            i = r[i];
        }
        i
    }
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (es[i], es[j]);
            if a.obj == b.obj && (a.va == b.va || a.va == b.vb || a.vb == b.va || a.vb == b.vb) {
                let (ri, rj) = (find(&mut root, i), find(&mut root, j));
                root[ri] = rj;
            }
        }
    }
    (0..n).map(|i| find(&mut root, i)).collect()
}

/// How many chains really pass through the cell: each has points well on both sides of the cell centre along its
/// own direction. Fewer than two means a `┼` would be a lie (a line that only ends at or touches another).
fn through_count(es: &[&Entry]) -> usize {
    let roots = chain_roots(es);
    let mut seen: Vec<usize> = vec![];
    let centre = Vec2::new(GW as f32 / 2.0, GH as f32 / 2.0);
    let mut count = 0;
    for &r in &roots {
        if seen.contains(&r) {
            continue;
        }
        seen.push(r);
        let mem: Vec<&&Entry> = es.iter().zip(&roots).filter(|(_, rr)| **rr == r).map(|(e, _)| e).collect();
        let Some(long) = mem.iter().max_by(|a, b| a.len_px().total_cmp(&b.len_px())) else { continue };
        let d = (long.p1 - long.p0).normalize_or_zero();
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for e in &mem {
            for p in [e.p0, e.p1] {
                let t = (p - centre).dot(d);
                lo = lo.min(t);
                hi = hi.max(t);
            }
        }
        if lo < -1.5 && hi > 1.5 {
            count += 1;
        }
    }
    count
}

/// Glyphs a diagonal run chooses from (the font decides by cost on the piece in the cell).
pub struct DiagGlyphs<'a> {
    glyphs: Vec<&'a Glyph>,
    ink_w: Vec<f32>,
    /// Keep last frame's glyph if its cost is at most this much above the best.
    margin: f32,
}

impl<'a> DiagGlyphs<'a> {
    pub fn new(atlas: &'a Atlas, margin: f32) -> Self {
        let glyphs: Vec<&Glyph> = ['╱', '╲', '│', '─', '⎺', '⎻', '⎼', '⎽'].iter().filter_map(|&c| atlas.get(c)).collect();
        let ink_w = glyphs.iter().map(|g| g.ink.iter().map(|i| i.2).sum()).collect();
        DiagGlyphs { glyphs, ink_w, margin }
    }
}

/// Glyph index for the stretch `a -> b` (cell units) inside cell `(col, row)`.
fn diag_glyph(a: Vec2, b: Vec2, col: i32, row: i32, dg: &DiagGlyphs, prev: Option<usize>) -> usize {
    let origin = Vec2::new(col as f32, row as f32);
    let g = Vec2::new(GW as f32, GH as f32);
    let (p0, p1) = match clip_cell(a, b, origin.x, origin.y) {
        Some((t0, t1)) if t1 - t0 > 1e-3 => (a + (b - a) * t0, a + (b - a) * t1),
        _ => {
            let d = (b - a).normalize_or_zero() * 0.3;
            let c = origin + Vec2::splat(0.5);
            (c - d, c + d)
        }
    };
    let (q0, q1) = ((p0 - origin) * g, (p1 - origin) * g);
    let shift = (g * 0.5 - (q0 + q1) * 0.5) * PULL;
    let (mut costs, mut inks) = (vec![], vec![]);
    glyph_costs(&[(q0 + shift, q1 + shift)], &dg.glyphs, &dg.ink_w, &mut costs, &mut inks);
    let best = (0..costs.len()).min_by(|&i, &j| costs[i].total_cmp(&costs[j])).unwrap_or(0);
    match prev {
        Some(p) if p < costs.len() && costs[p] <= costs[best] + dg.margin => p,
        _ => best,
    }
}

/// Claims of a diagonal piece: every cell the line crosses at a column centre or at a row centre (for slopes up to 1
/// the second set repeats the first, so a cell is claimed once; a chain of short edges that alternate between the two
/// classes leaves no gap). Glyph by cost.
fn diag_claims(pc: &Piece, mem: &RunMem, cols: usize, rows: usize, dg: &DiagGlyphs, out: &mut Vec<Claim>, new_mem: &mut RunMem) {
    if dg.glyphs.is_empty() {
        return;
    }
    let first = out.len();
    for axis in [3u8, 4u8] {
        diag_axis(pc, axis, mem, cols, rows, dg, out, new_mem, first);
    }
}

#[allow(clippy::too_many_arguments)]
fn diag_axis(pc: &Piece, axis: u8, mem: &RunMem, cols: usize, rows: usize, dg: &DiagGlyphs, out: &mut Vec<Claim>, new_mem: &mut RunMem, first: usize) {
    let per_col = axis == 3;
    let (a, b) = if per_col == (pc.a.x <= pc.b.x) { (pc.a, pc.b) } else { (pc.b, pc.a) };
    let (a, b) = if per_col { (a, b) } else if pc.a.y <= pc.b.y { (pc.a, pc.b) } else { (pc.b, pc.a) };
    // u: position along the run's main axis; v: the cross axis.
    let (ua, ub) = if per_col { (a.x, b.x) } else { (a.y, b.y) };
    let k = if per_col { (b.y - a.y) / (b.x - a.x).max(1e-6) } else { (b.x - a.x) / (b.y - a.y).max(1e-6) };
    let n_main = if per_col { cols } else { rows } as i32;
    let n_cross = if per_col { rows } else { cols } as i32;
    let u0 = (ua - 0.5 - END_KEEP).ceil().max(0.0) as i32;
    for m in u0..n_main {
        let u = m as f32 + 0.5;
        if u >= ub + END_KEEP {
            break;
        }
        let key = (pc.id, m, axis);
        if !(u >= ua && u < ub) && !mem.contains_key(&key) {
            continue;
        }
        let v = if per_col { a.y } else { a.x } + (u - ua) * k;
        let (mut cross, mut prev) = (v.floor() as i32, None);
        if let Some(&(pc_, pg)) = mem.get(&key)
            && (v - (pc_ as f32 + 0.5)).abs() <= 0.5 + 0.1
        {
            cross = pc_;
            prev = Some(pg as usize);
        }
        if cross < 0 || cross >= n_cross {
            continue;
        }
        let (col, row) = if per_col { (m, cross) } else { (cross, m) };
        let gi = diag_glyph(pc.a, pc.b, col, row, dg, prev);
        new_mem.insert(key, (cross, gi as u8));
        let cell = row as usize * cols + col as usize;
        if out[first..].iter().any(|c| c.cell == cell) {
            continue;
        }
        out.push(Claim { cell: row as usize * cols + col as usize, obj: pc.obj, id: pc.id, va: pc.va, vb: pc.vb, glyph: dg.glyphs[gi].ch, key: mix(pc.id ^ mix(m as u64 + 0x3000 * pc.elig as u64)) >> 1, orient: pc.elig });
    }
}

/// Claims of all runs: a shallow piece takes the cell of every column whose centre it covers, a steep piece the cell
/// of every row. Returns the claims and the new run memory.
fn run_claims(
    pieces: &[Piece],
    levels: &[(char, f32)],
    mem: &RunMem,
    keep: f32,
    cols: usize,
    rows: usize,
    dg: &DiagGlyphs,
) -> (Vec<Claim>, RunMem) {
    let mut out = vec![];
    let mut new_mem = HashMap::new();
    // Mean spacing of the levels, in cell heights.
    let sp = if levels.len() > 1 { (levels[levels.len() - 1].1 - levels[0].1) / (levels.len() - 1) as f32 } else { 0.25 };
    for pc in pieces.iter().filter(|p| p.elig >= 3) {
        diag_claims(pc, mem, cols, rows, dg, &mut out, &mut new_mem);
    }
    // Where a shallow run meets a slanted glyph, the run glyph is chosen to meet the slanted glyph's stroke end.
    let slants: HashMap<usize, char> = out.iter().filter(|c| c.glyph == '╱' || c.glyph == '╲').map(|c| (c.cell, c.glyph)).collect();
    for pc in pieces.iter().filter(|p| p.elig < 3) {
        let (a, b) = if pc.elig == 1 { if pc.a.x <= pc.b.x { (pc.a, pc.b) } else { (pc.b, pc.a) } } else if pc.a.y <= pc.b.y { (pc.a, pc.b) } else { (pc.b, pc.a) };
        if pc.elig == 1 {
            let k = (b.y - a.y) / (b.x - a.x).max(1e-6);
            let c0 = (a.x - 0.5 - END_KEEP).ceil().max(0.0) as i32;
            for c in c0..cols as i32 {
                let x = c as f32 + 0.5;
                if x >= b.x + END_KEEP {
                    break;
                }
                let mut y = a.y + (x - a.x) * k;
                // Stroke end of a slanted glyph in the neighbouring column, seen from this column, in rows; shift the target to meet it.
                if JOIN_PULL > 0.0 {
                    for (nc, toward_right) in [(c - 1, true), (c + 1, false)] {
                        if nc < 0 || nc >= cols as i32 {
                            continue;
                        }
                        let yb = y - k * 0.5 * if toward_right { 1.0 } else { -1.0 };
                        for r in (yb.floor() as i32 - 1)..=(yb.floor() as i32 + 1) {
                            if r < 0 || r >= rows as i32 {
                                continue;
                            }
                            let Some(&g) = slants.get(&(r as usize * cols + nc as usize)) else { continue };
                            // '╱' runs bottom-left to top-right, '╲' top-left to bottom-right.
                            let top = (g == '╱') == toward_right;
                            let end = r as f32 + if top { 0.0 } else { 1.0 };
                            if (end - yb).abs() <= JOIN_NEAR {
                                y += (end - yb).clamp(-JOIN_MAX, JOIN_MAX) * JOIN_PULL;
                                break;
                            }
                        }
                    }
                }
                let key = (pc.id, c, 1u8);
                // Presence hysteresis: the ends of a run hold a cell they had last frame a little longer.
                if !(x >= a.x && x < b.x) && !mem.contains_key(&key) {
                    continue;
                }
                let mut pick: Option<(i32, u8)> = None;
                if let Some(&(r, l)) = mem.get(&key)
                    && let Some(&(_, h)) = levels.get(l as usize)
                    && ((r as f32 + h) - y).abs() <= keep * sp
                {
                    pick = Some((r, l));
                }
                let (row, lv) = pick.unwrap_or_else(|| {
                    let r0 = y.floor() as i32;
                    let mut best = (f32::MAX, 0, 0u8);
                    for r in r0 - 1..=r0 + 1 {
                        for (l, &(_, h)) in levels.iter().enumerate() {
                            let d = ((r as f32 + h) - y).abs();
                            if d < best.0 {
                                best = (d, r, l as u8);
                            }
                        }
                    }
                    (best.1, best.2)
                });
                if row < 0 || row >= rows as i32 {
                    continue;
                }
                new_mem.insert(key, (row, lv));
                out.push(Claim { cell: row as usize * cols + c as usize, obj: pc.obj, id: pc.id, va: pc.va, vb: pc.vb, glyph: levels[lv as usize].0, key: mix(pc.id ^ mix(c as u64 + 0x1000)) >> 1, orient: 1 });
            }
        } else {
            let k = (b.x - a.x) / (b.y - a.y).max(1e-6);
            let r0 = (a.y - 0.5 - END_KEEP).ceil().max(0.0) as i32;
            for r in r0..rows as i32 {
                let y = r as f32 + 0.5;
                if y >= b.y + END_KEEP {
                    break;
                }
                let x = a.x + (y - a.y) * k;
                let key = (pc.id, r, 2u8);
                if !(y >= a.y && y < b.y) && !mem.contains_key(&key) {
                    continue;
                }
                let mut col = x.floor() as i32;
                if let Some(&(pc_, _)) = mem.get(&key)
                    && (x - (pc_ as f32 + 0.5)).abs() <= 0.5 + 0.1
                {
                    col = pc_;
                }
                if col < 0 || col >= cols as i32 {
                    continue;
                }
                new_mem.insert(key, (col, 0));
                out.push(Claim { cell: r as usize * cols + col as usize, obj: pc.obj, id: pc.id, va: pc.va, vb: pc.vb, glyph: '│', key: mix(pc.id ^ mix(r as u64 + 0x2000)) >> 1, orient: 2 });
            }
        }
    }
    (out, new_mem)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// One run owns the cell: its glyph.
    Run,
    /// Two or more chains meet (or a run is crossed): per-cell scoring over every entry.
    Junction,
    /// Only a stub of a run edge: nothing to draw.
    Drop,
    /// Ordinary per-cell path.
    Cell,
}

/// Direction class of an entry (that of its edge, so it matches the run claims): 1 shallow, 2 steep, 3 and 4 diagonal,
/// 0 not a run edge.
fn entry_orient(e: &Entry) -> u8 {
    e.elig
}

/// Decide how a cell with entries `es` and run claims `cl` is spelled.
fn cell_kind(es: &[&Entry], cl: &[&Claim], was: bool) -> Kind {
    let min_len = if was { MIN_KEEP_LEN } else { MIN_NEW_LEN };
    match cl.len() {
        0 => {
            if es.iter().all(|e| e.elig != 0) {
                let strong = chains(es).iter().filter(|c| c.0 >= min_len).count();
                if strong >= 2 { Kind::Cell } else { Kind::Drop }
            } else {
                Kind::Cell
            }
        }
        1 => {
            let c = cl[0];
            let others: Vec<&Entry> = es
                .iter()
                .copied()
                .filter(|e| !(e.obj == c.obj && (e.id == c.id || e.va == c.va || e.va == c.vb || e.vb == c.va || e.vb == c.vb)))
                .collect();
            // Lines running the same way as the run merge into it; only a line crossing it makes a junction.
            let crossing: Vec<&Entry> = others.into_iter().filter(|e| entry_orient(e) != c.orient).collect();
            if chains(&crossing).iter().any(|ch| ch.0 >= min_len) { Kind::Junction } else { Kind::Run }
        }
        _ if cl.iter().all(|c| c.orient == cl[0].orient) => {
            let crossing: Vec<&Entry> = es.iter().copied().filter(|e| entry_orient(e) != cl[0].orient).collect();
            if chains(&crossing).iter().any(|ch| ch.0 >= min_len) { Kind::Junction } else { Kind::Run }
        }
        _ => Kind::Junction,
    }
}

struct Decision {
    cell: usize,
    key: u64,
    glyph: char,
    obj: u16,
    /// Edge the cell was spelled from (for its chain length).
    edge: u64,
    /// Longest entry of the cell (glyph px) and how much it counts in a duel with a neighbour.
    seg: (Vec2, Vec2),
    score: f32,
    /// Shorter than the presence threshold: kept only as a bridge between two strong neighbours.
    weak: bool,
}

/// Does the line leave `a` through the side facing `b` (`b` is the right or lower neighbour) and enter `b` at the
/// same place? Steep lines are tested against the right neighbour, shallow ones against the lower neighbour.
fn continues(a: &Decision, b: &Decision, horizontal: bool, slack: f32) -> bool {
    let steep = |s: (Vec2, Vec2)| {
        let d = (s.1 - s.0).abs();
        if horizontal { d.y >= 1.5 * d.x } else { d.x >= 2.0 * d.y }
    };
    if !steep(a.seg) || !steep(b.seg) {
        return false;
    }
    let (ax, bx) = (a.seg.0, a.seg.1);
    let (pa, pb) = if horizontal {
        (if ax.x > bx.x { ax } else { bx }, if b.seg.0.x < b.seg.1.x { b.seg.0 } else { b.seg.1 })
    } else {
        (if ax.y > bx.y { ax } else { bx }, if b.seg.0.y < b.seg.1.y { b.seg.0 } else { b.seg.1 })
    };
    if horizontal {
        pa.x > GW as f32 - 1.5 - slack && pb.x < 1.5 + slack && (pa.y - pb.y).abs() < 2.5 + slack
    } else {
        pa.y > GH as f32 - 1.5 - slack && pb.y < 1.5 + slack && (pa.x - pb.x).abs() < 2.5 + slack
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_lines(
    scene: &Scene,
    _cam: &Camera,
    gb: &GBuffer,
    atlas: &Atlas,
    params: &LineParams,
    state: &mut LineState,
    cols: usize,
    rows: usize,
) -> LineOutput {
    let mut out = LineOutput { glyphs: vec![None; cols * rows], obj: vec![NO_OBJ; cols * rows], ids: vec![] };

    // Per-mesh edge tables (meshes are static).
    let sig: Vec<(usize, usize)> = scene.objects.iter().map(|o| (o.mesh.verts.len(), o.mesh.tris.len())).collect();
    if state.crease_deg != params.crease_deg
        || state.objs.len() != sig.len()
        || state.objs.iter().zip(&sig).any(|(a, &b)| a.sig != b)
    {
        state.objs = scene
            .objects
            .iter()
            .enumerate()
            .map(|(i, o)| ObjEdges { edges: build_edges(&o.mesh, i, params.crease_deg), sig: sig[i] })
            .collect();
        state.crease_deg = params.crease_deg;
    }

    // Candidate edges for this frame: (object, edge index).
    let mut cand: Vec<(usize, usize, bool)> = vec![];
    for (oi, o) in scene.objects.iter().enumerate() {
        let cam_obj = o.model.inverse().transform_point3(gb.cam_pos);
        let m = &o.mesh;
        let front: Vec<bool> = (0..m.tris.len())
            .map(|t| m.face_normal(t).dot(cam_obj - m.verts[m.tris[t][0] as usize]) > 0.0)
            .collect();
        for (ei, e) in state.objs[oi].edges.iter().enumerate() {
            let f0 = front[e.f0 as usize];
            let keep = if e.f1 == NO_FACE {
                true
            } else {
                let f1 = front[e.f1 as usize];
                if e.hard { f0 || f1 } else { f0 != f1 }
            };
            if keep {
                cand.push((oi, ei, e.f1 != NO_FACE && front[e.f0 as usize] != front[e.f1 as usize]));
            }
        }
    }

    let vis = Vis { gb };
    let objs = &state.objs;
    let traced: Vec<(Vec<Entry>, Vec<Piece>, Vec<Span>)> = cand
        .par_chunks(64)
        .map(|chunk| {
            let (mut v, mut p, mut sp) = (vec![], vec![], vec![]);
            for &(oi, ei, sil) in chunk {
                trace_edge(scene, gb, &vis, params.run_slope, params.diag_runs, oi, &objs[oi].edges[ei], sil, &mut v, &mut p, &mut sp);
            }
            (v, p, sp)
        })
        .collect();
    let mut entries: Vec<Entry> = vec![];
    let mut pieces: Vec<Piece> = vec![];
    let mut spans: Vec<Span> = vec![];
    for (e, p, sp) in traced {
        entries.extend(e);
        pieces.extend(p);
        spans.extend(sp);
    }
    let mut chain_of: HashMap<u64, f32> = HashMap::new();
    if params.lod || params.parallel_cull {
        spans.sort_by(|a, b| a.id.cmp(&b.id).then(a.a.x.total_cmp(&b.a.x)).then(a.a.y.total_cmp(&b.a.y)));
        let (dead, chain_len) = cull_edges(&spans, params.lod, params.parallel_cull, &state.alive);
        chain_of = chain_len;
        entries.retain(|e| !dead.contains(&e.id));
        pieces.retain(|p| !dead.contains(&p.id));
        state.alive = spans.iter().map(|s| s.id).filter(|id| !dead.contains(id)).collect();
    }
    let min_entry = MIN_ENTRY_LEN;
    entries.retain(|e| e.len_px() >= min_entry);
    entries.sort_unstable_by_key(|e| (e.cell, e.id));

    // Group by cell.
    let mut groups: Vec<(usize, usize)> = vec![];
    let mut i = 0;
    while i < entries.len() {
        let mut j = i;
        while j < entries.len() && entries[j].cell == entries[i].cell {
            j += 1;
        }
        groups.push((i, j));
        i = j;
    }

    // Run levels: the scan-line glyphs with the height of their ink (fraction of the cell), top to bottom.
    let mut levels: Vec<(char, f32)> = params
        .runset
        .chars()
        .iter()
        .filter_map(|&c| {
            let g = atlas.get(c)?;
            let w: f32 = g.ink.iter().map(|i| i.2).sum();
            (w > 0.0).then(|| (c, g.ink.iter().map(|&(_, y, c)| c * (y as f32 + 0.5)).sum::<f32>() / w / GH as f32))
        })
        .collect();
    levels.sort_by(|a, b| a.1.total_cmp(&b.1));
    let (claims, new_run_mem) = if params.run_slope > 0.0 && !levels.is_empty() { run_claims(&pieces, &levels, &state.run_mem, params.run_keep, cols, rows, &DiagGlyphs::new(atlas, params.keep_margin)) } else { (vec![], HashMap::new()) };
    let mut claim_at: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, c) in claims.iter().enumerate() {
        claim_at.entry(c.cell).or_default().push(i);
    }
    let kinds: Vec<Kind> = groups
        .par_iter()
        .map(|&(i, j)| {
            let cell = entries[i].cell as usize;
            let es: Vec<&Entry> = entries[i..j].iter().collect();
            let cl: Vec<&Claim> = claim_at.get(&cell).map(|v| v.iter().map(|&k| &claims[k]).collect()).unwrap_or_default();
            cell_kind(&es, &cl, state.had.get(cell).copied().unwrap_or(false))
        })
        .collect();
    let cross_idx: Vec<usize> = {
        let g: Vec<&Glyph> = params.lineset.chars().into_iter().filter_map(|c| atlas.get(c)).collect();
        g.iter().enumerate().filter(|(_, g)| g.ch == '┼' || g.ch == '╳').map(|(i, _)| i).collect()
    };
    let glyphs: Vec<&Glyph> = params.lineset.chars().into_iter().filter_map(|c| atlas.get(c)).collect();
    let ink_w: Vec<f32> = glyphs.iter().map(|g| g.ink.iter().map(|i| i.2).sum()).collect();
    let memory = &state.memory;
    let had: &[bool] = &state.had;
    // Where the font draws a centred line: x of `│`, y of `─` (ink centroids, pixel centres at +0.5).
    let centroid = |ch: char, axis: usize| -> Option<f32> {
        let g = atlas.get(ch)?;
        let w: f32 = g.ink.iter().map(|i| i.2).sum();
        let v: f32 = g.ink.iter().map(|&(x, y, c)| c * ([x, y][axis] as f32 + 0.5)).sum();
        (w > 0.0).then(|| v / w)
    };
    let centre = Vec2::new(centroid('│', 0).unwrap_or(GW as f32 / 2.0), centroid('─', 1).unwrap_or(GH as f32 / 2.0));
    let has_entries: std::collections::HashSet<usize> = groups.iter().map(|&(i, _)| entries[i].cell as usize).collect();
    let decisions: Vec<Decision> = groups
        .par_iter()
        .zip(kinds.par_iter())
        .filter_map(|(&(i, j), &kind)| {
            if kind == Kind::Run || kind == Kind::Drop {
                return None;
            }
            let mut es: Vec<&Entry> = entries[i..j].iter().collect();
            es.sort_by(|a, b| b.len_px().total_cmp(&a.len_px()).then(a.id.cmp(&b.id)));
            let was = had.get(entries[i].cell as usize).copied().unwrap_or(false);
            let ch = chains(&es);
            let (main_len, rep) = (ch[0].0, es[ch[0].1]);
            let weak = main_len < if was { MIN_KEEP_LEN } else { MIN_NEW_LEN };
            if weak && main_len < MIN_ENTRY_LEN {
                return None;
            }
            // Border hysteresis: a new line cell hugging the border next to a cell that already drew this line
            // would give `││` (or a hop between columns), so leave it to the neighbour.
            if !was {
                let cell = entries[i].cell as usize;
                let (x0, x1) = es.iter().fold((f32::MAX, f32::MIN), |(lo, hi), e| (lo.min(e.p0.x.min(e.p1.x)), hi.max(e.p0.x.max(e.p1.x))));
                let holds = |n: usize| had.get(n).copied().unwrap_or(false) && has_entries.contains(&n);
                if (x1 < BORDER_PX && !cell.is_multiple_of(cols) && holds(cell - 1)) || (x0 > GW as f32 - BORDER_PX && cell % cols + 1 < cols && holds(cell + 1)) {
                    return None;
                }
            }
                        let segs: Vec<(Vec2, Vec2)> = es
                .iter()
                .map(|e| {
                    let shift = (centre - (e.p0 + e.p1) * 0.5) * PULL;
                    (e.p0 + shift, e.p1 + shift)
                })
                .collect();
            let mut costs = vec![];
            let mut inks = vec![];
            glyph_costs(&segs, &glyphs, &ink_w, &mut costs, &mut inks);
            // One straight direction in the cell: no corner or tee glyph can be right (a comb `┘┘┘` on a slanted line).
            let d0 = (es[0].p1 - es[0].p0).normalize_or_zero();
            let straight = es.iter().all(|e| {
                let d = (e.p1 - e.p0).normalize_or_zero();
                d.perp_dot(d0).abs() < 0.25
            });
            if straight && params.straight_mask {
                for (gi, g) in glyphs.iter().enumerate() {
                    if "┌┐└┘├┤┬┴┼╳╭╮╯╰".contains(g.ch) {
                        costs[gi] = f32::INFINITY;
                        inks[gi] = f32::INFINITY;
                    }
                }
            }
            if through_count(&es) < 2 {
                for &ci in &cross_idx {
                    costs[ci] = f32::INFINITY;
                    inks[ci] = f32::INFINITY;
                }
            }
            let (bi, &best) = costs.iter().enumerate().min_by(|a, b| a.1.total_cmp(b.1))?;
            if !best.is_finite() {
                return None;
            }
            let bucket = (rep.t_mid * rep.len_cells * 2.0).round() as i64;
            let key_at = |bk: i64| -> u64 {
                let bk = bk as u64;
                if ch.len() >= 2 {
                    let (ia, ib) = (rep.id, es[ch[1].1].id);
                    let (a, b) = (ia.min(ib), ia.max(ib));
                    mix(a ^ mix(b ^ mix(bk))) >> 1
                } else {
                    mix(rep.id ^ mix(bk)) >> 1
                }
            };
            let key = key_at(bucket);
            let mut pick = bi;
            let cell_prev = state.prev.get(entries[i].cell as usize).copied().flatten();
            let hit = memory.get(&key).or_else(|| memory.get(&key_at(bucket - 1))).or_else(|| memory.get(&key_at(bucket + 1)));
            if let Some(&pg) = hit.or(cell_prev.as_ref())
                && let Some(pi) = glyphs.iter().position(|g| g.ch == pg)
            {
                let margin = if hit.is_some() { params.keep_margin } else { params.keep_margin * FALLBACK_MARGIN };
                if costs[pi] <= best + margin && costs[pi] <= params.keep_tol && inks[pi] <= inks[bi] + params.keep_ink {
                    pick = pi;
                }
            }
            Some(Decision { cell: entries[i].cell as usize, key, glyph: glyphs[pick].ch, obj: rep.obj, edge: rep.id, seg: (es[0].p0, es[0].p1), score: main_len + if was { STAIR_BONUS } else { 0.0 }, weak })
        })
        .collect();

    // Staircases: a slanted line that crosses a cell border gives a piece in both neighbours (`││`). Keep the
    // longer one (a cell that drew last frame gets a bonus).
    let mut decisions = decisions;
    // Bridges: a cell with only a short piece (a line clipping a cell corner) still draws if the line goes on in
    // two neighbouring cells, so the outline has no one-cell holes.
    {
        let strong: HashMap<usize, u16> = decisions.iter().filter(|d| !d.weak).map(|d| (d.cell, d.obj)).collect();
        decisions.retain(|d| {
            if !d.weak {
                return true;
            }
            let (cx, cy) = ((d.cell % cols) as i32, (d.cell / cols) as i32);
            let n = (-1..=1)
                .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                .filter(|&(dx, dy)| {
                    let (x, y) = (cx + dx, cy + dy);
                    (dx, dy) != (0, 0) && x >= 0 && y >= 0 && x < cols as i32 && y < rows as i32 && strong.get(&(y as usize * cols + x as usize)) == Some(&d.obj)
                })
                .count();
            n >= 2
        });
    }
    let at: HashMap<usize, usize> = decisions.iter().enumerate().map(|(i, d)| (d.cell, i)).collect();
    let mut dead = vec![false; decisions.len()];
    let mut pairs: Vec<(f32, usize, usize)> = vec![];
    for (i, d) in decisions.iter().enumerate() {
        for (n, horizontal) in [(d.cell + 1, true), (d.cell + cols, false)] {
            if horizontal && n % cols == 0 {
                continue;
            }
            let Some(&j) = at.get(&n) else { continue };
            let e = &decisions[j];
            let (hd, he) = (had.get(d.cell).copied().unwrap_or(false), had.get(e.cell).copied().unwrap_or(false));
            let slack = if hd != he { SLACK } else if hd { TIGHT } else { 0.0 };
            if d.obj == e.obj && continues(d, e, horizontal, slack) {
                pairs.push(if d.score >= e.score { (d.score, i, j) } else { (e.score, j, i) });
            }
        }
    }
    // Greedy: the strongest winners decide first; a cell that already lost does not kill its neighbour.
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    for &(_, w, l) in &pairs {
        if !dead[w] && !dead[l] {
            dead[l] = true;
        }
    }
    let mut k = 0;
    decisions.retain(|_| {
        k += 1;
        !dead[k - 1]
    });

    // Runs: their cells, except where the cell went to a junction.
    {
        let group_kind: HashMap<usize, Kind> = groups.iter().zip(&kinds).map(|(&(i, _), &k)| (entries[i].cell as usize, k)).collect();
        let mut taken: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut order: Vec<usize> = (0..claims.len()).collect();
        order.sort_by_key(|&i| (claims[i].cell, state.prev.get(claims[i].cell).copied().flatten() != Some(claims[i].glyph), claims[i].id));
        for i in order {
            let c = &claims[i];
            if matches!(group_kind.get(&c.cell), Some(Kind::Junction | Kind::Cell)) || !taken.insert(c.cell) {
                continue;
            }
            decisions.push(Decision { cell: c.cell, key: c.key, glyph: c.glyph, obj: c.obj, edge: c.id, seg: (Vec2::ZERO, Vec2::ZERO), score: 0.0, weak: false });
        }
        // Three steep runs side by side read as a comb: the middle one goes.
        let vert: std::collections::HashSet<usize> = decisions.iter().filter(|d| d.glyph == '│' && d.seg.1 == Vec2::ZERO).map(|d| d.cell).collect();
        decisions.retain(|d| !(d.glyph == '│' && d.seg.1 == Vec2::ZERO && d.cell % cols > 0 && d.cell % cols + 1 < cols && vert.contains(&(d.cell - 1)) && vert.contains(&(d.cell + 1))));
    }
    state.run_mem = new_run_mem;
    let mut used: HashMap<u64, u64> = HashMap::new();
    let mut mem: HashMap<u64, char> = HashMap::new();
    // Stubs: a 1-2 cell piece that touches a longer line at one end only and comes from a short visible chain.
    let mut pre_had: Vec<usize> = vec![];
    if params.stub_chain > 0.0 {
        pre_had = decisions.iter().map(|d| d.cell).collect();
        let mut grid = vec![' '; cols * rows];
        for d in &decisions {
            grid[d.cell] = d.glyph;
        }
        let edge_at: HashMap<usize, u64> = decisions.iter().map(|d| (d.cell, d.edge)).collect();
        let mut gone: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for piece in crate::metrics::stub_pieces(&grid, cols, true) {
            let cells: Vec<usize> = piece.iter().map(|&(r, c)| r as usize * cols + c as usize).collect();
            let thr = if cells.iter().any(|c| state.stub_gone.contains(c)) { params.stub_chain * STUB_HYST } else { params.stub_chain };
            if cells.iter().all(|c| edge_at.get(c).and_then(|e| chain_of.get(e)).is_some_and(|&l| l < thr)) {
                gone.extend(cells);
            }
        }
        decisions.retain(|d| !gone.contains(&d.cell));
        state.stub_gone = gone;
    }
    for d in decisions {
        let n = used.entry(d.key).or_insert(0);
        let key = if *n == 0 { d.key } else { mix(d.key ^ *n) >> 1 };
        *n += 1;
        out.glyphs[d.cell] = Some(d.glyph);
        out.obj[d.cell] = d.obj;
        out.ids.push((key, d.cell, d.glyph));
        mem.insert(key, d.glyph);
    }
    state.memory = mem;
    state.prev = out.glyphs.clone();
    state.had = out.glyphs.iter().map(|g| g.is_some()).collect();
    for c in pre_had {
        state.had[c] = true;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::{DEFAULT_FONT, all_chars};
    use crate::raster::rasterize;
    use crate::scene::{Object, cube, torus};
    use glam::Mat4;

    fn cube_scene() -> Scene {
        let mut s = Scene::shapes();
        s.objects = vec![Object { name: "c".into(), mesh: cube(), model: Mat4::IDENTITY, class: 0 }];
        s
    }

    fn run(scene: &Scene, cam: &Camera, st: &mut LineState, p: &LineParams) -> (LineOutput, usize, usize) {
        let (cols, rows) = (60, 30);
        let atlas = Atlas::load(DEFAULT_FONT, &all_chars()).unwrap();
        let gb = rasterize(scene, cam, cols, rows, 4, 8);
        (render_lines(scene, cam, &gb, &atlas, p, st, cols, rows), cols, rows)
    }

    #[test]
    fn cell_fallback_holds_glyphs_when_keys_miss() {
        let scene = cube_scene();
        let cam = Camera::look_at(Vec3::new(0.7, 0.5, 3.0), Vec3::ZERO);
        let mut st = LineState::default();
        let p = LineParams::default();
        let (a, _, _) = run(&scene, &cam, &mut st, &p);
        st.memory.clear();
        let (b, _, _) = run(&scene, &cam, &mut st, &p);
        assert_eq!(a.glyphs, b.glyphs);
    }

    #[test]
    fn crossing_glyph_not_kept_on_plain_line() {
        let atlas = Atlas::load(DEFAULT_FONT, &all_chars()).unwrap();
        let glyphs: Vec<&Glyph> = atlas.glyphs.iter().filter(|g| "─┼╱".contains(g.ch)).collect();
        let ink_w: Vec<f32> = glyphs.iter().map(|g| g.ink.iter().map(|i| i.2).sum()).collect();
        let y = GH as f32 / 2.0;
        let segs = [(Vec2::new(0.0, y), Vec2::new(GW as f32, y))];
        let (mut costs, mut inks) = (vec![], vec![]);
        glyph_costs(&segs, &glyphs, &ink_w, &mut costs, &mut inks);
        let at = |c: char| glyphs.iter().position(|g| g.ch == c).unwrap();
        assert!(inks[at('┼')] > inks[at('─')] + KEEP_INK_EXTRA, "{inks:?}");
    }

    #[test]
    fn cube_edges_straight() {
        let scene = cube_scene();
        let cam = Camera::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO);
        let (o, cols, rows) = run(&scene, &cam, &mut LineState::default(), &LineParams::default());
        let at = |x: usize, y: usize| o.glyphs[y * cols + x];
        let mut hz = vec![];
        let mut vt = vec![];
        for y in 0..rows {
            let row: String = (0..cols).map(|x| at(x, y).unwrap_or(' ')).collect();
            let n = row.chars().filter(|&c| "⎺⎻─⎼⎽".contains(c)).count();
            if n >= 8 {
                hz.push(y);
            }
        }
        for x in 0..cols {
            let n = (0..rows).filter(|&y| at(x, y) == Some('│')).count();
            if n >= 8 {
                vt.push(x);
            }
        }
        assert!(hz.len() >= 2, "horizontal runs of ─ at rows {hz:?}");
        assert!(vt.len() >= 2, "vertical runs of │ at cols {vt:?}");
    }

    #[test]
    fn torus_outline_closed() {
        let mut scene = Scene::shapes();
        scene.objects = vec![Object { name: "t".into(), mesh: torus(0.7, 0.28, 32, 20), model: Mat4::IDENTITY, class: 0 }];
        let cam = Camera::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO);
        let (o, cols, rows) = run(&scene, &cam, &mut LineState::default(), &LineParams::default());
        let xs: Vec<usize> = (0..cols).filter(|&x| (0..rows).any(|y| o.glyphs[y * cols + x].is_some())).collect();
        let (lo, hi) = (*xs.first().unwrap(), *xs.last().unwrap());
        assert!(hi - lo >= 20, "torus too small: {lo}..{hi}");
        let top = |x: usize| (0..=rows / 2).any(|y| o.glyphs[y * cols + x].is_some());
        // A shallow diagonal may skip one column; two empty columns in a row are a broken outline.
        for x in lo..=hi - 1 {
            assert!(top(x) || top(x + 1), "gap in the top half at columns {x} and {}", x + 1);
        }
    }

    #[test]
    fn torus_grazing_outline_closed() {
        // The shapes-scene torus seen from a low angle: its far outline must be one unbroken run of cells.
        let mut scene = Scene::shapes();
        let cam = scene.bench_camera(120);
        scene.objects = vec![scene.objects[3].clone()];
        let (cols, rows) = (110, 34);
        let atlas = Atlas::load(DEFAULT_FONT, &all_chars()).unwrap();
        let gb = rasterize(&scene, &cam, cols, rows, 4, 8);
        let o = render_lines(&scene, &cam, &gb, &atlas, &LineParams::default(), &mut LineState::default(), cols, rows);
        let on = |x: usize, y: usize| o.glyphs[y * cols + x].is_some();
        let ys: Vec<usize> = (0..rows).filter(|&y| (0..cols).any(|x| on(x, y))).collect();
        let (y0, y1) = (*ys.first().unwrap(), *ys.last().unwrap());
        let xs: Vec<usize> = (0..cols).filter(|&x| (0..rows).any(|y| on(x, y))).collect();
        let (lo, hi) = (*xs.first().unwrap(), *xs.last().unwrap());
        assert!(hi - lo >= 15, "torus too small: {lo}..{hi}");
        let mid = (y0 + y1) / 2;
        let missing: Vec<usize> = (lo..=hi).filter(|&x| !(y0..=mid).any(|y| on(x, y))).collect();
        assert!(missing.is_empty(), "gaps in the far outline at columns {missing:?} (rows {y0}..{mid})");
    }

    fn no_diag() -> DiagGlyphs<'static> {
        DiagGlyphs { glyphs: vec![], ink_w: vec![], margin: 2.0 }
    }

    fn levels5() -> Vec<(char, f32)> {
        vec![('⎺', 0.06), ('⎻', 0.31), ('─', 0.57), ('⎼', 0.75), ('⎽', 0.94)]
    }

    #[test]
    fn shallow_run_one_cell_per_column() {
        // A line dropping about 1.3 rows over 12 columns.
        let p = Piece { id: 7, obj: 0, va: 1, vb: 2, a: Vec2::new(2.0, 5.1), b: Vec2::new(14.0, 6.4), elig: 1 };
        let (claims, _) = run_claims(&[p], &levels5(), &HashMap::new(), 0.6, 40, 20, &no_diag());
        let cols = 40;
        let mut by_col: Vec<Vec<&Claim>> = vec![vec![]; cols];
        for c in &claims {
            by_col[c.cell % cols].push(c);
        }
        for x in 2..14 {
            assert_eq!(by_col[x].len(), 1, "column {x}");
            assert!("⎺⎻─⎼⎽".contains(by_col[x][0].glyph), "scan-line glyph only, got {}", by_col[x][0].glyph);
        }
        assert!(by_col[0].is_empty() && by_col[15].is_empty());
        // Rows never jump by more than one between neighbouring columns: no holes.
        for x in 3..14 {
            let (r0, r1) = ((by_col[x - 1][0].cell / cols) as i32, (by_col[x][0].cell / cols) as i32);
            assert!((r1 - r0).abs() <= 1, "rows {r0} -> {r1} at column {x}");
        }
        assert!(by_col[2][0].cell / cols <= by_col[13][0].cell / cols);
    }

    #[test]
    fn steep_run_one_cell_per_row() {
        let p = Piece { id: 9, obj: 0, va: 1, vb: 2, a: Vec2::new(5.2, 2.0), b: Vec2::new(6.6, 14.0), elig: 2 };
        let (claims, _) = run_claims(&[p], &levels5(), &HashMap::new(), 0.6, 40, 20, &no_diag());
        let mut cols_at: Vec<Option<usize>> = vec![None; 20];
        for c in &claims {
            assert_eq!(c.glyph, '│');
            assert!(cols_at[c.cell / 40].replace(c.cell % 40).is_none(), "two cells in one row");
        }
        for r in 2..14 {
            assert!(cols_at[r].is_some(), "row {r} missing");
        }
    }

    fn diag_glyphs(atlas: &Atlas) -> DiagGlyphs<'_> {
        DiagGlyphs::new(atlas, 2.0)
    }

    #[test]
    fn diagonal_run_one_cell_per_row_no_gap() {
        // About 60 degrees on screen in cell units: 1 column per 1.7 rows is too steep; use 1 per 1.0 (63 deg with 1:2 cells).
        let atlas = Atlas::load(DEFAULT_FONT, &all_chars()).unwrap();
        let dg = diag_glyphs(&atlas);
        for (a, b) in [(Vec2::new(5.2, 3.1), Vec2::new(15.0, 12.9)), (Vec2::new(5.2, 3.0), Vec2::new(12.0, 13.0)), (Vec2::new(15.0, 3.0), Vec2::new(8.0, 11.0))] {
            let p = Piece { id: 3, obj: 0, va: 1, vb: 2, a, b, elig: 4 };
            let (claims, _) = run_claims(&[p], &levels5(), &HashMap::new(), 0.6, 40, 20, &dg);
            let mut at: Vec<Vec<usize>> = vec![vec![]; 20];
            for c in &claims {
                at[c.cell / 40].push(c.cell % 40);
            }
            let (r0, r1) = (a.y.min(b.y).ceil() as usize, b.y.max(a.y).floor() as usize);
            for r in r0..r1 {
                assert!(!at[r].is_empty(), "row {r} has no cell for {a:?} -> {b:?}");
                assert!(at[r].len() <= 2, "row {r} has {} cells", at[r].len());
                if r > r0 {
                    let (x0, x1) = (at[r - 1][0] as i32, at[r][0] as i32);
                    assert!((x1 - x0).abs() <= 1, "column jump {x0} -> {x1} at row {r}");
                }
            }
            assert!(claims.iter().all(|c| "╱╲│─⎺⎻⎼⎽".contains(c.glyph)));
            // No zig-zag: the line only ever moves one way.
            let dir = (b.x - a.x).signum() as i32;
            for r in r0 + 1..r1 {
                assert!((at[r][0] as i32 - at[r - 1][0] as i32) * dir >= 0, "zig-zag at row {r}");
            }
        }
    }

    #[test]
    fn one_cell_stub_is_removed() {
        // A cube whose camera sees a short far chain would do; test the grid rule directly through the pipeline instead:
        // a `─` stuck to a long `│` goes, the long line stays.
        let mut grid = vec![' '; 10 * 8];
        for r in 0..8 {
            grid[r * 10 + 2] = '│';
        }
        grid[3 * 10 + 3] = '─';
        let stubs = crate::metrics::stub_pieces(&grid, 10, true);
        assert_eq!(stubs, vec![vec![(3, 3)]]);
    }

    #[test]
    fn t_junction_is_not_a_cross() {
        let mk = |id: u64, va: u32, vb: u32, p0: Vec2, p1: Vec2| Entry { cell: 0, obj: 0, id, va, vb, p0, p1, t_mid: 0.5, len_cells: 3.0, elig: 0 };
        let (cx, cy) = (GW as f32 / 2.0, GH as f32 / 2.0);
        let across = mk(1, 1, 2, Vec2::new(0.0, cy), Vec2::new(GW as f32, cy));
        let stem = mk(2, 3, 4, Vec2::new(cx, cy), Vec2::new(cx, GH as f32)); // ends at the centre: a tee
        assert_eq!(through_count(&[&across, &stem]), 1);
        let up = mk(5, 5, 6, Vec2::new(cx, 0.0), Vec2::new(cx, GH as f32));
        assert_eq!(through_count(&[&across, &up]), 2);
    }

    #[test]
    fn chapel_has_no_stray_crosses() {
        // Whole pipeline: crosses stay rare on the chapel (the old typist drew 26 per frame on average).
        let scene = Scene::chapel();
        let cam = scene.bench_camera(130);
        let (cols, rows) = (160, 50);
        let atlas = Atlas::load(DEFAULT_FONT, &all_chars()).unwrap();
        let gb = rasterize(&scene, &cam, cols, rows, 4, 8);
        let o = render_lines(&scene, &cam, &gb, &atlas, &LineParams::default(), &mut LineState::default(), cols, rows);
        let crosses = o.glyphs.iter().filter(|g| **g == Some('┼')).count();
        assert!(crosses <= 20, "{crosses} crosses");
    }

    #[test]
    fn memory_keeps_glyph() {
        let scene = cube_scene();
        let cam = Camera::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO);
        let mut st = LineState::default();
        let mut p = LineParams { run_slope: 0.0, ..LineParams::default() };
        let (o, ..) = run(&scene, &cam, &mut st, &p);
        // Poison the memory: remember '▔' for a key; with a huge margin it must be kept, with zero it must not.
        let &(key, cell, g) = o.ids.iter().find(|i| i.2 == '─').unwrap();
        st.memory.insert(key, '▔');
        p.keep_margin = 100.0;
        p.keep_tol = 100.0;
        p.keep_ink = 100.0;
        let (o2, ..) = run(&scene, &cam, &mut st, &p);
        assert_eq!(o2.glyphs[cell], Some('▔'));
        st.memory.insert(key, '▔');
        p.keep_margin = 0.0;
        let (o3, ..) = run(&scene, &cam, &mut st, &p);
        assert_eq!(o3.glyphs[cell], Some(g));
    }

    fn span(id: u64, va: u32, vb: u32, a: (f32, f32), b: (f32, f32)) -> Span {
        Span { id, obj: 0, va, vb, a: Vec2::new(a.0, a.1), b: Vec2::new(b.0, b.1), sil: false }
    }

    #[test]
    fn parallel_edges_half_cell_apart_draw_once() {
        let spans = [span(1, 0, 1, (0.0, 0.0), (10.0, 0.0)), span(2, 2, 3, (0.0, 0.5), (9.0, 0.5))];
        let dead = cull_edges(&spans, false, true, &Default::default()).0;
        assert_eq!(dead.len(), 1);
        assert!(dead.contains(&2), "the longer edge wins");
        // Far apart: both stay. Off: both stay.
        let far = [span(1, 0, 1, (0.0, 0.0), (10.0, 0.0)), span(2, 2, 3, (0.0, 4.0), (9.0, 4.0))];
        assert!(cull_edges(&far, false, true, &Default::default()).0.is_empty());
        assert!(cull_edges(&spans, false, false, &Default::default()).0.is_empty());
    }

    #[test]
    fn parallel_winner_is_stable() {
        let spans = [span(1, 0, 1, (0.0, 0.0), (10.0, 0.0)), span(2, 2, 3, (0.0, 0.5), (10.5, 0.5))];
        let alive: std::collections::HashSet<u64> = [1].into_iter().collect();
        let dead = cull_edges(&spans, false, true, &alive).0;
        assert!(dead.contains(&2) && !dead.contains(&1), "last frame's survivor keeps the line");
    }

    #[test]
    fn short_isolated_edge_is_dropped_but_chain_stays() {
        let lone = [span(1, 0, 1, (0.0, 0.0), (0.8, 0.0))];
        assert!(cull_edges(&lone, true, false, &Default::default()).0.contains(&1));
        let chain = [span(1, 0, 1, (0.0, 0.0), (1.6, 0.0)), span(2, 1, 2, (1.6, 0.0), (3.2, 0.1))];
        assert!(cull_edges(&chain, true, false, &Default::default()).0.is_empty());
    }

    #[test]
    fn sixteen_sided_pillar_draws_only_rims_and_silhouette() {
        let m = crate::scene::cylinder(0.35, 4.0, 16);
        let edges = build_edges(&m, 0, 35.0);
        // Only the cap rims are hard (horizontal); the vertical side facet edges are silhouette-only.
        let hard: Vec<&Edge> = edges.iter().filter(|e| e.hard).collect();
        assert!(!hard.is_empty());
        assert!(hard.iter().all(|e| (m.verts[e.a as usize].y - m.verts[e.b as usize].y).abs() < 1e-6));
    }
}
