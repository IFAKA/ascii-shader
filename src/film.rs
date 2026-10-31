//! The 30 s anime short ORB, made only of terminal cells (glyph + colour, one background colour per frame).
//! `prerender` turns the script into a list of cell frames. The same list is played in the terminal
//! (`--play-film`) and drawn with the font into an MP4 (`--film`), so both show the same thing.
use crate::compose::{BLANK, Cell};
use crate::glyphs::{self, Atlas, GH, GW};
use crate::render::{Options, Renderer};
use crate::scene::{self, Camera, Object, Scene};
use crossterm::{
    cursor, event::{self, Event, KeyCode, KeyEventKind}, execute, queue,
    style::{Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use glam::{Mat4, Vec3, Vec4};
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const COLS: usize = 160;
pub const ROWS: usize = 45;
pub(crate) const W: usize = COLS * GW;
pub(crate) const H: usize = ROWS * GH;
pub(crate) const FPS: usize = 24;
const SECS: f32 = 30.0;
/// Letterbox: this many blank rows at the top and the bottom.
pub(crate) const BAR_ROWS: usize = 4;
pub(crate) const PI: f32 = std::f32::consts::PI;
pub(crate) const INK: [f32; 3] = [14.0, 10.0, 22.0];
pub(crate) const PAPER: [f32; 3] = [238.0, 236.0, 230.0];

/// (start s, end s, frames per drawing). Most shots are drawn on twos (12 drawings a second), like cel anime.
const SHOTS: [(f32, f32, usize); 7] = [(0.0, 5.0, 2), (5.0, 9.0, 2), (9.0, 13.0, 2), (13.0, 17.0, 1), (17.0, 21.5, 1), (21.5, 26.5, 1), (26.5, 30.0, 2)];
/// Frames drawn as a dark-ink-on-paper impact frame.
const IMPACT: [usize; 4] = [408, 409, 516, 517];

const SUBTITLES: [(f32, f32, &str); 5] = [
    (1.0, 4.3, "before dawn."),
    (5.6, 8.6, "something waits at the altar."),
    (9.6, 12.6, "it has slept a long time."),
    (13.4, 16.6, "it is waking."),
    (21.9, 24.6, "go."),
];

/// One frame of the film: cells plus the background colour of the whole screen.
#[derive(Clone)]
pub struct FilmFrame {
    pub cells: Vec<Cell>,
    pub bg: (u8, u8, u8),
}

pub(crate) fn h(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x & 0xFF_FFFF) as f32 / 16_777_216.0
}
pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
pub(crate) fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}
pub(crate) fn smooth(x: f32) -> f32 {
    let x = clamp01(x);
    x * x * (3.0 - 2.0 * x)
}
pub(crate) fn ease_out(x: f32) -> f32 {
    1.0 - (1.0 - clamp01(x)).powi(3)
}
pub(crate) fn ease_in(x: f32) -> f32 {
    clamp01(x).powi(3)
}
pub(crate) fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]
}
pub(crate) fn rgb(c: [f32; 3]) -> (u8, u8, u8) {
    (c[0].clamp(0.0, 255.0) as u8, c[1].clamp(0.0, 255.0) as u8, c[2].clamp(0.0, 255.0) as u8)
}
pub(crate) fn scale3(c: [f32; 3], k: f32) -> [f32; 3] {
    [c[0] * k, c[1] * k, c[2] * k]
}

/// Everything that changes from frame to frame.
struct State {
    cam: Camera,
    /// Dawn amount, 0 = night, 1 = sunrise.
    d: f32,
    orb: Vec3,
    orb_s: f32,
    /// Ring size 0..1.
    rings: f32,
    spin: f32,
    glow: f32,
    speed: f32,
    streak: f32,
}

fn dawn(t: f32) -> f32 {
    if t < 17.0 {
        0.0
    } else if t < 26.5 {
        0.05 + (t - 17.0) / 9.5 * 0.55
    } else {
        0.75 + 0.25 * smooth((t - 26.5) / 3.5)
    }
}

