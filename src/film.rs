//! `--film OUT.mp4`: a 30 s anime-style short built on the chapel scene. The ASCII renderer draws the 3D shots;
//! this file adds what makes it read as anime: a painted sky behind the glyphs, drawing on twos, glow, speed
//! lines, impact frames, letterbox, subtitles and a title. Frames are piped to ffmpeg as raw RGB.
use crate::compose::Cell;
use crate::glyphs::{Atlas, GH, GW};
use crate::render::{Options, Renderer};
use crate::scene::{self, Camera, Object, Scene};
use glam::{Mat4, Vec3, Vec4};
use std::io::Write;
use std::process::{Command, Stdio};

const COLS: usize = 192;
const ROWS: usize = 54;
const W: usize = COLS * GW;
const H: usize = ROWS * GH;
const FPS: usize = 24;
const SECS: f32 = 30.0;
/// Letterbox bar height in pixels.
const BAR: usize = 96;
const PI: f32 = std::f32::consts::PI;

/// (start s, end s, frames per drawing). Most shots are drawn on twos (12 drawings a second), like cel anime.
const SHOTS: [(f32, f32, usize); 7] = [(0.0, 5.0, 2), (5.0, 9.0, 2), (9.0, 13.0, 2), (13.0, 17.0, 1), (17.0, 21.5, 1), (21.5, 26.5, 1), (26.5, 30.0, 2)];
/// Frames drawn as a black-on-white impact frame.
const IMPACT: [usize; 4] = [408, 409, 516, 517];

const SUBTITLES: [(f32, f32, &str); 6] = [
    (1.0, 4.3, "before dawn."),
    (5.6, 8.6, "something waits at the altar."),
    (9.6, 12.6, "it has slept a long time."),
    (13.4, 16.6, "it is waking."),
    (21.9, 24.6, "go."),
    (28.6, 29.6, "an ascii short"),
];

fn h(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x & 0xFF_FFFF) as f32 / 16_777_216.0
}
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}
fn smooth(x: f32) -> f32 {
    let x = clamp01(x);
    x * x * (3.0 - 2.0 * x)
}
fn ease_out(x: f32) -> f32 {
    1.0 - (1.0 - clamp01(x)).powi(3)
}
fn ease_in(x: f32) -> f32 {
    clamp01(x).powi(3)
}
fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]
}

/// Everything that changes from frame to frame.
struct State {
    shot: usize,
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
    let mut s = State { shot, cam: Camera::look_at(Vec3::new(0.0, 1.8, 3.0), Vec3::new(0.0, 2.0, -12.0)), d: dawn(t), orb: orb0, orb_s: 0.5, rings: 0.0, spin: t * 2.0, glow: 0.15, speed: 0.0, streak: 0.0 };
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
            s.cam = Camera::look_at(pos, Vec3::new(0.0, 8.5, -30.0));
            s.cam.fov_y = 52f32.to_radians();
            s.orb = Vec3::new(0.0, -500.0, 0.0);
            s.glow = 0.0;
        }
    }
    s
}

fn project(cam: &Camera, p: Vec4) -> Option<(f32, f32, f32)> {
    let c = cam.proj(COLS, ROWS) * cam.view() * p;
    if c.w <= 0.01 {
        return None;
    }
    Some(((c.x / c.w * 0.5 + 0.5) * W as f32, (0.5 - c.y / c.w * 0.5) * H as f32, c.w))
}

type Img = Vec<f32>;

fn blend(img: &mut [f32], x: i32, y: i32, col: [f32; 3], a: f32) {
    if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 {
        return;
    }
    let o = (y as usize * W + x as usize) * 3;
    for k in 0..3 {
        img[o + k] += (col[k] - img[o + k]) * a;
    }
}

fn add_glow(img: &mut [f32], cx: f32, cy: f32, r: f32, col: [f32; 3], k: f32) {
    let ext = r * 3.0;
    let (x0, x1) = ((cx - ext).max(0.0) as usize, ((cx + ext) as usize + 1).min(W));
    let (y0, y1) = ((cy - ext).max(0.0) as usize, ((cy + ext) as usize + 1).min(H));
    for y in y0..y1 {
        for x in x0..x1 {
            let d2 = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)) / (r * r);
            if d2 > 9.0 {
                continue;
            }
            let f = k * (-d2 * 1.2).exp();
            let o = (y * W + x) * 3;
            for c in 0..3 {
                img[o + c] += col[c] * f;
            }
        }
    }
}

