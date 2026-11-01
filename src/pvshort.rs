//! ONE CUT, the 10 s social short (`--story pvshort`). Its own shot list, not a trim of the 30 s PV.
//! 120 BPM: a beat is 0.5 s = 12 frames at 24 fps. The last frame is black like the first, so it loops.
//! `events()` is the single source of truth for the sound: tools/pv_short_audio.py reads it (`--film-events`).
use super::*;
use crate::fighter2d::{self, ANTICIP, CLASH, Canvas, Ease, FOLLOW, Fs, Fx, HIT, Key, READY, RISE, RUN, SHEATHED, STRIKE, fs, run_pose, seq};

const SHORT_SECS: f32 = 10.0;

#[derive(Clone, Copy)]
enum K {
    Void,
    MioEye,
    KageEye,
    Dash(u8),
    Clash,
    Slow,
    After,
    Title,
}

/// (start, end, drawings per second step, what)
const SHOTS: [(f32, f32, usize, K); 9] = [
    (0.0, 0.5, 1, K::Void),
    (0.5, 1.0, 1, K::MioEye),
    (1.0, 1.5, 1, K::KageEye),
    (1.5, 2.5, 2, K::Dash(0)),
    (2.5, 3.0, 2, K::Dash(1)),
    (3.0, 3.5, 2, K::Clash),
    (3.5, 5.75, 2, K::Slow),
    (5.75, 7.5, 2, K::After),
    (7.5, 10.0, 1, K::Title),
];

/// Paper impact frames: (start, length).
const IMPACTS_S: [(f32, f32); 3] = [(3.0, 0.125), (5.0, 0.085), (6.5, 0.085)];
const SLASH_T: f32 = 6.5;
const SPLIT_T: f32 = 7.0;
const FLASH_T: f32 = 5.5;
const SILENCE: f32 = 2.0 / FPS as f32;

const WORDS: [Txt; 2] = [
    Txt { t0: 7.5, t1: 10.0, text: "ONE", y: 5, dw: 5, dh: 2, col: CYAN },
    Txt { t0: 8.0, t1: 10.0, text: "CUT", y: 21, dw: 5, dh: 2, col: ORANGE },
];

fn ev(out: &mut Vec<String>, t: f32, kind: &str, extra: &str) {
    out.push(format!("{{\"t\":{t:.4},\"frame\":{},\"type\":\"{kind}\"{extra}}}", (t * FPS as f32).round() as i32));
}

/// Every visual event with its time in seconds, as JSON. The sound is built from this list.
pub fn events_json() -> String {
    let mut e: Vec<String> = vec![];
    for (i, s) in SHOTS.iter().enumerate() {
        ev(&mut e, s.0, "shot_cut", &format!(",\"shot\":{i}"));
    }
    ev(&mut e, 0.0, "hook_line", ",\"dur\":0.35");
    ev(&mut e, 0.35, "hook_tick", "");
    ev(&mut e, 0.5, "eye_open", ",\"who\":\"mio\",\"n\":0");
    ev(&mut e, 1.0, "eye_open", ",\"who\":\"kage\",\"n\":1");
    ev(&mut e, 1.5, "dash_start", ",\"dur\":1.5");
    ev(&mut e, 2.9, "speed_peak", "");
    ev(&mut e, 3.0, "clash", "");
    for (i, p) in IMPACTS_S.iter().enumerate() {
        ev(&mut e, p.0, "impact", &format!(",\"dur\":{:.4},\"n\":{i}", p.1));
    }
    ev(&mut e, 3.0, "sparks", ",\"dur\":0.4");
    ev(&mut e, 3.5, "slowmo_start", ",\"dur\":2.0");
    ev(&mut e, 5.0, "pass", "");
    ev(&mut e, 5.0, "sparks", ",\"dur\":0.3");
    ev(&mut e, FLASH_T, "slowmo_end", "");
    ev(&mut e, FLASH_T, "flash", &format!(",\"dur\":0.25,\"silence\":{SILENCE:.4}"));
    ev(&mut e, 5.75, "aftermath", "");
    ev(&mut e, SLASH_T, "slash", "");
    ev(&mut e, SPLIT_T, "split", "");
    for (i, w) in WORDS.iter().enumerate() {
        ev(&mut e, w.t0, "title_word", &format!(",\"text\":\"{}\",\"n\":{i},\"letters\":{},\"letter_dt\":0.045", w.text, w.text.len()));
    }
    ev(&mut e, 8.5, "title_settle", "");
    ev(&mut e, 8.5, "tagline", "");
    ev(&mut e, 9.0, "coming_soon", "");
    ev(&mut e, 9.5, "fade", ",\"dur\":0.5");
    ev(&mut e, SHORT_SECS, "loop_end", "");
    e.sort_by(|a, b| {
        let t = |s: &String| s[5..s.find(',').unwrap()].parse::<f32>().unwrap();
        t(a).partial_cmp(&t(b)).unwrap()
    });
    format!("{{\"fps\":{FPS},\"duration\":{SHORT_SECS},\"bpm\":120,\"events\":[\n{}\n]}}\n", e.join(",\n"))
}