fn state(shot: usize, t: f32, fq: usize) -> State {
    let (t0, t1, _) = SHOTS[shot];
    let u = (t - t0) / (t1 - t0);
    let orb0 = Vec3::new(0.0, 1.55, -17.0);
    let mut s = State { cam: Camera::look_at(Vec3::new(0.0, 1.8, 3.0), Vec3::new(0.0, 2.0, -12.0)), d: dawn(t), orb: orb0, orb_s: 0.5, rings: 0.0, spin: t * 2.0, glow: 0.15, speed: 0.0, streak: 0.0 };
    let shake = |amp: f32| Vec3::new(h(fq as u32 * 7 + 1) - 0.5, h(fq as u32 * 7 + 2) - 0.5, h(fq as u32 * 7 + 3) - 0.5) * 2.0 * amp;
    match shot {
        0 => {
            // Low dolly down the nave.
            let pos = Vec3::new((t * 0.9).sin() * 0.15, 0.7 + 0.9 * u, lerp(7.0, -7.0, u));
            s.cam = Camera::look_at(pos, Vec3::new(pos.x * 0.3, 1.9, -17.0));
            s.cam.fov_y = 62f32.to_radians();
            s.glow = 0.15 + 0.05 * (t * 3.0).sin();
        }
        1 => {
            // Tilt up past the columns and arches to the sky.
            let pos = Vec3::new(lerp(-2.2, -1.4, u), 0.35, -5.5);
            s.cam = Camera::look_at(pos, pos + Vec3::new(0.0, 0.0, -10.0));
            s.cam.yaw = -0.25 + 0.15 * u;
            s.cam.pitch = 0.1 + 1.05 * smooth(u);
            s.cam.fov_y = 72f32.to_radians();
            s.glow = 0.2;
        }
        2 => {
            // The orb wakes: it rises, pulses, rings open around it.
            let up = smooth(u);
            s.orb = orb0 + Vec3::new(0.0, 1.8 * up, 0.0);
            s.orb_s = 0.5 * (1.0 + 0.06 * (u * 40.0).sin());
            s.rings = smooth((u - 0.45) / 0.4);
            let pos = Vec3::new(0.0, 1.6 + 0.4 * u, lerp(-11.0, -12.8, u));
            s.cam = Camera::look_at(pos, Vec3::new(0.0, s.orb.y, -17.0));
            s.cam.fov_y = 55f32.to_radians();
            s.glow = 0.4 + 0.8 * u;
        }
        3 => {
            // Dive at the orb: lens stretch, shake, speed lines.
            let orb = orb0 + Vec3::new(0.0, 1.8, 0.0);
            s.orb = orb;
            s.orb_s = lerp(0.5, 0.75, u);
            s.rings = 1.0;
            s.spin = t * 9.0;
            let pos = Vec3::new(0.0, 3.3, lerp(-9.0, -14.6, ease_in(u) * 0.8 + u * 0.2)) + shake(0.03 + 0.1 * u);
            s.cam = Camera::look_at(pos, orb);
            s.cam.fov_y = lerp(58.0, 90.0, u * u).to_radians();
            s.glow = lerp(1.2, 2.2, u);
            s.speed = 0.55;
        }
        4 => {
            // Fast orbit around the awake orb, chapel behind.
            let orb = orb0 + Vec3::new(0.0, 1.8 + 0.9 * u, 0.0);
            s.orb = orb;
            s.orb_s = 0.75;
            s.rings = 1.0 + 0.1 * (t * 6.0).sin();
            s.spin = t * 5.0;
            let a = lerp(-1.0, 2.4, ease_out(u));
            let r = lerp(5.8, 4.2, u);
            let pos = orb + Vec3::new(a.sin() * r, lerp(-1.2, 1.4, u), a.cos() * r);
            s.cam = Camera::look_at(pos, orb);
            s.cam.fov_y = 62f32.to_radians();
            s.glow = 1.7;
        }
        5 => {
            // The orb leaves through the roof; the camera rises after it.
            let oy = 4.2 + 3.0 * u + 90.0 * u * u * u;
            let orb = Vec3::new(0.0, oy, -17.0 + 6.0 * u);
            s.orb = orb;
            s.orb_s = lerp(0.75, 0.5, u);
            s.rings = lerp(1.0, 0.35, smooth(u));
            s.spin = t * 12.0;
            let pos = Vec3::new(0.0, 1.6 + 22.0 * u * u, lerp(-8.0, -3.0, u)) + shake(0.06);
            let tgt = Vec3::new(0.0, 4.2 + (oy - 4.2) * 0.85, orb.z);
            s.cam = Camera::look_at(pos, tgt);
            s.cam.fov_y = 66f32.to_radians();
            s.glow = 1.8 + u;
            s.streak = smooth((u - 0.2) / 0.3);
        }
        _ => {
            // Dawn: the empty chapel from outside, sun coming up behind it.
            let pos = Vec3::new(0.0, 2.2, 15.0 - 3.0 * u);
            s.cam = Camera::look_at(pos, Vec3::new(0.0, 15.0, -30.0));
            s.cam.fov_y = 52f32.to_radians();
            s.orb = Vec3::new(0.0, -500.0, 0.0);
            s.glow = 0.0;
        }
    }
    s
}