fn paint_sky(img: &mut Img, st: &State, t: f32) {
    let cam = &st.cam;
    let d = smooth(st.d);
    let top = mix3([3.0, 4.0, 18.0], [30.0, 36.0, 105.0], d);
    let hor = mix3([24.0, 26.0, 74.0], [255.0, 130.0, 72.0], d);
    for y in 0..H {
        let e = cam.pitch + (0.5 - (y as f32 + 0.5) / H as f32) * cam.fov_y;
        let s = clamp01((e + 0.12) / 1.1).powf(0.7);
        let mut c = mix3(hor, top, s);
        if e < -0.12 {
            let k = (1.0 - ((-0.12 - e) * 2.5)).max(0.35);
            c = [c[0] * k, c[1] * k, c[2] * k];
        }
        for x in 0..W {
            let o = (y * W + x) * 3;
            img[o..o + 3].copy_from_slice(&c);
        }
    }
    // Stars, fading out as dawn comes.
    let sv = 1.0 - clamp01(st.d * 2.2);
    if sv > 0.0 {
        for i in 0..300u32 {
            let az = h(i * 3) * 2.0 * PI;
            let el = 0.05 + h(i * 3 + 1) * 1.4;
            let dir = Vec4::new(az.sin() * el.cos(), el.sin(), -az.cos() * el.cos(), 0.0);
            let Some((px, py, _)) = project(cam, dir) else { continue };
            let tw = 0.55 + 0.45 * (t * (2.0 + 4.0 * h(i * 3 + 2)) + i as f32).sin();
            let b = (90.0 + 160.0 * h(i * 5)) * tw * sv;
            let big = h(i * 7) > 0.85;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                if big || (dx == 0 && dy == 0) {
                    blend(img, px as i32 + dx, py as i32 + dy, [b, b, b * 1.1], 0.9);
                }
            }
        }
    }
    // Moon, then sun.
    let el = if t < 26.5 { 0.55 - 0.012 * t } else { -0.08 + 0.2 * smooth((t - 26.5) / 3.5) };
    let dir = Vec4::new(0.0, el.sin(), -el.cos(), 0.0);
    if let Some((px, py, _)) = project(cam, dir) {
        let rad = if t < 26.5 { 0.05 } else { 0.095 };
        let r = rad * H as f32 / cam.fov_y;
        let col = mix3([205.0, 218.0, 255.0], [255.0, 205.0, 120.0], smooth((st.d - 0.2) / 0.5));
        add_glow(img, px, py, r * 2.4, [col[0] * 0.5, col[1] * 0.5, col[2] * 0.5], 0.35);
        let (x0, x1) = ((px - r - 2.0).max(0.0) as usize, ((px + r + 2.0) as usize).min(W));
        let (y0, y1) = ((py - r - 2.0).max(0.0) as usize, ((py + r + 2.0) as usize).min(H));
        for y in y0..y1 {
            for x in x0..x1 {
                let dist = ((x as f32 - px).powi(2) + (y as f32 - py).powi(2)).sqrt();
                let a = clamp01((r - dist) / 1.5 + 0.5);
                if a > 0.0 {
                    blend(img, x as i32, y as i32, col, a);
                }
            }
        }
    }
}

fn paint_orb_glow(img: &mut Img, st: &State) {
    if st.glow <= 0.0 {
        return;
    }
    let Some((px, py, w)) = project(&st.cam, st.orb.extend(1.0)) else { return };
    let focal = H as f32 / (2.0 * (st.cam.fov_y * 0.5).tan());
    let r = (st.orb_s * 1.6 * focal / w).clamp(6.0, 900.0);
    add_glow(img, px, py, r * 2.5, [255.0, 120.0, 30.0], 0.22 * st.glow.min(2.0));
    add_glow(img, px, py, r, [255.0, 200.0, 120.0], 0.35 * st.glow.min(2.0));
}