/// Where two line segments cross, if they do.
pub(super) fn crossing(a0: (f32, f32), a1: (f32, f32), b0: (f32, f32), b1: (f32, f32)) -> Option<(f32, f32)> {
    let (d1, d2) = ((a1.0 - a0.0, a1.1 - a0.1), (b1.0 - b0.0, b1.1 - b0.1));
    let den = d1.0 * d2.1 - d1.1 * d2.0;
    if den.abs() < 1e-4 {
        return None;
    }
    let t = ((b0.0 - a0.0) * d2.1 - (b0.1 - a0.1) * d2.0) / den;
    let u = ((b0.0 - a0.0) * d1.1 - (b0.1 - a0.1) * d1.0) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some((a0.0 + t * d1.0, a0.1 + t * d1.1))
}

fn paper_on(t: f32) -> bool {
    IMPACTS_S.iter().any(|&(a, d)| t >= a - 1e-4 && t < a + d - 1e-4)
}

/// White-out: ramps up before 5.5, full for 3 frames, decays across the cut at 5.75; short flashes on the title.
fn white(t: f32) -> f32 {
    if t < 5.25 {
        0.0
    } else if t < FLASH_T {
        ((t - 5.25) / 0.25).powi(2)
    } else if t < FLASH_T + 0.125 {
        1.0
    } else if t < 6.0 {
        1.0 - (t - FLASH_T - 0.125) / (6.0 - FLASH_T - 0.125)
    } else if (7.5..7.6).contains(&t) {
        1.0 - (t - 7.5) / 0.1
    } else if (8.0..8.08).contains(&t) {
        1.0 - (t - 8.0) / 0.08
    } else {
        0.0
    }
}

/// Fade to black at the end so the last frame equals the first.
fn fade(t: f32) -> f32 {
    if t < 9.5 {
        0.0
    } else {
        clamp01((t - 9.5) / 0.4)
    }
}