/// World point (w=1) or direction (w=0) to cell units (x right, y down) and view depth.
pub(crate) fn project(cam: &Camera, p: Vec4) -> Option<(f32, f32, f32)> {
    let c = cam.proj(COLS, ROWS) * cam.view() * p;
    if c.w <= 0.01 {
        return None;
    }
    Some(((c.x / c.w * 0.5 + 0.5) * COLS as f32, (0.5 - c.y / c.w * 0.5) * ROWS as f32, c.w))
}

pub(crate) fn put(cells: &mut [Cell], x: i32, y: i32, ch: char, fg: [f32; 3], bold: bool) {
    if x >= 0 && y >= 0 && (x as usize) < COLS && (y as usize) < ROWS {
        cells[y as usize * COLS + x as usize] = Cell { ch, fg: rgb(fg), bold };
    }
}

/// Dither haze near the horizon, stars, and the moon or sun as a disc of block cells.
fn paint_sky(cells: &mut [Cell], st: &State, t: f32) {
    let cam = &st.cam;
    let d = smooth(st.d);
    let cool = [70.0, 82.0, 175.0];
    let warm = [255.0, 150.0, 90.0];
    for y in 0..ROWS {
        let e = cam.pitch + (0.5 - (y as f32 + 0.5) / ROWS as f32) * cam.fov_y;
        let dens = (0.07 + 0.5 * d) * (-((e + 0.03) / 0.4).powi(2)).exp();
        for x in 0..COLS {
            let hv = h((y * COLS + x) as u32 + 77);
            if hv < dens {
                put(cells, x as i32, y as i32, '.', scale3(mix3(cool, warm, d), 0.45 + 0.55 * h((y * COLS + x) as u32 + 991)), false);
            }
        }
    }
    let sv = 1.0 - clamp01(st.d * 2.2);
    if sv > 0.0 {
        for i in 0..300u32 {
            let az = h(i * 3) * 2.0 * PI;
            let el = 0.05 + h(i * 3 + 1) * 1.4;
            let dir = Vec4::new(az.sin() * el.cos(), el.sin(), -az.cos() * el.cos(), 0.0);
            let Some((px, py, _)) = project(cam, dir) else { continue };
            let tw = 0.55 + 0.45 * (t * (2.0 + 4.0 * h(i * 3 + 2)) + i as f32).sin();
            let b = (110.0 + 145.0 * h(i * 5)) * tw * sv;
            let ch = if h(i * 7) > 0.93 { '*' } else if h(i * 7) > 0.75 { '+' } else { '.' };
            put(cells, px as i32, py as i32, ch, [b, b, b * 1.1], false);
        }
    }
    let el = if t < 26.5 { 0.55 - 0.012 * t } else { -0.08 + 0.2 * smooth((t - 26.5) / 3.5) };
    let dir = Vec4::new(0.0, el.sin(), -el.cos(), 0.0);
    if let Some((px, py, _)) = project(cam, dir) {
        let rad = if t < 26.5 { 0.07 } else { 0.095 };
        let r = rad * ROWS as f32 * 2.0 / cam.fov_y;
        let col = mix3([205.0, 218.0, 255.0], [255.0, 205.0, 120.0], smooth((st.d - 0.2) / 0.5));
        let (x0, x1) = ((px - r - 2.0).floor() as i32, (px + r + 2.0).ceil() as i32);
        let (y0, y1) = ((py - r / 2.0 - 2.0).floor() as i32, (py + r / 2.0 + 2.0).ceil() as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                let dist = ((x as f32 + 0.5 - px).powi(2) + ((y as f32 + 0.5 - py) * 2.0).powi(2)).sqrt();
                if dist < r - 0.6 {
                    put(cells, x, y, '█', col, false);
                } else if dist < r + 0.5 {
                    put(cells, x, y, '▒', col, false);
                }
            }
        }
    }
}