/// Draw the cell grid over the background with the atlas masks. `ink` replaces the colours of impact frames.
fn blit_cells(img: &mut Img, cells: &[Cell], atlas: &Atlas, ink: Option<[f32; 3]>) {
    for (i, c) in cells.iter().enumerate() {
        if c.ch == ' ' {
            continue;
        }
        let Some(g) = atlas.get(c.ch) else { continue };
        let (ox, oy) = ((i % COLS) * GW, (i / COLS) * GH);
        let col = ink.unwrap_or([c.fg.0 as f32, c.fg.1 as f32, c.fg.2 as f32]);
        for y in 0..GH {
            for x in 0..GW {
                let cov = (g.mask[y * GW + x] * if c.bold { 1.25 } else { 1.0 }).min(1.0);
                if cov > 0.0 {
                    blend(img, (ox + x) as i32, (oy + y) as i32, col, cov);
                }
            }
        }
    }
}

fn speed_lines(img: &mut Img, seed: u32, n: u32, inner: f32, col: [f32; 3], alpha: f32) {
    let (cx, cy) = (W as f32 / 2.0, H as f32 / 2.0);
    for i in 0..n {
        let k = seed.wrapping_mul(131).wrapping_add(i);
        let ang = h(k) * 2.0 * PI;
        let r0 = inner * H as f32 * (0.75 + 0.7 * h(k + 9000));
        let r1 = W as f32 * 0.75;
        let (dx, dy) = (ang.cos(), ang.sin());
        let wmax = 1.5 + 5.0 * h(k + 4000);
        let mut r = r0;
        while r < r1 {
            let f = (r - r0) / (r1 - r0);
            let hw = 0.4 + wmax * f;
            let (x, y) = (cx + dx * r, cy + dy * r * 0.9);
            let m = hw.ceil() as i32;
            for j in -m..=m {
                let a = clamp01(hw - j.abs() as f32 + 0.5) * alpha * clamp01(f * 4.0);
                if a > 0.0 {
                    blend(img, (x - dy * j as f32) as i32, (y + dx * j as f32) as i32, col, a);
                }
            }
            r += 1.0;
        }
    }
}