/// Where a fighter is and which drawing it is on, at absolute time `t` (seconds). Mio comes from the left, Kage is
/// the mirror image from the right. Each shot has its own framing; the cloth chains never look back past `shot_start`.
fn fs_at(who: Who, t: f32) -> Fs {
    let mio = who == Who::Mio;
    let dir = if mio { 1.0 } else { -1.0 };
    let sc = if mio { 1.0 } else { 1.06 };
    let mx = |x: f32| if mio { x } else { COLS as f32 - x };
    let fx = |x: f32, gy: f32, s: f32| Fx { x: mx(x), gy, s: s * sc, dir };
    let io = |a: f32, b: f32, t0: f32, t1: f32, e: fn(f32) -> f32| lerp(a, b, e(clamp01((t - t0) / (t1 - t0))));
    if t < 2.5 {
        // wide dash: sprinting in, sword still sheathed behind
        let x = io(-2.0, 40.0, 1.5, 2.5, |u| u.powf(1.5));
        fs(run_pose(t), fx(x, 68.0, 0.85), t, 1.0)
    } else if t < 3.0 {
        // closer: sprint, pull back (anticipation), lunge
        let x = io(10.0, 33.0, 2.5, 2.75, smooth) + io(0.0, -4.0, 2.75, 2.83, smooth) + io(0.0, 11.0, 2.83, 2.93, ease_out);
        let pose = if t < 2.75 { run_pose(t) } else { seq(&[Key(2.75, run_pose(2.75), 0.0, Ease::Io), Key(2.83, ANTICIP, 0.08, Ease::Io), Key(2.93, STRIKE, 0.1, Ease::Out)], t) };
        fs(pose, fx(x, 72.0, 1.1), t, 1.0)
    } else if t < 3.5 {
        // blades crossed, pressing
        let u = (t - 3.0) / 0.5;
        let press = (t * 38.0).sin() * 0.8 * (1.0 - u);
        let shake = (h((t * 24.0) as u32 * 3 + 1) - 0.5) * 2.4 * (1.0 - u);
        let g = 76.0 +(h((t * 24.0) as u32 * 3 + 2) - 0.5) * 1.6 * (1.0 - u);
        let pose = CLASH.lerp(&STRIKE, 0.0);
        let mut f = fs(pose, fx(49.0 + press + shake, g, 1.5), t, 2.5);
        if !mio {
            f.pose.bl = 46.0;
        }
        f
    } else if t < 5.75 {
        slow_fs(who, t)
    } else {
        let s = 1.45;
        let g = 80.0;
        if mio {
            let p = seq(
                &[Key(5.75, FOLLOW, 0.0, Ease::Io), Key(6.45, FOLLOW, 0.0, Ease::Io), Key(6.85, RISE, 0.4, Ease::Io), Key(7.2, SHEATHED, 0.35, Ease::Io)],
                t,
            );
            fs(p, fx(108.0, g, s), t, 1.0)
        } else {
            let p = seq(&[Key(5.75, FOLLOW, 0.0, Ease::Io), Key(6.4, FOLLOW, 0.0, Ease::Io), Key(6.52, HIT, 0.12, Ease::Out), Key(7.4, SHEATHED, 0.9, Ease::Io)], t);
            let x = 108.0 + io(0.0, -3.0, 6.4, 6.6, ease_out);
            fs(p, fx(x, g, s), t, 1.0)
        }
    }
}

/// The slow-motion pass: stand off, crouch, lunge, cross at 5.0, follow through.
fn slow_fs(who: Who, t: f32) -> Fs {
    let mio = who == Who::Mio;
    let dir = if mio { 1.0 } else { -1.0 };
    let sc = if mio { 1.0 } else { 1.06 };
    let io = |a: f32, b: f32, t0: f32, t1: f32, e: fn(f32) -> f32| lerp(a, b, e(clamp01((t - t0) / (t1 - t0))));
    let x = io(24.0, 33.0, 3.5, 4.3, smooth) + io(0.0, -2.5, 4.3, 4.55, smooth) + io(0.0, 27.5, 4.55, 4.95, ease_in) + io(0.0, 34.0, 4.95, 5.12, smooth) + io(0.0, 16.0, 5.12, 5.55, ease_out);
    let x = if mio { x } else { COLS as f32 - x };
    let pose = seq(
        &[
            Key(3.5, READY, 0.0, Ease::Io),
            Key(4.3, READY, 0.0, Ease::Io),
            Key(4.55, ANTICIP, 0.25, Ease::Io),
            Key(4.82, RUN[1], 0.27, Ease::In),
            Key(4.95, STRIKE, 0.13, Ease::Out),
            Key(5.2, STRIKE, 0.0, Ease::Io),
            Key(5.6, FOLLOW, 0.4, Ease::Io),
        ],
        t,
    );
    fs(pose, Fx { x, gy: 72.0, s: 1.15 * sc, dir }, t, 1.0)
}

