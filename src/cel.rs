//! 2D cell art for the PV: block-font kinetic type, eye close-ups, slash/split, embers, moon, neon grid.
use crate::compose::{BLANK, Cell};
use crate::film::{COLS, PI, ROWS, clamp01, ease_out, h, line_char, lerp, mix3, project, put, scale3, smooth};
use crate::scene::Camera;
use glam::{Vec3, Vec4};

pub type Col = [f32; 3];
pub const CYAN: Col = [0.0, 230.0, 255.0];
pub const ORANGE: Col = [255.0, 140.0, 0.0];
pub const WHITE: Col = [255.0, 250.0, 235.0];

fn glyph5x7(c: char) -> [&'static str; 7] {
    match c {
        'A' => ["01110", "10001", "10001", "11111", "10001", "10001", "10001"],
        'B' => ["11110", "10001", "10001", "11110", "10001", "10001", "11110"],
        'C' => ["01110", "10001", "10000", "10000", "10000", "10001", "01110"],
        'D' => ["11110", "10001", "10001", "10001", "10001", "10001", "11110"],
        'E' => ["11111", "10000", "10000", "11110", "10000", "10000", "11111"],
        'G' => ["01110", "10001", "10000", "10111", "10001", "10001", "01111"],
        'I' => ["01110", "00100", "00100", "00100", "00100", "00100", "01110"],
        'L' => ["10000", "10000", "10000", "10000", "10000", "10000", "11111"],
        'M' => ["10001", "11011", "10101", "10101", "10001", "10001", "10001"],
        'N' => ["10001", "11001", "10101", "10011", "10001", "10001", "10001"],
        'O' => ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
        'R' => ["11110", "10001", "10001", "11110", "10100", "10010", "10001"],
        'S' => ["01111", "10000", "10000", "01110", "00001", "00001", "11110"],
        'T' => ["11111", "00100", "00100", "00100", "00100", "00100", "00100"],
        'U' => ["10001", "10001", "10001", "10001", "10001", "10001", "01110"],
        'W' => ["10001", "10001", "10001", "10101", "10101", "11011", "10001"],
        '.' => ["00000", "00000", "00000", "00000", "00000", "00000", "00100"],
        _ => ["00000"; 7],
    }
}

/// A line of big block type. Letters slam in one after another with a white flash and a short glitch.
pub struct Txt {
    pub t0: f32,
    pub t1: f32,
    pub text: &'static str,
    /// Top row; the text is centred on the screen.
    pub y: i32,
    pub dw: i32,
    pub dh: i32,
    pub col: Col,
}

pub fn draw_txt(cells: &mut [Cell], tx: &Txt, t: f32) {
    if t < tx.t0 || t >= tx.t1 {
        return;
    }
    let adv = |c: char| if c == ' ' { 3 } else { 6 };
    let total: i32 = tx.text.chars().map(adv).sum::<i32>() - 1;
    let mut x = COLS as i32 / 2 - total * tx.dw / 2;
    let out = smooth(((tx.t1 - t) / 0.2).min(1.0));
    for (i, ch) in tx.text.chars().enumerate() {
        let ti = t - tx.t0 - i as f32 * 0.045;
        if ti >= 0.0 && ch != ' ' {
            let drop = ((1.0 - ease_out(clamp01(ti / 0.12))) * -7.0) as i32;
            let flash = clamp01(1.0 - ti / 0.14);
            let col = mix3(tx.col, WHITE, flash);
            let g = glyph5x7(ch);
            for (ry, row) in g.iter().enumerate() {
                let glitch = if ti < 0.25 && h((ry * 31 + i * 7) as u32 + (ti * 24.0) as u32 * 101) < 0.35 { ((h(ry as u32 + i as u32 * 13 + (ti * 24.0) as u32 * 7) - 0.5) * 6.0) as i32 } else { 0 };
                for (rx, c) in row.chars().enumerate() {
                    if c != '1' {
                        continue;
                    }
                    for j in 0..tx.dh {
                        for k in 0..tx.dw {
                            put(cells, x + rx as i32 * tx.dw + k + glitch, tx.y + ry as i32 * tx.dh + j + drop, '█', scale3(col, out), true);
                        }
                    }
                }
            }
        }
        x += adv(ch) * tx.dw;
    }
}