/// Orb glow: a dithered cloud of dots that thickens toward the orb (fixed noise, so it does not shimmer).
fn paint_orb_glow(cells: &mut [Cell], st: &State) {
    if st.glow <= 0.0 {
        return;
    }
    let Some((px, py, w)) = project(&st.cam, st.orb.extend(1.0)) else { return };
    let focal = ROWS as f32 / (st.cam.fov_y * 0.5).tan();
    let r = (st.orb_s * 1.6 * focal / w).clamp(2.0, 400.0);
    let k = st.glow.min(2.0);
    let ext = r * 7.5;
    let (x0, x1) = ((px - ext).max(0.0) as usize, ((px + ext) as usize + 1).min(COLS));
    let (y0, y1) = ((py - ext / 2.0).max(0.0) as usize, ((py + ext / 2.0) as usize + 1).min(ROWS));
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = x as f32 + 0.5 - px;
            let dy = (y as f32 + 0.5 - py) * 2.0;
            let d2 = (dx * dx + dy * dy) / (r * r);
            let v = k * (0.22 * (-d2 / 6.25 * 1.2).exp() + 0.35 * (-d2 * 1.2).exp()) * 1.8;
            let hv = h((y * COLS + x) as u32 + 4242);
            if v > 0.6 || hv < v * 1.3 {
                let ch = if v > 0.9 { '@' } else if v > 0.6 { '*' } else if v > 0.35 { '+' } else if v > 0.2 { ':' } else { '.' };
                let c = mix3([255.0, 110.0, 30.0], [255.0, 225.0, 170.0], clamp01(v));
                put(cells, x as i32, y as i32, ch, scale3(c, 0.55 + 0.45 * clamp01(v)), false);
            }
        }
    }
}

pub(crate) fn line_char(dx: f32, dy: f32) -> char {
    let (ax, ay) = (dx.abs(), dy.abs());
    if ay < 0.38 * ax {
        '─'
    } else if ax < 0.38 * ay {
        '│'
    } else if dx * dy > 0.0 {
        '\\'
    } else {
        '/'
    }
}

/// Radial speed lines on blank cells, one glyph per cell along each ray.
pub(crate) fn speed_lines(cells: &mut [Cell], seed: u32, n: u32, inner: f32, col: [f32; 3], alpha: f32) {
    let (cx, cy) = (COLS as f32 / 2.0, ROWS as f32 / 2.0);
    for i in 0..n {
        let k = seed.wrapping_mul(131).wrapping_add(i);
        let ang = h(k) * 2.0 * PI;
        let (dx, dy) = (ang.cos(), ang.sin());
        let r0 = inner * ROWS as f32 * 2.0 * (0.75 + 0.7 * h(k + 9000));
        let r1 = COLS as f32 * 0.75;
        let ch = line_char(dx, dy);
        let mut r = r0;
        while r < r1 {
            let f = (r - r0) / (r1 - r0);
            let (x, y) = ((cx + dx * r).floor() as i32, (cy + dy * r / 2.0).floor() as i32);
            if x >= 0 && y >= 0 && (x as usize) < COLS && (y as usize) < ROWS && cells[y as usize * COLS + x as usize].ch == ' ' {
                put(cells, x, y, ch, scale3(col, alpha * (0.45 + 0.55 * f)), f > 0.5);
            }
            r += 0.5;
        }
    }
}