fn shot_start(k: K) -> f32 {
    SHOTS.iter().find(|s| matches!((s.3, k), (K::Dash(a), K::Dash(b)) if a == b) || (std::mem::discriminant(&s.3) == std::mem::discriminant(&k) && !matches!(k, K::Dash(_)))).map(|s| s.0).unwrap_or(0.0)
}

fn camera(k: K, t: f32, fq: usize) -> Camera {
    let mid = Vec3::new(0.0, 1.2, 0.0);
    match k {
        K::Dash(0) => cam(Vec3::new(0.0, 0.7, lerp(5.6, 4.6, (t - 1.5) / 1.0)), Vec3::new(0.0, 1.1, 0.0), 40.0),
        K::Dash(_) => {
            let u = (t - 2.5) / 0.5;
            cam(Vec3::new(0.0, lerp(0.9, 1.3, u), lerp(4.6, 3.8, u)) + shake(fq, 0.01 + 0.05 * u * u), mid, lerp(42.0, 38.0, u))
        }
        K::Clash => {
            let u = (t - 3.0) / 0.5;
            cam(Vec3::new(lerp(0.3, 1.0, u), 0.25, 3.0) + shake(fq, 0.05 * (1.0 - u)), mid, 45.0)
        }
        K::Slow => {
            let u = (t - 3.5) / 2.25;
            cam(Vec3::new(lerp(-1.2, 0.6, u), 0.2, lerp(5.2, 3.5, u)), Vec3::new(0.0, 1.25, 0.0), 52.0)
        }
        _ => {
            let u = (t - 5.75) / 1.75;
            cam(Vec3::new(0.0, lerp(0.8, 1.0, u), lerp(3.2, 2.5, u)), Vec3::new(0.0, 1.3, 0.0), 42.0)
        }
    }
}