/// Small letter-spaced line of normal text.
pub fn small_text(cells: &mut [Cell], text: &str, row: i32, col: Col, alpha: f32) {
    let chars: Vec<char> = text.chars().flat_map(|c| [c, ' ']).collect();
    let n = chars.len() as i32 - 1;
    let x0 = COLS as i32 / 2 - n / 2;
    for (i, ch) in chars.into_iter().enumerate() {
        if ch != ' ' {
            put(cells, x0 + i as i32, row, ch, scale3(col, alpha), false);
        }
    }
}

pub struct Eye {
    pub cx: f32,
    pub cy: f32,
    /// Half width in cells.
    pub w: f32,
    /// Lid height in cell-pixels (1 cell = 1 wide, 2 high).
    pub hgt: f32,
    pub open: f32,
    /// Iris offset, -1..1.
    pub look: (f32, f32),
    pub col: Col,
    /// Mirror: the outer corner is on the right.
    pub flip: bool,
    /// Brow tilt: positive = angry (inner end low).
    pub brow: f32,
    /// Iris size scale (small = intense).
    pub iris: f32,
    pub glint: f32,
}

/// Draw an anime eye: heavy upper lid, lash flick, iris with ring, pupil, glints, brow.
pub fn draw_eye(cells: &mut [Cell], e: &Eye) {
    let s = if e.flip { -1.0 } else { 1.0 };
    let lid = |u: f32| -> (f32, f32) {
        // u in -1..1 along the eye (outer corner at u = -1). Corner tilt lifts the outer end.
        let shape = (1.0 - u * u).max(0.0).powf(0.9);
        let tilt = -u * e.hgt * 0.16;
        (tilt - e.open * e.hgt * shape * if u < 0.0 { 1.0 } else { 0.85 }, tilt + e.open * e.hgt * 0.7 * shape)
    };
    let (ix, iy) = (e.look.0 * e.w * 0.28, e.look.1 * e.hgt * 0.25);
    let r = e.hgt * 0.95 * e.iris;
    for cyi in 0..ROWS {
        for cxi in 0..COLS {
            let px = (cxi as f32 + 0.5 - e.cx) * s;
            let py = (cyi as f32 + 0.5 - e.cy) * 2.0;
            let u = px / e.w;
            let mut out: Option<(char, Col, bool)> = None;
            if u.abs() <= 1.0 {
                let (up, lo) = lid(u);
                let inside = py > up && py < lo;
                let slope = {
                    let (u2, _) = (u + 0.02, 0.0);
                    (lid(u2).0 - up) / (0.02 * e.w)
                };
                if inside && e.open > 0.05 {
                    let (dx, dy) = (px - ix, py - iy);
                    let d = (dx * dx + dy * dy).sqrt() / r;
                    if d < 1.0 {
                        let shade = if py < up + 6.0 { 0.45 } else { 1.0 };
                        let glint = |gx: f32, gy: f32, gr: f32| ((px - ix - gx * r).powi(2) + (py - iy - gy * r).powi(2)).sqrt() < gr * r;
                        if (glint(-0.38, -0.36, 0.26) || glint(0.3, 0.38, 0.12)) && e.glint > 0.0 {
                            out = Some(('█', scale3(WHITE, e.glint), true));
                        } else if d < 0.36 {
                            out = None;
                        } else if d < 0.72 {
                            out = Some(('█', scale3(e.col, shade), true));
                        } else if h((cyi * COLS + cxi) as u32 + 5) < 0.55 {
                            out = Some(('█', scale3(e.col, 0.55 * shade), false));
                        } else {
                            out = Some(('▒', scale3(e.col, 0.8 * shade), false));
                        }
                    }
                }
                if out.is_none() || out.is_some_and(|o| o.0 == '▒') {
                    if (py - up).abs() < 1.8 + 3.2 * (1.0 - u).clamp(0.0, 1.0) * (1.0 - u.abs() * 0.3) {
                        out = Some(('█', scale3(e.col, 1.0), true));
                    } else if (py - lo).abs() < 1.1 && e.open > 0.05 {
                        out = Some((line_char(1.0, -slope * 0.5), scale3(e.col, 0.8), false));
                    }
                }
                if e.open <= 0.05 && (py - (up + lo) * 0.5).abs() < 1.6 {
                    out = Some(('█', e.col, true));
                }
            } else if u.abs() < 1.14 && u * s.signum() * s < 0.0 {
                // lash flick at the outer corner
                let k = (u.abs() - 1.0) / 0.14;
                let (up, _) = lid(-1.0);
                if (py - (up - k * e.hgt * 0.32)).abs() < 2.2 {
                    out = Some(('█', e.col, true));
                }
            }
            // brow
            let bu = (px / e.w).clamp(-1.0, 1.0);
            let by = -e.hgt * 1.25 - 5.0 + e.brow * bu * -16.0;
            if px.abs() < e.w * 0.8 && (py - by).abs() < 1.2 + 1.6 * (1.0 - bu.abs()) && out.is_none() {
                out = Some(('█', scale3(e.col, 0.85), true));
            }
            if let Some((ch, col, bold)) = out {
                put(cells, cxi as i32, cyi as i32, ch, col, bold);
            }
        }
    }
}