/// Vertical streaks falling past the camera while it rises.
fn streaks(cells: &mut [Cell], t: f32, amount: f32) {
    for i in 0..40u32 {
        let x = (h(i * 11) * COLS as f32) as i32;
        let len = 4.0 + 12.0 * h(i * 11 + 1);
        let sp = ROWS as f32 * (2.0 + 3.0 * h(i * 11 + 2)) * (0.4 + amount);
        let y0 = (h(i * 11 + 3) * ROWS as f32 + t * sp) % (ROWS as f32 + len) - len;
        for yy in 0..len as i32 {
            let f = yy as f32 / len;
            let y = y0 as i32 + yy;
            if y >= 0 && (y as usize) < ROWS && cells[y as usize * COLS + x as usize].ch == ' ' {
                put(cells, x, y, '│', scale3([255.0, 235.0, 200.0], amount * (1.0 - 0.7 * f)), false);
            }
        }
    }
}

const FONT_ORB: [[&str; 7]; 3] = [
    ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
    ["11110", "10001", "10001", "11110", "10100", "10010", "10001"],
    ["11110", "10001", "10001", "11110", "10001", "10001", "11110"],
];

/// Big block title, each dot is 3x2 cells of '█'. `shown` letters are visible (fractional = fading in).
fn draw_title(cells: &mut [Cell], shown: f32) {
    let (dw, dh) = (3i32, 2i32);
    let x0 = COLS as i32 / 2 - 17 * dw / 2;
    let y0 = (ROWS as f32 * 0.31) as i32 - 7 * dh / 2;
    for (li, letter) in FONT_ORB.iter().enumerate() {
        let a = clamp01(shown - li as f32);
        if a <= 0.0 {
            continue;
        }
        for (ry, row) in letter.iter().enumerate() {
            for (rx, c) in row.chars().enumerate() {
                if c != '1' {
                    continue;
                }
                for j in 0..dh {
                    for i in 0..dw {
                        put(cells, x0 + (li as i32 * 6 + rx as i32) * dw + i, y0 + ry as i32 * dh + j, '█', scale3([255.0, 240.0, 210.0], a), true);
                    }
                }
            }
        }
    }
}

fn draw_text(cells: &mut [Cell], text: &str, row: usize, col: [f32; 3], alpha: f32) {
    // letter-spaced, like a film subtitle
    let chars: Vec<char> = text.chars().flat_map(|c| [c, ' ']).collect();
    let n = chars.len() as i32 - 1;
    let x0 = COLS as i32 / 2 - n / 2;
    for (i, ch) in chars.into_iter().enumerate() {
        if ch != ' ' {
            put(cells, x0 + i as i32, row as i32, ch, scale3(col, alpha), false);
        }
    }
}

fn flash_amount(t: f32) -> f32 {
    if t < 25.6 {
        0.0
    } else if t < 26.5 {
        ((t - 25.6) / 0.9).powi(2)
    } else if t < 27.2 {
        1.0 - (t - 26.5) / 0.7
    } else {
        0.0
    }
}

fn fade_amount(t: f32) -> f32 {
    if t < 1.0 {
        1.0 - t
    } else if t > 29.3 {
        (t - 29.3) / 0.7
    } else {
        0.0
    }
}

fn build_scene() -> Scene {
    let mut s = Scene::chapel();
    for (i, major) in [1.0f32, 1.4, 1.9].iter().enumerate() {
        s.objects.push(Object { name: format!("ring{i}"), mesh: scene::torus(*major, 0.035, 40, 8), model: Mat4::from_translation(Vec3::new(0.0, -500.0, 0.0)), class: 1 });
    }
    s
}

