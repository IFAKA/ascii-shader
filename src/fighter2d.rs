//! Hand-authored 2D cel fighters (Mio, Kage) drawn straight into terminal cells.
//!
//! Units: one unit is one column wide and half a row high, so the unit space is square. A pose lives in a box about
//! 40 units high, faces +x, y up, origin on the ground between the feet. Arms and legs are two-bone IK chains that
//! reach the authored hand and foot points, so bone lengths never change between key poses. Shapes are painted
//! into a 2x2-per-cell canvas and written out as quadrant blocks, so a silhouette is one flat colour per cell.
//! Hair, scarf and coat tails are chains that follow the body with a delay (overlapping action).
use crate::cel::{CYAN, Col, ORANGE, WHITE};
use crate::compose::Cell;
use crate::film::{COLS, PI, ROWS, clamp01, ease_out, h, lerp, line_char, mix3, put, rgb, scale3, smooth};

pub type P = (f32, f32);

fn add(a: P, b: P) -> P {
    (a.0 + b.0, a.1 + b.1)
}
fn sub(a: P, b: P) -> P {
    (a.0 - b.0, a.1 - b.1)
}
fn mul(a: P, k: f32) -> P {
    (a.0 * k, a.1 * k)
}
fn len(a: P) -> f32 {
    (a.0 * a.0 + a.1 * a.1).sqrt()
}
fn norm(a: P) -> P {
    let l = len(a).max(1e-4);
    (a.0 / l, a.1 / l)
}
fn dot(a: P, b: P) -> f32 {
    a.0 * b.0 + a.1 * b.1
}
fn lerp2(a: P, b: P, t: f32) -> P {
    (lerp(a.0, b.0, t), lerp(a.1, b.1, t))
}

#[derive(Clone, Copy, PartialEq)]
pub enum Who {
    Mio,
    Kage,
}

// ---------------------------------------------------------------------------------------------------------------
// Canvas: 2x2 samples per cell
// ---------------------------------------------------------------------------------------------------------------
const SW: usize = COLS * 2;
const SH: usize = ROWS * 2;

pub struct Canvas {
    seq: Vec<u32>,
    col: Vec<[u8; 3]>,
    n: u32,
    dither: bool,
    strokes: Vec<(P, P, Col)>,
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas { seq: vec![0; SW * SH], col: vec![[0; 3]; SW * SH], n: 0, dither: false, strokes: vec![] }
    }
}

impl Canvas {
    fn paint(&mut self, ix: usize, iy: usize, c: Col) {
        if self.dither && (ix ^ iy) & 1 == 0 {
            return;
        }
        self.n += 1;
        let i = iy * SW + ix;
        self.seq[i] = self.n;
        let c = rgb(c);
        self.col[i] = [c.0, c.1, c.2];
    }

    fn range(lo: f32, hi: f32, step: f32, max: usize) -> std::ops::Range<usize> {
        let a = ((lo / step).floor() as i32).max(0) as usize;
        let b = ((hi / step).ceil() as i32).clamp(0, max as i32) as usize;
        a.min(max)..b
    }

    /// Tapered capsule from `a` (radius `r0`) to `b` (radius `r1`).
    pub fn capsule(&mut self, a: P, b: P, r0: f32, r1: f32, c: Col) {
        let rm = r0.max(r1);
        let (x0, x1) = (a.0.min(b.0) - rm, a.0.max(b.0) + rm);
        let (y0, y1) = (a.1.min(b.1) - rm, a.1.max(b.1) + rm);
        let d = sub(b, a);
        let l2 = dot(d, d).max(1e-6);
        for iy in Self::range(y0, y1, 1.0, SH) {
            for ix in Self::range(x0, x1, 0.5, SW) {
                let p = ((ix as f32 + 0.5) * 0.5, iy as f32 + 0.5);
                let t = (dot(sub(p, a), d) / l2).clamp(0.0, 1.0);
                let q = add(a, mul(d, t));
                if len(sub(p, q)) <= lerp(r0, r1, t) {
                    self.paint(ix, iy, c);
                }
            }
        }
    }

    pub fn disc(&mut self, a: P, r: f32, c: Col) {
        self.capsule(a, a, r, r, c);
    }

    /// Filled polygon (even-odd).
    pub fn poly(&mut self, pts: &[P], c: Col) {
        if pts.len() < 3 {
            return;
        }
        let y0 = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min);
        let y1 = pts.iter().map(|p| p.1).fold(f32::MIN, f32::max);
        for iy in Self::range(y0, y1, 1.0, SH) {
            let py = iy as f32 + 0.5;
            // crossings of this scan line
            let mut xs: Vec<f32> = vec![];
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                if (a.1 <= py) != (b.1 <= py) {
                    xs.push(a.0 + (py - a.1) / (b.1 - a.1) * (b.0 - a.0));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for w in xs.chunks(2) {
                if w.len() == 2 {
                    for ix in Self::range(w[0] - 0.25, w[1] + 0.25, 0.5, SW) {
                        let px = (ix as f32 + 0.5) * 0.5;
                        if px >= w[0] && px <= w[1] {
                            self.paint(ix, iy, c);
                        }
                    }
                }
            }
        }
    }