/// Glint streaks that flash on the iris at a beat.
pub fn eye_shine(cells: &mut [Cell], e: &Eye, k: f32) {
    if k <= 0.0 {
        return;
    }
    let n = (e.w * 1.4 * k) as i32;
    let y = (e.cy + e.hgt * 0.1 / 2.0) as i32;
    for i in 0..n {
        put(cells, e.cx as i32 - n / 2 + i, y, '─', scale3(WHITE, k), true);
    }
    let m = (4.0 + 5.0 * k) as i32;
    for j in -m..=m {
        put(cells, (e.cx + e.look.0 * e.w * 0.28 - e.w * 0.1) as i32, y + j, '│', scale3(WHITE, k * (1.0 - j.abs() as f32 / (m as f32 + 1.0))), true);
    }
}

/// Embers or sparks drifting up (quantised time keeps them on twos with the drawing).
pub fn embers(cells: &mut [Cell], t: f32, n: u32, a: Col, b: Col, up: f32) {
    for i in 0..n {
        let sp = 3.0 + 6.0 * h(i * 3 + 1);
        let ph = h(i * 3 + 2) * 100.0;
        let y = (ROWS as f32 + 2.0) - ((t * sp * up + ph * ROWS as f32) % (ROWS as f32 + 4.0));
        let x = h(i * 3) * COLS as f32 + (t * (0.6 + h(i + 500)) + ph).sin() * 3.0;
        let (xi, yi) = (x as i32, y as i32);
        if xi >= 0 && yi >= 0 && (xi as usize) < COLS && (yi as usize) < ROWS && cells[yi as usize * COLS + xi as usize].ch == ' ' {
            let ch = ['.', ',', ':', '*'][(h(i + 900) * 4.0) as usize % 4];
            put(cells, xi, yi, ch, scale3(mix3(a, b, h(i + 77)), 0.5 + 0.5 * h(i + 31)), false);
        }
    }
}

/// Moon ring and a dithered halo, in world direction `dir`.
pub fn moon(cells: &mut [Cell], cam: &Camera, dir: Vec3, rad: f32, col: Col) {
    let Some((mx, my, _)) = project(cam, Vec4::new(dir.x, dir.y, dir.z, 0.0)) else { return };
    let r = rad * 60f32.to_radians() / cam.fov_y;
    for y in 0..ROWS {
        for x in 0..COLS {
            let (dx, dy) = (x as f32 + 0.5 - mx, (y as f32 + 0.5 - my) * 2.0);
            let d = (dx * dx + dy * dy).sqrt();
            if (d - r).abs() < 1.1 {
                put(cells, x as i32, y as i32, line_char(-dy, dx), col, true);
            } else if d < r - 1.0 && h((y * COLS + x) as u32 + 3) < 0.10 {
                put(cells, x as i32, y as i32, '.', scale3(col, 0.5), false);
            } else if d > r && d < r * 1.7 && h((y * COLS + x) as u32 + 8) < 0.06 * (1.0 - (d - r) / (r * 0.7)).max(0.0) * 3.0 {
                put(cells, x as i32, y as i32, '.', scale3(col, 0.35), false);
            }
        }
    }
}