fn place(scene: &mut Scene, st: &State) {
    let hidden = Mat4::from_translation(Vec3::new(0.0, -500.0, 0.0));
    let mut ring = 0;
    for o in scene.objects.iter_mut() {
        if o.name == "orb" {
            o.model = if st.orb.y < -100.0 { hidden } else { Mat4::from_translation(st.orb) * Mat4::from_scale(Vec3::splat(st.orb_s)) };
        } else if o.name.starts_with("ring") {
            let k = ring as f32;
            ring += 1;
            o.model = if st.rings < 0.02 || st.orb.y < -100.0 {
                hidden
            } else {
                let tilt = [(0.0, 0.0), (1.1, 0.5), (-0.8, 1.2)][ring - 1];
                Mat4::from_translation(st.orb)
                    * Mat4::from_rotation_y(tilt.0 + st.spin * (0.35 + 0.2 * k))
                    * Mat4::from_rotation_x(tilt.1 + st.spin * (0.5 - 0.15 * k) * if ring == 2 { -1.0 } else { 1.0 })
                    * Mat4::from_scale(Vec3::splat(st.rings.max(0.02)))
            };
        }
    }
}

/// Draw the whole film into cell frames (720 of them).
pub fn prerender(atlas: &Atlas, opts: &Options) -> Vec<FilmFrame> {
    let total = (SECS * FPS as f32) as usize;
    let mut frames = Vec::with_capacity(total);
    let mut scene = build_scene();
    let mut renderer = Renderer::default();
    let mut cur_shot = usize::MAX;
    let mut base: Vec<Cell> = vec![BLANK; COLS * ROWS];
    for f in 0..total {
        let t = f as f32 / FPS as f32;
        let shot = SHOTS.iter().rposition(|s| t >= s.0).unwrap_or(0);
        if shot != cur_shot {
            cur_shot = shot;
            scene = build_scene();
            renderer = Renderer::default();
        }
        let f0 = (SHOTS[shot].0 * FPS as f32).round() as usize;
        let step = SHOTS[shot].2;
        let fq = f0 + (f - f0) / step * step;
        let impact = IMPACT.contains(&f);
        if fq == f {
            let tq = fq as f32 / FPS as f32;
            let st = state(shot, tq, fq);
            place(&mut scene, &st);
            let frame = renderer.render(&scene, &st.cam, atlas, opts, COLS, ROWS);
            base = vec![BLANK; COLS * ROWS];
            if !impact {
                paint_sky(&mut base, &st, tq);
                paint_orb_glow(&mut base, &st);
            }
            for (b, c) in base.iter_mut().zip(&frame.cells) {
                if c.ch != ' ' {
                    *b = *c;
                }
            }
            if impact {
                speed_lines(&mut base, fq as u32, 110, 0.12, [255.0; 3], 1.0);
            } else {
                if st.speed > 0.0 {
                    speed_lines(&mut base, fq as u32, 55, 0.3 + 0.1 * h(fq as u32), [235.0, 230.0, 220.0], st.speed);
                }
                if st.streak > 0.0 {
                    streaks(&mut base, tq, st.streak);
                }
            }
        }
        frames.push(compose_frame(&base, t, impact));
    }
    frames
}