    /// A line drawn with line glyphs on top of everything (blade, streak edges).
    pub fn stroke(&mut self, a: P, b: P, c: Col) {
        self.strokes.push((a, b, c));
    }

    pub fn covered(&self, cx: usize, cy: usize) -> bool {
        let i = cy * 2 * SW + cx * 2;
        self.seq[i] != 0 || self.seq[i + 1] != 0 || self.seq[i + SW] != 0 || self.seq[i + SW + 1] != 0
    }

    /// Blank the cells around every covered cell so lines behind the fighters do not touch them.
    pub fn clear_halo(&self, cells: &mut [Cell], halo: i32) {
        let mut hit = vec![false; COLS * ROWS];
        for cy in 0..ROWS {
            for cx in 0..COLS {
                if self.covered(cx, cy) {
                    hit[cy * COLS + cx] = true;
                }
            }
        }
        for cy in 0..ROWS as i32 {
            for cx in 0..COLS as i32 {
                'o: for dy in -halo..=halo {
                    for dx in -halo..=halo {
                        let (x, y) = (cx + dx, cy + dy);
                        if x >= 0 && y >= 0 && (x as usize) < COLS && (y as usize) < ROWS && hit[y as usize * COLS + x as usize] {
                            cells[cy as usize * COLS + cx as usize] = crate::compose::BLANK;
                            break 'o;
                        }
                    }
                }
            }
        }
    }

    /// Write blocks, then line strokes, into `cells`.
    pub fn blit(&self, cells: &mut [Cell]) {
        const G: [char; 16] = [' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█'];
        for cy in 0..ROWS {
            for cx in 0..COLS {
                let idx = [cy * 2 * SW + cx * 2, cy * 2 * SW + cx * 2 + 1, (cy * 2 + 1) * SW + cx * 2, (cy * 2 + 1) * SW + cx * 2 + 1];
                let mut bits = 0;
                let mut best = (0u32, 0usize);
                for (k, &i) in idx.iter().enumerate() {
                    if self.seq[i] != 0 {
                        bits |= 1 << k;
                        if self.seq[i] > best.0 {
                            best = (self.seq[i], i);
                        }
                    }
                }
                if bits != 0 {
                    let c = self.col[best.1];
                    cells[cy * COLS + cx] = Cell { ch: G[bits], fg: (c[0], c[1], c[2]), bold: true };
                }
            }
        }
        for &(a, b, c) in &self.strokes {
            let n = (len(sub(b, a)) / 0.5).ceil() as i32 + 1;
            let ch = line_char(b.0 - a.0, b.1 - a.1);
            for i in 0..=n {
                let p = lerp2(a, b, i as f32 / n as f32);
                put(cells, p.0.floor() as i32, (p.1 / 2.0).floor() as i32, ch, c, true);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Poses
// ---------------------------------------------------------------------------------------------------------------
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub hip: P,
    pub chest: P,
    pub head: P,
    /// Near (sword) hand and far hand, near and far foot.
    pub hn: P,
    pub hf: P,
    pub fnr: P,
    pub ffr: P,
    /// Blade angle in degrees, 0 = forward, 90 = up, 180 = back.
    pub bl: f32,
}

#[allow(clippy::too_many_arguments)]
const fn pz(hip: P, chest: P, head: P, hn: P, hf: P, fnr: P, ffr: P, bl: f32) -> Pose {
    Pose { hip, chest, head, hn, hf, fnr, ffr, bl }
}

impl Pose {
    pub fn lerp(&self, o: &Pose, t: f32) -> Pose {
        let a = ((o.bl - self.bl + 540.0).rem_euclid(360.0)) - 180.0;
        Pose {
            hip: lerp2(self.hip, o.hip, t),
            chest: lerp2(self.chest, o.chest, t),
            head: lerp2(self.head, o.head, t),
            hn: lerp2(self.hn, o.hn, t),
            hf: lerp2(self.hf, o.hf, t),
            fnr: lerp2(self.fnr, o.fnr, t),
            ffr: lerp2(self.ffr, o.ffr, t),
            bl: self.bl + a * t,
        }
    }
}

// Key poses (Mio proportions; Kage uses the same data, scaled up a little at draw time).
pub const STAND: Pose = pz((0.0, 19.6), (0.3, 32.0), (0.8, 37.6), (3.8, 20.2), (-0.6, 17.6), (2.5, 0.0), (-2.5, 0.0), 200.0);
pub const HILT: Pose = pz((-0.3, 17.8), (2.0, 30.0), (3.0, 35.6), (6.5, 21.5), (-3.0, 19.0), (5.0, 0.0), (-5.0, 0.0), 195.0);
pub const READY: Pose = pz((-0.5, 15.5), (2.5, 28.0), (4.5, 33.5), (10.0, 26.0), (6.2, 24.6), (7.5, 0.0), (-7.0, 0.0), 20.0);
pub const DRAW: Pose = pz((2.0, 14.5), (7.0, 26.0), (10.0, 31.5), (14.0, 24.0), (-2.0, 20.0), (11.0, 0.0), (-9.0, 0.0), 5.0);
pub const ANTICIP: Pose = pz((-4.5, 13.0), (-7.0, 25.0), (-7.5, 30.5), (-8.0, 27.0), (-11.5, 26.0), (3.0, 0.0), (-10.5, 0.0), 8.0);
pub const STRIKE: Pose = pz((4.0, 15.0), (12.0, 25.5), (16.5, 30.5), (22.0, 27.0), (18.2, 26.0), (12.0, 0.0), (-10.5, 1.0), 3.0);
pub const CLASH: Pose = pz((1.0, 14.0), (5.5, 25.5), (8.5, 31.0), (14.0, 28.0), (10.5, 26.0), (9.0, 0.0), (-8.5, 0.0), 52.0);
pub const FOLLOW: Pose = pz((0.0, 11.5), (5.0, 22.5), (8.5, 27.5), (14.0, 13.5), (9.0, 12.0), (9.0, 0.5), (-9.0, 0.0), 8.0);
pub const RISE: Pose = pz((0.0, 16.0), (1.5, 28.5), (2.5, 34.0), (9.0, 24.0), (2.0, 22.0), (4.0, 0.0), (-4.0, 0.0), 70.0);
pub const SHEATHED: Pose = pz((0.0, 19.4), (0.8, 31.8), (1.8, 36.6), (3.6, 19.8), (-0.8, 17.8), (2.5, 0.0), (-2.5, 0.0), 205.0);
pub const HIT: Pose = pz((-2.0, 19.0), (-5.0, 31.0), (-7.2, 36.5), (-4.0, 27.0), (-9.0, 25.0), (2.0, 0.0), (-6.0, 0.5), -100.0);
pub const PARRY: Pose = pz((-0.5, 16.0), (0.0, 28.5), (1.5, 34.0), (7.0, 40.0), (3.2, 38.5), (7.0, 0.0), (-6.0, 0.0), 40.0);
pub const LOW: Pose = pz((2.0, 12.5), (8.0, 22.0), (11.5, 27.0), (15.0, 10.0), (11.0, 12.0), (11.0, 0.5), (-11.0, 0.0), -15.0);
pub const JUMP: Pose = pz((0.0, 30.0), (2.0, 42.0), (3.5, 47.5), (9.0, 50.0), (5.0, 49.0), (6.0, 22.0), (-4.0, 18.0), 120.0);
pub const SPIN: Pose = pz((0.0, 15.0), (-1.0, 27.5), (-0.5, 33.0), (6.5, 25.0), (-5.0, 25.0), (6.0, 0.0), (-6.0, 0.0), 178.0);
pub const SLASHD: Pose = pz((2.0, 14.0), (7.0, 26.0), (10.5, 31.5), (19.0, 22.0), (15.0, 25.0), (10.0, 0.0), (-10.0, 0.0), -40.0);
/// Sprint with the blade trailing behind; four drawings make one cycle.
pub const RUN: [Pose; 4] = [
    pz((2.0, 18.3), (10.0, 28.5), (14.5, 32.5), (1.0, 19.0), (14.5, 26.5), (11.0, 3.0), (-12.0, 2.0), 195.0),
    pz((2.0, 16.6), (10.0, 27.0), (14.5, 31.0), (-1.0, 18.0), (7.0, 22.0), (0.0, 9.0), (-3.0, 1.0), 192.0),
    pz((2.0, 18.3), (10.0, 28.5), (14.5, 32.5), (1.0, 19.0), (-4.0, 22.0), (-12.0, 2.0), (11.0, 3.0), 195.0),
    pz((2.0, 16.6), (10.0, 27.0), (14.5, 31.0), (-1.0, 18.0), (7.0, 22.0), (2.0, 1.0), (0.0, 9.0), 192.0),
];

/// One run drawing per 1/12 s: the cycle is four drawings.
pub fn run_pose(t: f32) -> Pose {
    RUN[(((t * 12.0 + 0.01).floor() as i32).rem_euclid(4)) as usize]
}

#[derive(Clone, Copy)]
pub enum Ease {
    Out,
    In,
    Io,
    Back,
}

fn ease(e: Ease, u: f32) -> f32 {
    let u = clamp01(u);
    match e {
        Ease::Out => ease_out(u),
        Ease::In => u * u * u,
        Ease::Io => smooth(u),
        Ease::Back => {
            let c = 1.9;
            1.0 + (c + 1.0) * (u - 1.0).powi(3) + c * (u - 1.0).powi(2)
        }
    }
}

/// A pose that is reached at time `t` after moving for `dur` seconds with easing `e`.
pub struct Key(pub f32, pub Pose, pub f32, pub Ease);

pub fn seq(keys: &[Key], t: f32) -> Pose {
    let mut cur = keys[0].1;
    for w in keys.windows(2) {
        let start = w[1].0 - w[1].2;
        if t >= w[1].0 {
            cur = w[1].1;
        } else if t > start {
            return w[0].1.lerp(&w[1].1, ease(w[1].3, (t - start) / w[1].2));
        } else {
            break;
        }
    }
    cur
}

/// Placement of a fighter on the screen.
#[derive(Clone, Copy)]
pub struct Fx {
    /// Column of the ground origin.
    pub x: f32,
    /// Ground line in units (2 per row).
    pub gy: f32,
    pub s: f32,
    /// +1 faces right, -1 faces left.
    pub dir: f32,
}

#[derive(Clone, Copy)]
pub struct Fs {
    pub pose: Pose,
    pub fx: Fx,
    /// Idle motion strength (breathing, tremble).
    pub idle: f32,
}

/// Small idle motion so a held drawing is never dead.
fn alive(p: Pose, t: f32, k: f32) -> Pose {
    let b = (t * 2.0 * PI / 1.7).sin() * k;
    let b2 = (t * 2.0 * PI / 1.7 - 0.7).sin() * k;
    let mut q = p;
    q.chest.1 += 0.45 * b;
    q.head.1 += 0.6 * b2;
    q.head.0 += 0.25 * b2;
    q.hn.1 += 0.5 * b2;
    q.hf.1 += 0.5 * b2;
    q.hip.1 -= 0.15 * b;
    q.bl += 1.6 * b2;
    q
}

pub fn fs(pose: Pose, fx: Fx, t: f32, idle: f32) -> Fs {
    Fs { pose: alive(pose, t, idle), fx, idle }
}

// ---------------------------------------------------------------------------------------------------------------
// Drawing a fighter
// ---------------------------------------------------------------------------------------------------------------
struct Pal {
    main: Col,
    shade: Col,
    light: Col,
    hair: Col,
    skin: Col,
}

fn pal(w: Who) -> Pal {
    match w {
        Who::Mio => Pal { main: CYAN, shade: [0.0, 120.0, 160.0], light: [200.0, 252.0, 255.0], hair: [0.0, 130.0, 190.0], skin: [225.0, 250.0, 255.0] },
        Who::Kage => Pal { main: ORANGE, shade: [165.0, 75.0, 0.0], light: [255.0, 232.0, 160.0], hair: [215.0, 95.0, 0.0], skin: [255.0, 232.0, 190.0] },
    }
}

const UA: f32 = 7.0;
const FA: f32 = 7.2;
const TH: f32 = 10.2;
const SHN: f32 = 10.0;

fn ik(root: P, end: P, l1: f32, l2: f32, hint: P) -> (P, P) {
    let d = sub(end, root);
    let dist = len(d).clamp(0.6, l1 + l2 - 0.02);
    let u = norm(d);
    let end = add(root, mul(u, dist));
    let a = (l1 * l1 - l2 * l2 + dist * dist) / (2.0 * dist);
    let hh = (l1 * l1 - a * a).max(0.0).sqrt();
    let base = add(root, mul(u, a));
    let perp = (-u.1, u.0);
    let c1 = add(base, mul(perp, hh));
    let c2 = sub(base, mul(perp, hh));
    let m = if dot(sub(c1, base), hint) >= dot(sub(c2, base), hint) { c1 } else { c2 };
    (m, end)
}

/// Joints in pose space after IK.
struct Joints {
    hip: P,
    chest: P,
    head: P,
    sh: P,
    el_n: P,
    ha_n: P,
    el_f: P,
    ha_f: P,
    kn_n: P,
    ft_n: P,
    kn_f: P,
    ft_f: P,
    sword: P,
}

fn joints(p: &Pose, who: Who) -> Joints {
    let sh = add(p.chest, (0.0, -1.6));
    let (el_n, ha_n) = ik(sh, p.hn, UA, FA, (-0.4, -1.0));
    let (el_f, ha_f) = ik(sh, p.hf, UA, FA, (-0.4, -1.0));
    let hp = add(p.hip, (0.0, -0.6));
    let (kn_n, ft_n) = ik(hp, p.fnr, TH, SHN, (1.0, 0.1));
    let (kn_f, ft_f) = ik(hp, p.ffr, TH, SHN, (1.0, 0.1));
    let a = p.bl.to_radians();
    let _ = who;
    Joints { hip: p.hip, chest: p.chest, head: p.head, sh, el_n, ha_n, el_f, ha_f, kn_n, ft_n, kn_f, ft_f, sword: (a.cos(), a.sin()) }
}

fn blade_len(w: Who) -> f32 {
    if w == Who::Mio { 21.0 } else { 24.0 }
}

fn map(fx: &Fx, p: P) -> P {
    (fx.x + fx.dir * fx.s * p.0, fx.gy - fx.s * p.1)
}

/// Screen position of the blade tip and of a point 30% along the blade.
pub fn blade(fs: &Fs, who: Who) -> (P, P, P) {
    let j = joints(&fs.pose, who);
    let l = blade_len(who);
    let base = add(j.ha_n, mul(j.sword, -2.5));
    let tip = add(j.ha_n, mul(j.sword, l));
    (map(&fs.fx, base), map(&fs.fx, add(j.ha_n, mul(j.sword, l * 0.3))), map(&fs.fx, tip))
}

/// Screen anchors that the cloth chains hang from.
fn anchor(fs: &Fs, who: Who, which: u8) -> P {
    let p = &fs.pose;
    let a = match (who, which) {
        (_, 0) => add(p.head, (-2.6, 2.0)),
        (_, 1) => add(p.chest, (-1.4, 0.6)),
        (_, _) => add(p.hip, (-2.0, 2.6)),
    };
    map(&fs.fx, a)
}

/// A delayed-trail chain: segment `i` follows the anchor as it was `i * delay` seconds ago, plus a rest
/// direction and a little wave, with every segment kept at a fixed length.
#[allow(clippy::too_many_arguments)]
fn chain(at: &dyn Fn(f32) -> Fs, who: Who, which: u8, t: f32, tmin: f32, n: usize, seg: f32, delay: f32, rest: P, wave: f32, phase: f32) -> Vec<P> {
    let now = at(t);
    let mut pts = vec![anchor(&now, who, which)];
    let mut cum = (0.0, 0.0);
    for i in 1..=n {
        let tau = (t - i as f32 * delay).max(tmin);
        let a = anchor(&at(tau), who, which);
        let r = norm((rest.0 * now.fx.dir, -rest.1));
        cum = add(cum, mul(r, seg * now.fx.s));
        let w = (t * 9.0 + phase + i as f32 * 0.9).sin() * wave * (i as f32 / n as f32) * now.fx.s;
        let target = add(add(a, cum), (0.0, w));
        let prev = pts[i - 1];
        let d = sub(target, prev);
        pts.push(add(prev, mul(norm(d), seg * now.fx.s)));
    }
    pts
}

fn ribbon(cv: &mut Canvas, pts: &[P], r0: f32, r1: f32, c: Col, s: f32) {
    let n = pts.len() - 1;
    for i in 0..n {
        let (f0, f1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
        cv.capsule(pts[i], pts[i + 1], lerp(r0, r1, f0) * s, lerp(r0, r1, f1) * s, c);
    }
}

/// Everything about one drawing: ghosts, cloth, body, blade and its streak. `tmin` is the start of the shot.
pub fn draw(cv: &mut Canvas, who: Who, at: &dyn Fn(f32) -> Fs, t: f32, tmin: f32) {
    let cur = at(t);
    let pl = pal(who);
    let big = if who == Who::Kage { 1.1 } else { 1.0 };
    // speed over the last 0.04 s decides how much smear to draw
    let prev = at((t - 0.04).max(tmin));
    let (hn0, hn1) = (map(&prev.fx, prev.pose.hip), map(&cur.fx, cur.pose.hip));
    let sp_body = len(sub(hn1, hn0));
    let sp_tip = len(sub(blade(&cur, who).2, blade(&prev, who).2));
    // 1. ghosts
    if sp_body > 2.2 {
        let n = ((sp_body / 2.5) as usize).clamp(1, 3);
        cv.dither = true;
        for k in (1..=n).rev() {
            let g = at((t - 0.035 * k as f32).max(tmin));
            let c = scale3(pl.main, 0.38 - 0.08 * k as f32);
            body(cv, &g, who, big, [c, c, c, c, c], false);
        }
        cv.dither = false;
    }
    // 2. blade streak: the outer part of the blade swept over the last moments
    if sp_tip > 3.0 {
        let steps = 7;
        let span = (0.10 + (sp_tip / 60.0).min(0.07)).min(0.17);
        let mut last: Option<(P, P)> = None;
        for k in 0..=steps {
            let tau = (t - span * k as f32 / steps as f32).max(tmin);
            let (b0, _, tip) = blade(&at(tau), who);
            let inner = lerp2(b0, tip, 0.6);
            if let Some((m0, t0)) = last {
                let f = k as f32 / steps as f32;
                let col = if f < 0.25 { mix3(WHITE, pl.light, f / 0.25) } else { mix3(pl.light, scale3(pl.main, 0.3), (f - 0.25) / 0.75) };
                cv.poly(&[m0, t0, tip, inner], col);
            }
            last = Some((inner, tip));
        }
    }
    // 3. cloth behind the body
    let wave = 0.8 + (sp_body / 3.0).min(2.5);
    let s = cur.fx.s;
    match who {
        Who::Mio => {
            let tail = chain(at, who, 1, t, tmin, 7, 3.0, 0.03, (-0.92, -0.3), wave, 0.0);
            ribbon(cv, &tail, 1.6, 0.4, pl.light, s);
            let tail2 = chain(at, who, 1, t, tmin, 5, 2.6, 0.045, (-0.85, -0.45), wave, 2.0);
            ribbon(cv, &tail2, 1.2, 0.3, mix3(pl.light, pl.main, 0.5), s);
            let pony = chain(at, who, 0, t, tmin, 7, 2.6, 0.04, (-0.55, -0.85), wave, 4.0);
            ribbon(cv, &pony, 1.5, 0.3, pl.hair, s);
        }
        Who::Kage => {
            for (k, (dx, dr)) in [(0.0f32, 1.0f32), (0.7, 0.8)].iter().enumerate() {
                let c = chain(at, who, 2, t, tmin, 5, 3.6, 0.035 + 0.01 * k as f32, (-0.5 - dx * 0.3, -0.85), wave * *dr, k as f32 * 1.7);
                ribbon(cv, &c, 2.7 - k as f32 * 0.4, 0.7, if k == 0 { scale3(pl.main, 0.6) } else { scale3(pl.main, 0.85) }, s);
            }
        }
    }
    // 4. the body
    let drag = sub(map(&prev.fx, prev.pose.head), map(&cur.fx, cur.pose.head));
    body_detail(cv, &cur, who, big, &pl, drag);
    // 5. blade (a scabbard when it points back and down)
    let j = joints(&cur.pose, who);
    let bs = blade(&cur, who);
    let hilt = map(&cur.fx, add(j.ha_n, mul(j.sword, -3.0)));
    let guard = map(&cur.fx, add(j.ha_n, mul(j.sword, 1.2)));
    let gp = (-j.sword.1 * cur.fx.dir, -j.sword.0);
    let gv = mul(gp, 2.1 * s);
    let sheathed = cur.pose.bl > 150.0 && cur.pose.bl < 270.0;
    cv.capsule(hilt, bs.0, 0.9 * s, 0.9 * s, pl.shade);
    if sheathed {
        let end = lerp2(bs.0, bs.2, 0.72);
        cv.capsule(guard, end, 1.0 * s, 0.8 * s, scale3(pl.shade, 0.7));
        cv.capsule(add(guard, gv), sub(guard, gv), 0.6 * s, 0.6 * s, pl.light);
    } else {
        cv.capsule(add(guard, gv), sub(guard, gv), 0.6 * s, 0.6 * s, pl.light);
        cv.capsule(bs.0, bs.2, 1.7 * s, 0.9 * s, scale3(pl.main, 0.5));
        cv.capsule(lerp2(bs.0, bs.2, 0.1), bs.2, 0.62 * s, 0.4 * s, WHITE);
    }
    // 6. front cloth (Mio scarf end flutters over the shoulder)
    if who == Who::Mio {
        let sc = chain(at, who, 1, t, tmin, 2, 2.0, 0.02, (-0.5, -0.9), wave * 0.5, 1.0);
        ribbon(cv, &sc, 1.4, 0.9, mix3(pl.light, pl.main, 0.4), s);
    }
}

/// Flat or detailed limb set. `cols` = [far limb, torso, near limb, head, extra].
fn body(cv: &mut Canvas, f: &Fs, who: Who, big: f32, cols: [Col; 5], _detail: bool) {
    let j = joints(&f.pose, who);
    let s = f.fx.s * big;
    let m = |p: P| map(&f.fx, p);
    let (tw, hw) = if who == Who::Mio { (2.6, 2.3) } else { (3.2, 2.7) };
    limb(cv, &[m(j.sh), m(j.el_f), m(j.ha_f)], [1.5, 1.3, 1.1], s, cols[0]);
    limb(cv, &[m(add(j.hip, (0.0, -0.6))), m(j.kn_f), m(j.ft_f)], [2.4, 1.9, 1.3], s, cols[0]);
    torso(cv, f, tw, hw, s, cols[1]);
    cv.disc(m(j.head), 3.3 * s, cols[3]);
    limb(cv, &[m(add(j.hip, (0.0, -0.6))), m(j.kn_n), m(j.ft_n)], [2.4, 1.9, 1.3], s, cols[2]);
    limb(cv, &[m(j.sh), m(j.el_n), m(j.ha_n)], [1.5, 1.3, 1.1], s, cols[2]);
}

fn limb(cv: &mut Canvas, p: &[P; 3], r: [f32; 3], s: f32, c: Col) {
    cv.capsule(p[0], p[1], r[0] * s, r[1] * s, c);
    cv.capsule(p[1], p[2], r[1] * s, r[2] * s, c);
}

fn torso(cv: &mut Canvas, f: &Fs, tw: f32, hw: f32, s: f32, c: Col) {
    let (hip, chest) = (f.pose.hip, f.pose.chest);
    let sp = norm(sub(chest, hip));
    let n = (sp.1, -sp.0);
    let m = |p: P| map(&f.fx, p);
    let big = s / f.fx.s;
    let pts = [
        m(add(add(chest, mul(n, tw * big)), (0.0, -0.5))),
        m(add(chest, mul(n, -tw * big * 0.9))),
        m(add(hip, mul(n, -hw * big))),
        m(add(hip, mul(n, hw * big))),
    ];
    cv.poly(&pts, c);
}

fn body_detail(cv: &mut Canvas, f: &Fs, who: Who, big: f32, pl: &Pal, drag: P) {
    let j = joints(&f.pose, who);
    let s = f.fx.s * big;
    let m = |p: P| map(&f.fx, p);
    let dir = f.fx.dir;
    let (tw, hw) = if who == Who::Mio { (2.7, 2.3) } else { (3.3, 2.8) };
    let hp = m(add(j.hip, (0.0, -0.6)));
    // far limbs (dark)
    let far = scale3(pl.shade, 0.62);
    limb(cv, &[m(j.sh), m(j.el_f), m(j.ha_f)], [1.5, 1.3, 1.1], s, far);
    cv.disc(m(j.ha_f), 1.3 * s, scale3(pl.skin, 0.6));
    limb(cv, &[hp, m(j.kn_f), m(j.ft_f)], [2.4, 1.9, 1.4], s, far);
    cv.capsule(m(j.ft_f), m(add(j.ft_f, (2.2, 0.0))), 1.0 * s, 0.9 * s, far);
    // torso, with a coat skirt for Kage and a short skirt for Mio
    let sp = norm(sub(j.chest, j.hip));
    let n = (sp.1, -sp.0);
    let sk = if who == Who::Kage { 7.0 } else { 3.6 };
    let down = (0.0, -1.0);
    let skirt = [
        m(add(j.hip, mul(n, hw))),
        m(add(add(j.hip, mul(n, -hw)), (0.0, 0.0))),
        m(add(add(j.hip, mul(n, -hw - 1.6)), mul(down, sk))),
        m(add(add(j.hip, mul(n, hw + 1.4)), mul(down, sk * 0.85))),
    ];
    cv.poly(&skirt, if who == Who::Kage { scale3(pl.main, 0.85) } else { scale3(pl.shade, 0.8) });
    torso(cv, f, tw, hw, s, mix3(pl.main, pl.shade, 0.5));
    // belt and chest highlight
    cv.capsule(m(add(j.hip, mul(n, hw))), m(add(j.hip, mul(n, -hw))), 0.9 * s, 0.9 * s, pl.light);
    cv.capsule(m(add(j.chest, add(mul(n, tw * 0.7), (0.0, -2.0)))), m(add(j.hip, add(mul(n, hw * 0.6), (0.0, 3.0)))), 0.6 * s, 0.6 * s, pl.light);
    // near leg (trousers, shade), boot
    limb(cv, &[hp, m(j.kn_n), m(j.ft_n)], [2.5, 2.0, 1.4], s, if who == Who::Kage { scale3(pl.shade, 0.75) } else { scale3(pl.shade, 0.9) });
    cv.capsule(m(j.ft_n), m(add(j.ft_n, (2.6, 0.0))), 1.1 * s, 1.0 * s, pl.light);
    // neck, head and hair
    cv.capsule(m(add(j.chest, (0.0, -1.0))), m(j.head), 1.5 * s, 1.5 * s, scale3(pl.skin, 0.8));
    let hc = m(j.head);
    let rh = 3.3 * s;
    cv.disc(hc, rh, pl.skin);
    let back = m(add(j.head, (-1.5, 0.9)));
    match who {
        Who::Mio => {
            cv.disc(back, rh * 0.95, pl.hair);
            // fringe sweeping over the face
            cv.poly(&[m(add(j.head, (-0.5, 3.6))), m(add(j.head, (3.8, 1.9))), m(add(j.head, (3.0, 0.9))), m(add(j.head, (0.5, 1.6)))], pl.hair);
            cv.capsule(m(add(j.head, (1.2, 2.0))), add(m(add(j.head, (4.4, -1.0))), mul(drag, 0.3)), 0.9 * s, 0.4 * s, pl.hair);
        }
        Who::Kage => {
            cv.disc(back, rh * 0.95, pl.hair);
            cv.poly(&[m(add(j.head, (-0.5, 3.8))), m(add(j.head, (3.6, 2.0))), m(add(j.head, (3.0, 1.0))), m(add(j.head, (0.2, 1.8)))], pl.hair);
            for (k, a) in [20.0f32, 50.0, 80.0, 112.0, 142.0].iter().enumerate() {
                let r = a.to_radians();
                let l = 6.5 + 2.5 * h(k as u32 + 5);
                let tip0 = add(j.head, (r.cos() * (3.3 + l), r.sin() * (3.3 + l) * 0.9 + 0.5));
                let tip = add(m(tip0), mul(drag, 0.45 + 0.1 * k as f32));
                let (b1, b2) = (r - 0.32, r + 0.32);
                let base1 = m(add(j.head, (b1.cos() * 3.2, b1.sin() * 3.2)));
                let base2 = m(add(j.head, (b2.cos() * 3.2, b2.sin() * 3.2)));
                cv.poly(&[base1, tip, base2], if k % 2 == 0 { pl.hair } else { mix3(pl.hair, pl.light, 0.45) });
            }
        }
    }
    // eye: one bright dot
    cv.disc(m(add(j.head, (1.9, 0.0))), 0.45 * s, if who == Who::Mio { [20.0, 60.0, 100.0] } else { [90.0, 20.0, 0.0] });
    // near arm in front (sleeve + glove)
    limb(cv, &[m(j.sh), m(j.el_n), m(j.ha_n)], [1.7, 1.45, 1.2], s, pl.main);
    cv.capsule(m(j.el_n), m(j.ha_n), 1.2 * s, 1.15 * s, pl.light);
    cv.disc(m(j.ha_n), 1.35 * s, pl.skin);
    // far hand holds the grip
    cv.disc(m(j.ha_f), 1.2 * s, pl.skin);
    let _ = dir;
}

/// Radial spark lines from a point.
pub fn sparks(cells: &mut [Cell], at: P, k: f32, seed: u32, a: Col, b: Col) {
    for i in 0..34u32 {
        let ang = h(seed + i * 5) * 2.0 * PI;
        let l = (3.0 + 17.0 * h(seed + i * 5 + 1)) * (0.3 + k);
        let (dx, dy) = (ang.cos(), ang.sin());
        let ch = line_char(dx, dy);
        let mut r = 1.5;
        while r < l {
            let (x, y) = ((at.0 + dx * r).floor() as i32, ((at.1 + dy * r) / 2.0).floor() as i32);
            let f = r / l;
            put(cells, x, y, ch, mix3(a, b, f), f < 0.5);
            r += 0.6;
        }
    }
}

/// A dev sheet of every key pose, both fighters, for judging drawings in one still.
pub fn pose_sheet() -> Vec<crate::film::FilmFrame> {
    let all: [(&str, Pose); 17] = [
        ("stand", STAND),
        ("hilt", HILT),
        ("ready", READY),
        ("draw", DRAW),
        ("anticip", ANTICIP),
        ("strike", STRIKE),
        ("clash", CLASH),
        ("follow", FOLLOW),
        ("rise", RISE),
        ("sheathed", SHEATHED),
        ("hit", HIT),
        ("parry", PARRY),
        ("low", LOW),
        ("jump", JUMP),
        ("spin", SPIN),
        ("slashd", SLASHD),
        ("run0", RUN[0]),
    ];
    let page: usize = std::env::var("POSES_PAGE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut cells = vec![crate::compose::BLANK; COLS * ROWS];
    let mut cv = Canvas::default();
    for (k, (name, p)) in all.iter().skip(page * 4).take(4).enumerate() {
        for (row, who) in [Who::Mio, Who::Kage].into_iter().enumerate() {
            let fx = Fx { x: 20.0 + k as f32 * 40.0, gy: 42.0 + row as f32 * 44.0, s: 1.0, dir: 1.0 };
            let st = fs(*p, fx, 0.0, 0.0);
            let at = move |_t: f32| st;
            draw(&mut cv, who, &at, 0.0, 0.0);
        }
        for (i, ch) in name.chars().enumerate() {
            put(&mut cells, 10 + k as i32 * 40 + i as i32, 0, ch, [200.0; 3], false);
        }
    }
    cv.blit(&mut cells);
    vec![crate::film::FilmFrame { cells, bg: (0, 0, 0) }]
}