/// Neon floor grid drawn straight into cells (clean one-cell lines that follow the perspective).
pub fn grid(cells: &mut [Cell], cam: &Camera, col: Col, glow: f32) {
    let seg = |cells: &mut [Cell], a: Vec3, b: Vec3, c: Col| {
        let n = 90;
        let mut last: Option<(i32, i32)> = None;
        for i in 0..=n {
            let p = a.lerp(b, i as f32 / n as f32);
            let Some((x, y, w)) = project(cam, Vec4::new(p.x, p.y, p.z, 1.0)) else { continue };
            let (xi, yi) = (x.floor() as i32, y.floor() as i32);
            if last == Some((xi, yi)) {
                continue;
            }
            let Some((x2, y2, _)) = project(cam, Vec4::new(p.x + (b.x - a.x) * 0.01, p.y, p.z + (b.z - a.z) * 0.01, 1.0)) else { continue };
            let fade = clamp01(1.4 - w / 70.0);
            if xi >= 0 && yi >= 0 && (xi as usize) < COLS && (yi as usize) < ROWS && cells[yi as usize * COLS + xi as usize].ch == ' ' {
                put(cells, xi, yi, line_char(x2 - x, (y2 - y) * 2.0), scale3(c, fade), false);
            }
            last = Some((xi, yi));
        }
    };
    for i in -15..=15 {
        let bright = if i % 5 == 0 { 1.0 } else { 0.5 };
        let c = scale3(col, bright * (0.55 + 0.45 * glow));
        seg(cells, Vec3::new(i as f32 * 2.0, 0.0, -50.0), Vec3::new(i as f32 * 2.0, 0.0, 30.0), c);
    }
    for k in -25..=15 {
        let c = scale3(col, if k % 5 == 0 { 1.0 } else { 0.5 } * (0.55 + 0.45 * glow));
        seg(cells, Vec3::new(-30.0, 0.0, k as f32 * 2.0), Vec3::new(30.0, 0.0, k as f32 * 2.0), c);
    }
}

/// Cut the picture along a slanted line through the middle and slide the halves apart.
pub fn split(cells: &[Cell], slope: f32, shift: f32) -> Vec<Cell> {
    let mut out = vec![BLANK; cells.len()];
    let (cx, cy) = (COLS as f32 / 2.0, ROWS as f32 / 2.0);
    let sh = shift.round() as i32;
    for y in 0..ROWS {
        for x in 0..COLS {
            let c = cells[y * COLS + x];
            if c.ch == ' ' {
                continue;
            }
            let above = (y as f32 + 0.5) < cy + slope * (x as f32 + 0.5 - cx) / 2.0;
            let nx = x as i32 + if above { -sh } else { sh };
            let ny = y as i32 + if above { -(sh / 4) } else { sh / 4 };
            if nx >= 0 && ny >= 0 && (nx as usize) < COLS && (ny as usize) < ROWS {
                out[ny as usize * COLS + nx as usize] = c;
            }
        }
    }
    out
}

/// A bright slash drawn from one end of the line through the middle; `k` 0..1 is how far it has travelled.
pub fn slash(cells: &mut [Cell], slope: f32, k: f32, col: Col) {
    let (cx, cy) = (COLS as f32 / 2.0, ROWS as f32 / 2.0);
    let half = COLS as f32 / 2.0;
    let n = (2.0 * half * k) as i32;
    let ch = line_char(1.0, slope);
    for i in 0..n {
        let x = cx - half + i as f32;
        let y = cy + slope * (x - cx) / 2.0;
        let f = 1.0 - (n - i) as f32 / (n as f32 + 1.0) * 0.0;
        put(cells, x as i32, y as i32, ch, scale3(col, f), true);
        put(cells, x as i32 + 1, y as i32, ch, scale3(col, f), true);
        if (i % 3) == 0 {
            put(cells, x as i32, y as i32 - 1, '.', scale3(col, 0.5), false);
            put(cells, x as i32, y as i32 + 1, '.', scale3(col, 0.5), false);
        }
    }
}

/// Whiten everything: colours go to white by `k`.
pub fn whiten(c: Col, k: f32) -> Col {
    mix3(c, [255.0, 255.0, 255.0], k)
}

pub fn lerp_pt(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (lerp(a.0, b.0, t), lerp(a.1, b.1, t))
}

pub const TAU: f32 = 2.0 * PI;
