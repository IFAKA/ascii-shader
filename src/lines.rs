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
const TOL: f32 = 1.0;
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
    /// Cells that held a line glyph last frame (presence hysteresis).
    had: Vec<bool>,
    prev: Vec<Option<char>>,
}

/// When the key misses (chain id or bucket changed at a junction), fall back to last frame's glyph in this cell with
/// this fraction of keep_margin.
const FALLBACK_MARGIN: f32 = 0.75;

#[derive(Clone, Debug)]
pub struct LineParams {
    /// Dihedral angle (degrees) above which an edge is a crease. Default 35.
    pub crease_deg: f32,
    /// Keep last frame's glyph if its cost <= best cost + keep_margin (glyph pixel units).
    pub keep_margin: f32,
    /// ... and its cost <= keep_tol.
    pub keep_tol: f32,
    pub lineset: LineSet,
}

impl Default for LineParams {
    fn default() -> Self {
        LineParams { crease_deg: 35.0, keep_margin: 2.0, keep_tol: 5.0, lineset: LineSet::Box }
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
fn trace_edge(scene: &Scene, gb: &GBuffer, vis: &Vis, obj: usize, e: &Edge, out: &mut Vec<Entry>) {
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
            continue;
        }
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
                        None => mine.push((Entry { cell, obj: o16, id: e.id, va: e.a, vb: e.b, p0: q0, p1: q1, t_mid: 0.0, len_cells }, t0, t1)),
                    }
                }
            }
        }
        prev = Some((cur, t));
    }
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
fn glyph_costs(segs: &[(Vec2, Vec2)], glyphs: &[&Glyph], ink_w: &[f32], costs: &mut Vec<f32>) {
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
        for (g, &wsum) in glyphs.iter().zip(ink_w) {
        if g.ink.is_empty() || wsum <= 0.0 {
            costs.push(f32::INFINITY);
            continue;
        }
        let ink: f32 = g.ink.iter().map(|&(x, y, c)| c * (dmap[y as usize * GW + x as usize] - TOL).max(0.0)).sum::<f32>() / wsum;
        let path: f32 = pidx.iter().map(|&i| (g.dist[i].min(CLAMP) - TOL).max(0.0)).sum::<f32>() / pidx.len() as f32;
        costs.push(ink + path + prior(g.ch));
    }
}

/// Chains of entries in one cell: entries of one object that share a mesh vertex (a smooth mesh's silhouette is a
/// zigzag of short edges). Returns `(total length in px, representative entry = smallest edge id)` per chain,
/// longest first.
fn chains(es: &[&Entry]) -> Vec<(f32, usize)> {
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
    let mut out: Vec<(f32, usize)> = vec![];
    let mut roots: Vec<usize> = vec![];
    for i in 0..n {
        let r = find(&mut root, i);
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

struct Decision {
    cell: usize,
    key: u64,
    glyph: char,
    obj: u16,
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
        if horizontal { d.y >= d.x } else { d.x >= 2.0 * d.y }
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
    let mut cand: Vec<(usize, usize)> = vec![];
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
                cand.push((oi, ei));
            }
        }
    }

    let vis = Vis { gb };
    let objs = &state.objs;
    let mut entries: Vec<Entry> = cand
        .par_chunks(64)
        .flat_map_iter(|chunk| {
            let mut v = vec![];
            for &(oi, ei) in chunk {
                trace_edge(scene, gb, &vis, oi, &objs[oi].edges[ei], &mut v);
            }
            v
        })
        .collect();
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
        .filter_map(|&(i, j)| {
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
            glyph_costs(&segs, &glyphs, &ink_w, &mut costs);
            let (bi, &best) = costs.iter().enumerate().min_by(|a, b| a.1.total_cmp(b.1))?;
            if !best.is_finite() {
                return None;
            }
            let bucket = (rep.t_mid * rep.len_cells * 2.0).round() as i64 as u64;
            let key = if ch.len() >= 2 {
                let (ia, ib) = (rep.id, es[ch[1].1].id);
                let (a, b) = (ia.min(ib), ia.max(ib));
                mix(a ^ mix(b ^ mix(bucket)))
            } else {
                mix(rep.id ^ mix(bucket))
            } >> 1;
            let mut pick = bi;
            let cell_prev = state.prev.get(entries[i].cell as usize).copied().flatten();
            let hit = memory.get(&key);
            if let Some(&pg) = hit.or(cell_prev.as_ref())
                && let Some(pi) = glyphs.iter().position(|g| g.ch == pg)
            {
                let margin = if hit.is_some() { params.keep_margin } else { params.keep_margin * FALLBACK_MARGIN };
                if costs[pi] <= best + margin && costs[pi] <= params.keep_tol {
                    pick = pi;
                }
            }
            Some(Decision { cell: entries[i].cell as usize, key, glyph: glyphs[pick].ch, obj: rep.obj, seg: (es[0].p0, es[0].p1), score: main_len + if was { STAIR_BONUS } else { 0.0 }, weak })
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

    let mut used: HashMap<u64, u64> = HashMap::new();
    let mut mem: HashMap<u64, char> = HashMap::new();
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
    fn cube_edges_straight() {
        let scene = cube_scene();
        let cam = Camera::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO);
        let (o, cols, rows) = run(&scene, &cam, &mut LineState::default(), &LineParams::default());
        let at = |x: usize, y: usize| o.glyphs[y * cols + x];
        let mut hz = vec![];
        let mut vt = vec![];
        for y in 0..rows {
            let row: String = (0..cols).map(|x| at(x, y).unwrap_or(' ')).collect();
            let n = row.chars().filter(|&c| c == '─').count();
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

    #[test]
    fn memory_keeps_glyph() {
        let scene = cube_scene();
        let cam = Camera::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO);
        let mut st = LineState::default();
        let mut p = LineParams::default();
        let (o, ..) = run(&scene, &cam, &mut st, &p);
        // Poison the memory: remember '▔' for a key; with a huge margin it must be kept, with zero it must not.
        let &(key, cell, g) = o.ids.iter().find(|i| i.2 == '─').unwrap();
        st.memory.insert(key, '▔');
        p.keep_margin = 100.0;
        p.keep_tol = 100.0;
        let (o2, ..) = run(&scene, &cam, &mut st, &p);
        assert_eq!(o2.glyphs[cell], Some('▔'));
        st.memory.insert(key, '▔');
        p.keep_margin = 0.0;
        let (o3, ..) = run(&scene, &cam, &mut st, &p);
        assert_eq!(o3.glyphs[cell], Some(g));
    }
}