pub fn prerender(atlas: &Atlas, opts: &Options) -> Vec<FilmFrame> {
    let total = (SHORT_SECS * FPS as f32) as usize;
    let mut frames = Vec::with_capacity(total);
    let mut scene = build_scene();
    let mut renderer = Renderer::default();
    let mut cur = usize::MAX;
    let mut base = vec![BLANK; COLS * ROWS];
    for f in 0..total {
        let t = f as f32 / FPS as f32;
        let ci = SHOTS.iter().rposition(|c| t >= c.0 - 1e-4).unwrap_or(0);
        if ci != cur {
            cur = ci;
            renderer = Renderer::default();
        }
        let (c0, _, step, kind) = SHOTS[ci];
        let f0 = (c0 * FPS as f32).round() as usize;
        let fq = f0 + (f - f0) / step * step;
        let paper = paper_on(t);
        if fq == f {
            let tq = fq as f32 / FPS as f32;
            let tl = tq - c0;
            base = vec![BLANK; COLS * ROWS];
            match kind {
                K::Void => {
                    let n = (COLS as f32 * ease_out(clamp01(tq / 0.3))) as i32;
                    let thick = tq > 0.35;
                    for i in 0..n {
                        let x = COLS as i32 / 2 - n / 2 + i;
                        let f = 0.45 + 0.55 * (1.0 - (i as f32 / n as f32 * 2.0 - 1.0).abs());
                        let c = if thick { mix3(CYAN, WHITE, 0.6) } else { CYAN };
                        put(&mut base, x, 24, '█', scale3(c, f), true);
                        put(&mut base, x, 23, '─', scale3(CYAN, f * 0.6), true);
                        put(&mut base, x, 25, '─', scale3(CYAN, f * 0.6), true);
                        if thick {
                            put(&mut base, x, 22, '█', scale3(CYAN, f * 0.8), true);
                            put(&mut base, x, 26, '█', scale3(CYAN, f * 0.8), true);
                        }
                    }
                    if thick {
                        let m = (6.0 + (tq - 0.35) * 60.0) as i32;
                        for j in -m..=m {
                            put(&mut base, COLS as i32 / 2, 24 + j, '│', scale3(WHITE, 1.0 - j.abs() as f32 / (m as f32 + 1.0)), true);
                        }
                    }
                }
                K::MioEye | K::KageEye => {
                    let mio = matches!(kind, K::MioEye);
                    let col = if mio { CYAN } else { ORANGE };
                    let (cells, e) = if mio { eye_cel(Who::Mio, 0, tq, tl * 1.8) } else { eye_cel(Who::Kage, 1, tq, tl) };
                    base = cells;
                    cel::eye_shine(&mut base, &e, if mio { clamp01(1.0 - (tl - 0.3).abs() * 6.0) } else { clamp01(1.0 - tl * 5.0) });
                    if !mio {
                        speed_lines(&mut base, fq as u32, 30, 0.4, col, 0.6);
                    }
                    cel::embers(&mut base, tq, 24, col, WHITE, 1.0);
                }
                K::Dash(_) | K::Clash | K::Slow | K::After => {
                    scene.light_dir = Vec3::new(0.4, 0.8, 0.5).normalize();
                    let camera = camera(kind, tq, fq);
                    let frame = renderer.render(&scene, &camera, atlas, opts, COLS, ROWS);
                    let mut back = vec![BLANK; COLS * ROWS];
                    cel::moon(&mut back, &camera, Vec3::new(-0.12, 0.34, -1.0).normalize(), 11.0, [190.0, 215.0, 255.0]);
                    let gk = match kind {
                        K::Dash(_) => 0.6,
                        K::Slow => 0.35,
                        _ => 0.0,
                    };
                    if gk > 0.0 {
                        cel::grid(&mut back, &camera, scale3(CYAN, gk), 0.5 + 0.5 * (tq * 4.0).sin());
                    }
                    for (i, c) in back.iter_mut().enumerate() {
                        if frame.gbuffer.cell_coverage(i % COLS, i / COLS) > 0.2 {
                            *c = BLANK;
                        }
                    }
                    base = back;
                    for (b, c) in base.iter_mut().zip(&frame.cells) {
                        if c.ch != ' ' {
                            *b = *c;
                        }
                    }
                    let speed = match kind {
                        K::Dash(_) => 0.3 + 0.7 * clamp01((tq - 1.5) / 1.4),
                        _ => 0.0,
                    };
                    let close = !matches!(kind, K::Dash(0));
                    if speed > 0.0 {
                        speed_lines(&mut base, fq as u32, if close { 22 } else { 50 }, 0.32 + 0.1 * h(fq as u32), [200.0, 225.0, 255.0], speed);
                    }
                    cel::embers(&mut base, tq, if close { 10 } else { 40 }, ORANGE, CYAN, 1.0);
                    if paper {
                        speed_lines(&mut base, fq as u32 + 3, if close { 35 } else { 90 }, 0.1, WHITE, 1.0);
                    }
                    // the two fighters, drawn as 2D cels over the backdrop
                    let t0 = shot_start(kind);
                    let (am, ak) = (|t: f32| fs_at(Who::Mio, t), |t: f32| fs_at(Who::Kage, t));
                    let mut cv = Canvas::default();
                    fighter2d::draw(&mut cv, Who::Kage, &ak, tq, t0);
                    fighter2d::draw(&mut cv, Who::Mio, &am, tq, t0);
                    cv.clear_halo(&mut base, 1);
                    cv.blit(&mut base);
                    let spark = match kind {
                        K::Clash => Some(0.9 - 0.8 * (tq - 3.0) / 0.5),
                        K::Slow if (4.95..5.3).contains(&tq) => Some(0.7 - 0.5 * (tq - 4.95) / 0.35),
                        _ => None,
                    };
                    if let Some(kk) = spark {
                        let (a, b) = (fighter2d::blade(&am(tq), Who::Mio), fighter2d::blade(&ak(tq), Who::Kage));
                        let at = if matches!(kind, K::Clash) { crossing(a.0, a.2, b.0, b.2).unwrap_or(((a.2.0 + b.2.0) * 0.5, (a.2.1 + b.2.1) * 0.5)) } else { ((a.2.0 + b.2.0) * 0.5, (a.2.1 + b.2.1) * 0.5) };
                        fighter2d::sparks(&mut base, at, kk, fq as u32 * 17, WHITE, ORANGE);
                    }
                    if matches!(kind, K::After) {
                        let slope = -0.55;
                        let sh = clamp01((tq - SPLIT_T) / 0.3);
                        if sh > 0.0 {
                            base = cel::split(&base, slope, ease_out(sh) * 14.0);
                            cel::embers(&mut base, tq, 60, WHITE, ORANGE, 2.0);
                        }
                        let kk = clamp01((tq - SLASH_T) / 0.25);
                        if kk > 0.0 {
                            cel::slash(&mut base, slope, kk, WHITE);
                        }
                    }
                }
                K::Title => {
                    let u = (tq - 7.5) / 2.5;
                    let camera = cam(Vec3::new(0.0, lerp(1.0, 1.5, u), lerp(9.0, 7.4, u)), Vec3::new(0.0, 1.8, 0.0), 55.0);
                    cel::moon(&mut base, &camera, Vec3::new(0.0, 0.3, -1.0).normalize(), 15.0, [190.0, 215.0, 255.0]);
                    cel::grid(&mut base, &camera, scale3(CYAN, 0.7), 0.6);
                    cel::embers(&mut base, tq, 70, ORANGE, CYAN, 1.2);
                }
            }
        }
        frames.push(finish(&base, t, paper));
    }
    frames
}