fn streaks(img: &mut Img, t: f32, amount: f32) {
    for i in 0..44u32 {
        let x = (h(i * 11) * W as f32) as i32;
        let len = 90.0 + 260.0 * h(i * 11 + 1);
        let sp = H as f32 * (2.0 + 3.0 * h(i * 11 + 2)) * (0.4 + amount);
        let y0 = (h(i * 11 + 3) * H as f32 + t * sp) % (H as f32 + len) - len;
        let wd = 1 + (h(i * 11 + 4) * 2.0) as i32;
        for yy in 0..len as i32 {
            let f = yy as f32 / len;
            let a = amount * 0.7 * (1.0 - f) * clamp01(f * 6.0 + 0.1);
            for dx in 0..wd {
                blend(img, x + dx, y0 as i32 + yy, [255.0, 235.0, 200.0], a);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text(img: &mut Img, atlas: &Atlas, text: &str, cx: f32, y: f32, scale: usize, col: [f32; 3], alpha: f32) {
    let adv = GW * scale + 3 * scale;
    let total = adv * text.chars().count() - 3 * scale;
    let x0 = cx as i32 - total as i32 / 2;
    for (n, ch) in text.chars().enumerate() {
        let Some(g) = atlas.get(ch) else { continue };
        for py in 0..GH * scale {
            for px in 0..GW * scale {
                // bilinear sample of the mask
                let fx = ((px as f32 + 0.5) / scale as f32 - 0.5).clamp(0.0, GW as f32 - 1.0);
                let fy = ((py as f32 + 0.5) / scale as f32 - 0.5).clamp(0.0, GH as f32 - 1.0);
                let (ix, iy) = (fx as usize, fy as usize);
                let (jx, jy) = ((ix + 1).min(GW - 1), (iy + 1).min(GH - 1));
                let (ax, ay) = (fx - ix as f32, fy - iy as f32);
                let m = |x: usize, y: usize| g.mask[y * GW + x];
                let v = lerp(lerp(m(ix, iy), m(jx, iy), ax), lerp(m(ix, jy), m(jx, jy), ax), ay);
                if v > 0.0 {
                    blend(img, x0 + (n * adv + px) as i32, y as i32 + py as i32, col, (v * 1.3).min(1.0) * alpha);
                }
            }
        }
    }
}

const FONT_ORB: [[&str; 7]; 3] = [
    ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
    ["11110", "10001", "10001", "11110", "10100", "10010", "10001"],
    ["11110", "10001", "10001", "11110", "10001", "10001", "11110"],
];

/// Big pixel-block title. `shown` is how many letters are visible (fractional = fading in).
fn draw_title(img: &mut Img, shown: f32) {
    let dot = 22i32;
    let total_w = 17 * dot;
    let (x0, y0) = (W as i32 / 2 - total_w / 2, H as i32 * 36 / 100 - 7 * dot / 2);
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
                let bx = x0 + (li as i32 * 6 + rx as i32) * dot;
                let by = y0 + ry as i32 * dot;
                for y in -3..dot + 3 {
                    for x in -3..dot + 3 {
                        let inside = x >= 2 && x < dot - 2 && y >= 2 && y < dot - 2;
                        if inside {
                            blend(img, bx + x, by + y, [255.0, 240.0, 210.0], a);
                        } else {
                            blend(img, bx + x, by + y, [20.0, 10.0, 30.0], 0.35 * a);
                        }
                    }
                }
            }
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

pub fn run(out: &str, atlas: &Atlas, font: &str, opts: &Options) -> Result<(), String> {
    let text_chars: Vec<char> = (' '..='~').collect();
    let tatlas = Atlas::load(font, &text_chars)?;
    let mut ff = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", &format!("{W}x{H}"), "-r", &FPS.to_string(), "-i", "-"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "15", "-preset", "medium", out])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let mut pipe = ff.stdin.take().ok_or("no ffmpeg stdin")?;

    let total = (SECS * FPS as f32) as usize;
    let mut scene = build_scene();
    let mut renderer = Renderer::default();
    let mut cur_shot = usize::MAX;
    let mut cache: Img = vec![0.0; W * H * 3];
    let mut bytes = vec![0u8; W * H * 3];
    let t_start = std::time::Instant::now();
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
        if fq == f {
            let tq = fq as f32 / FPS as f32;
            let st = state(shot, tq, fq);
            place(&mut scene, &st);
            let frame = renderer.render(&scene, &st.cam, atlas, opts, COLS, ROWS);
            let impact = IMPACT.contains(&f);
            if impact {
                cache.iter_mut().for_each(|v| *v = 242.0);
                speed_lines(&mut cache, fq as u32, 110, 0.12, [10.0, 8.0, 16.0], 1.0);
                blit_cells(&mut cache, &frame.cells, atlas, Some([10.0, 8.0, 16.0]));
            } else {
                paint_sky(&mut cache, &st, tq);
                paint_orb_glow(&mut cache, &st);
                blit_cells(&mut cache, &frame.cells, atlas, None);
                if st.speed > 0.0 {
                    speed_lines(&mut cache, fq as u32, 55, 0.3 + 0.1 * h(fq as u32), [255.0, 240.0, 220.0], st.speed);
                }
                if st.streak > 0.0 {
                    streaks(&mut cache, tq, st.streak);
                }
            }
        }
        // Per-frame overlay on a copy of the cached drawing.
        let mut img = cache.clone();
        let fl = flash_amount(t);
        if fl > 0.0 {
            img.iter_mut().for_each(|v| *v += (255.0 - *v) * fl);
        }
        for y in (0..BAR).chain(H - BAR..H) {
            img[y * W * 3..(y + 1) * W * 3].fill(0.0);
        }
        if t >= 27.6 {
            draw_title(&mut img, (t - 27.6) / 0.28);
        }
        for &(a, b, txt) in SUBTITLES.iter() {
            if t >= a && t < b {
                let al = smooth((t - a) / 0.3).min(smooth((b - t) / 0.3));
                if txt == "an ascii short" {
                    draw_text(&mut img, &tatlas, txt, W as f32 / 2.0, H as f32 * 0.36 + 100.0, 2, [255.0, 240.0, 215.0], al);
                } else {
                    draw_text(&mut img, &tatlas, txt, W as f32 / 2.0, (H - BAR + 32) as f32, 2, [235.0, 240.0, 255.0], al);
                }
            }
        }
        let fd = fade_amount(t);
        for (b, v) in bytes.iter_mut().zip(&img) {
            *b = (v * (1.0 - fd)).clamp(0.0, 255.0) as u8;
        }
        pipe.write_all(&bytes).map_err(|e| format!("ffmpeg pipe: {e}"))?;
    }
    drop(pipe);
    let status = ff.wait().map_err(|e| e.to_string())?;
    eprintln!("film: {total} frames in {:.1}s -> {out} (ffmpeg {status})", t_start.elapsed().as_secs_f32());
    Ok(())
}