/// Letterbox, subtitles, title, paper (impact frames and the white-out) and fades on top of a drawing.
fn compose_frame(base: &[Cell], t: f32, impact: bool) -> FilmFrame {
    let mut cells = base.to_vec();
    cells[..BAR_ROWS * COLS].fill(BLANK);
    cells[(ROWS - BAR_ROWS) * COLS..].fill(BLANK);
    if t >= 27.6 {
        draw_title(&mut cells, (t - 27.6) / 0.28);
    }
    if (28.6..29.8).contains(&t) {
        draw_text(&mut cells, "an ascii short", (ROWS as f32 * 0.31) as usize + 9, [255.0, 240.0, 215.0], smooth((t - 28.6) / 0.3).min(smooth((29.8 - t) / 0.3)));
    }
    for &(a, b, txt) in SUBTITLES.iter() {
        if t >= a && t < b {
            draw_text(&mut cells, txt, ROWS - BAR_ROWS + 1, [235.0, 240.0, 255.0], smooth((t - a) / 0.3).min(smooth((b - t) / 0.3)));
        }
    }
    let p = if impact { 1.0 } else { flash_amount(t) };
    let fd = fade_amount(t);
    for c in cells.iter_mut() {
        if c.ch != ' ' {
            let col = mix3([c.fg.0 as f32, c.fg.1 as f32, c.fg.2 as f32], INK, p);
            c.fg = rgb(scale3(col, 1.0 - fd));
        }
    }
    FilmFrame { cells, bg: rgb(scale3(mix3([0.0; 3], PAPER, p), 1.0 - fd)) }
}

/// Draw one cell frame with the font into RGB (the same glyphs the terminal shows).
pub(crate) fn frame_rgb(f: &FilmFrame, atlas: &Atlas, rgb_out: &mut [u8]) {
    for px in rgb_out.as_chunks_mut::<3>().0 {
        *px = [f.bg.0, f.bg.1, f.bg.2];
    }
    for (i, c) in f.cells.iter().enumerate() {
        if c.ch == ' ' {
            continue;
        }
        let Some(g) = atlas.get(c.ch) else { continue };
        let (ox, oy) = ((i % COLS) * GW, (i / COLS) * GH);
        for y in 0..GH {
            for x in 0..GW {
                // Terminals draw block elements edge to edge; the font's own blocks leave gaps between rows.
                let cov = match c.ch {
                    '█' => 1.0,
                    '▒' => ((x + y) % 2 == 0) as u8 as f32,
                    _ => (g.mask[y * GW + x] * if c.bold { 1.25 } else { 1.0 }).min(1.0),
                };
                if cov > 0.0 {
                    let o = ((oy + y) * W + ox + x) * 3;
                    let (fg, bg) = ([c.fg.0, c.fg.1, c.fg.2], [f.bg.0, f.bg.1, f.bg.2]);
                    for k in 0..3 {
                        rgb_out[o + k] = (bg[k] as f32 + (fg[k] as f32 - bg[k] as f32) * cov) as u8;
                    }
                }
            }
        }
    }
}

fn text_atlas(font: &str) -> Result<Atlas, String> {
    let mut chars = glyphs::all_chars();
    chars.extend(' '..='~');
    chars.extend("█▒─│·".chars());
    chars.sort();
    chars.dedup();
    Atlas::load(font, &chars)
}

/// Write the film as an MP4 (needs ffmpeg), optionally with a WAV soundtrack.
pub fn write_video(frames: &[FilmFrame], font: &str, out: &str, audio: Option<&str>) -> Result<(), String> {
    let atlas = text_atlas(font)?;
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", &format!("{W}x{H}"), "-r", &FPS.to_string(), "-i", "-"]);
    if let Some(a) = audio {
        cmd.args(["-i", a, "-c:a", "aac", "-b:a", "192k", "-shortest"]);
    }
    cmd.args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "15", "-preset", "medium", out]);
    let mut ff = cmd.stdin(Stdio::piped()).spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    let mut pipe = ff.stdin.take().ok_or("no ffmpeg stdin")?;
    let mut buf = vec![0u8; W * H * 3];
    for f in frames {
        frame_rgb(f, &atlas, &mut buf);
        pipe.write_all(&buf).map_err(|e| format!("ffmpeg pipe: {e}"))?;
    }
    drop(pipe);
    let status = ff.wait().map_err(|e| e.to_string())?;
    eprintln!("film: {} frames -> {out} (ffmpeg {status})", frames.len());
    Ok(())
}