fn finish(base: &[Cell], t: f32, paper: bool) -> FilmFrame {
    let mut cells = base.to_vec();
    cells[..BAR_ROWS * COLS].fill(BLANK);
    cells[(ROWS - BAR_ROWS) * COLS..].fill(BLANK);
    for tx in WORDS.iter() {
        cel::draw_txt(&mut cells, tx, t);
    }
    if t >= 8.5 {
        cel::small_text(&mut cells, "AN ASCII ANIME", 38, WHITE, smooth((t - 8.5) / 0.2));
    }
    if t >= 9.0 {
        let blink = if t < 9.3 { ((t * 12.0) as i32 % 2) as f32 } else { 1.0 };
        cel::small_text(&mut cells, "COMING SOON", 40, CYAN, smooth((t - 9.0) / 0.2) * (0.4 + 0.6 * blink));
    }
    let p = if paper { 1.0 } else { 0.0 };
    let (w, fd) = (white(t), fade(t));
    for c in cells.iter_mut() {
        if c.ch != ' ' {
            let col = mix3([c.fg.0 as f32, c.fg.1 as f32, c.fg.2 as f32], INK, p);
            c.fg = rgb(scale3(mix3(col, [255.0; 3], w), 1.0 - fd));
        }
    }
    let bg = mix3(mix3([0.0; 3], PAPER, p), [255.0; 3], w);
    FilmFrame { cells, bg: rgb(scale3(bg, 1.0 - fd)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_json_in_range_and_ordered() {
        let j = events_json();
        assert!(j.contains("\"type\":\"clash\"") && j.contains("\"type\":\"flash\""));
        let ts: Vec<f32> = j.lines().filter(|l| l.starts_with("{\"t\"")).map(|l| l[5..l.find(',').unwrap()].parse().unwrap()).collect();
        assert!(ts.windows(2).all(|w| w[0] <= w[1]) && ts.iter().all(|&t| (0.0..=SHORT_SECS).contains(&t)));
    }

    #[test]
    fn shots_tile_the_short() {
        assert_eq!(SHOTS[0].0, 0.0);
        assert!(SHOTS.windows(2).all(|w| w[0].1 == w[1].0));
        assert_eq!(SHOTS[8].1, SHORT_SECS);
    }
}