/// Write PNG stills of the frames at these times (seconds) into `dir`, for looking at the film without playing it.
pub fn write_stills(frames: &[FilmFrame], font: &str, dir: &str, times: &[f32]) -> Result<(), String> {
    let atlas = text_atlas(font)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; W * H * 3];
    for &t in times {
        let i = ((t * FPS as f32).round() as usize).min(frames.len() - 1);
        frame_rgb(&frames[i], &atlas, &mut buf);
        let path = format!("{dir}/t{:05.2}.png", t);
        let file = std::fs::File::create(&path).map_err(|e| format!("{path}: {e}"))?;
        let mut enc = png::Encoder::new(io::BufWriter::new(file), W as u32, H as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().map_err(|e| e.to_string())?.write_image_data(&buf).map_err(|e| e.to_string())?;
    }
    Ok(())
}

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), SetAttribute(Attribute::Reset), SetBackgroundColor(Color::Reset), cursor::Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

/// Play the film in the terminal in real time (frames are dropped if the terminal is slow). Needs a terminal of
/// at least COLS x ROWS cells. Sound: `afplay` (macOS) or `ffplay` on the WAV if given. Esc or x quits.
pub fn play(frames: &[FilmFrame], audio: Option<&str>) -> io::Result<()> {
    let (tc, tr) = terminal::size()?;
    if (tc as usize) < COLS || (tr as usize) < ROWS {
        return Err(io::Error::other(format!("terminal is {tc}x{tr}; the film needs {COLS}x{ROWS} cells (make the window bigger or the font smaller)")));
    }
    terminal::enable_raw_mode()?;
    let _guard = Guard;
    let mut out = io::BufWriter::with_capacity(1 << 20, io::stdout());
    execute!(out, EnterAlternateScreen, cursor::Hide)?;
    let mut snd = audio.and_then(|a| {
        let have_afplay = Command::new("afplay").arg("-h").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok();
        let (prog, args): (&str, Vec<&str>) = if have_afplay { ("afplay", vec![a]) } else { ("ffplay", vec!["-nodisp", "-autoexit", "-loglevel", "quiet", a]) };
        Command::new(prog).args(args).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()
    });
    let (ox, oy) = (((tc as usize - COLS) / 2) as u16, ((tr as usize - ROWS) / 2) as u16);
    let mut screen: Vec<Cell> = vec![Cell { ch: '\u{0}', ..BLANK }; COLS * ROWS];
    let mut cur_bg = (u8::MAX, 0, 0);
    let start = Instant::now();
    let mut last = usize::MAX;
    'play: loop {
        let idx = (start.elapsed().as_secs_f32() * FPS as f32) as usize;
        if idx >= frames.len() {
            break;
        }
        while event::poll(Duration::ZERO)? {
            if let Event::Key(k) = event::read()?
                && k.kind != KeyEventKind::Release
                && matches!(k.code, KeyCode::Esc | KeyCode::Char('x') | KeyCode::Char('q'))
            {
                break 'play;
            }
        }
        if idx == last {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        last = idx;
        let f = &frames[idx];
        if f.bg != cur_bg {
            cur_bg = f.bg;
            queue!(out, SetBackgroundColor(Color::Rgb { r: f.bg.0, g: f.bg.1, b: f.bg.2 }), terminal::Clear(terminal::ClearType::All))?;
            screen.fill(BLANK);
        }
        let mut cur: Option<(Color, bool)> = None;
        for (i, (n, o)) in f.cells.iter().zip(screen.iter()).enumerate() {
            if n == o || (n.ch == ' ' && o.ch == ' ') {
                continue;
            }
            queue!(out, cursor::MoveTo(ox + (i % COLS) as u16, oy + (i / COLS) as u16))?;
            let col = Color::Rgb { r: n.fg.0, g: n.fg.1, b: n.fg.2 };
            if cur != Some((col, n.bold)) {
                queue!(out, SetAttribute(if n.bold { Attribute::Bold } else { Attribute::NormalIntensity }), SetForegroundColor(col))?;
                cur = Some((col, n.bold));
            }
            queue!(out, Print(n.ch))?;
        }
        out.flush()?;
        screen.clone_from(&f.cells);
    }
    if let Some(c) = snd.as_mut() {
        let _ = c.kill();
    }
    Ok(())
}
