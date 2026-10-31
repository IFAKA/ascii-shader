//! ONE CUT: the 30 s teaser PV. 120 BPM, so a beat is 0.5 s and a bar is 2 s; every cut lands on a beat.
//! Everything is terminal cells. The same frames are played in the terminal and drawn into the MP4.
use crate::cel::{self, CYAN, Col, Eye, ORANGE, Txt, WHITE};
use crate::chars::{self, ARM_L, ARM_R, HEAD, HY, LEAN, LEG_L, LEG_R, OX, OZ, Pose, SW_P, SW_Q, TWIST, Who, stand};
use crate::compose::{BLANK, Cell};
use crate::film::{BAR_ROWS, COLS, FPS, FilmFrame, INK, PAPER, PI, ROWS, clamp01, ease_in, ease_out, h, lerp, mix3, project, put, rgb, scale3, smooth, speed_lines};
use crate::glyphs::Atlas;
use crate::render::{Options, Renderer};
use crate::scene::{self, Camera, Object, Scene};
use glam::{Mat4, Vec3, Vec4};

#[path = "pvshort.rs"]
pub mod short;

const SECS: f32 = 30.0;

#[derive(Clone, Copy)]
enum Kind {
    Void,
    EyeM(u8),
    EyeK(u8),
    Shot(u8),
    Title,
}

/// (start, end, drawings per second step, what)
const CUTS: [(f32, f32, usize, Kind); 31] = [
    (0.0, 2.0, 1, Kind::Void),
    (2.0, 4.0, 2, Kind::EyeM(0)),
    (4.0, 6.0, 2, Kind::EyeK(0)),
    (6.0, 8.0, 2, Kind::Shot(0)),
    (8.0, 8.5, 2, Kind::Shot(1)),
    (8.5, 9.0, 2, Kind::Shot(2)),
    (9.0, 9.5, 2, Kind::Shot(3)),
    (9.5, 10.0, 2, Kind::Shot(4)),
    (10.0, 10.5, 1, Kind::EyeM(1)),
    (10.5, 11.0, 1, Kind::EyeK(1)),
    (11.0, 12.0, 2, Kind::Shot(5)),
    (12.0, 13.0, 2, Kind::Shot(6)),
    (13.0, 14.0, 2, Kind::Shot(7)),
    (14.0, 14.5, 2, Kind::Shot(8)),
    (14.5, 15.0, 2, Kind::Shot(9)),
    (15.0, 15.5, 1, Kind::EyeM(2)),
    (15.5, 16.0, 1, Kind::EyeK(2)),
    (16.0, 16.5, 2, Kind::Shot(10)),
    (16.5, 17.0, 2, Kind::Shot(11)),
    (17.0, 17.5, 2, Kind::Shot(12)),
    (17.5, 18.0, 2, Kind::Shot(13)),
    (18.0, 18.5, 2, Kind::Shot(14)),
    (18.5, 19.0, 2, Kind::Shot(15)),
    (19.0, 19.5, 2, Kind::Shot(16)),
    (19.5, 20.0, 2, Kind::Shot(17)),
    (20.0, 22.0, 1, Kind::Shot(18)),
    (22.0, 24.0, 2, Kind::Shot(19)),
    (24.0, 26.0, 1, Kind::Title),
    (26.0, 28.0, 1, Kind::Title),
    (28.0, 29.0, 1, Kind::Title),
    (29.0, 30.0, 1, Kind::Title),
];

/// Paper (ink-on-paper) impact frames: (start, length) in seconds.
const IMPACTS: [(f32, f32); 5] = [(13.96, 0.09), (14.5, 0.04), (15.0, 0.13), (20.0, 0.06), (22.0, 0.09)];

const TEXTS: [Txt; 6] = [
    Txt { t0: 0.5, t1: 2.0, text: "TWO BLADES.", y: 11, dw: 2, dh: 1, col: CYAN },
    Txt { t0: 6.5, t1: 8.0, text: "ONE ARENA.", y: 8, dw: 2, dh: 1, col: CYAN },
    Txt { t0: 24.0, t1: 30.0, text: "ONE", y: 5, dw: 5, dh: 2, col: CYAN },
    Txt { t0: 24.5, t1: 30.0, text: "CUT", y: 21, dw: 5, dh: 2, col: ORANGE },
    Txt { t0: 0.0, t1: 0.0, text: "", y: 0, dw: 1, dh: 1, col: WHITE },
    Txt { t0: 0.0, t1: 0.0, text: "", y: 0, dw: 1, dh: 1, col: WHITE },
];

struct Place {
    pos: Vec3,
    yaw: f32,
    pose: Pose,
}

struct View {
    cam: Camera,
    mio: Option<Place>,
    kage: Option<Place>,
    sparks: Option<f32>,
    speed: f32,
    moon: bool,
    shake: f32,
}

const MIO_YAW: f32 = PI / 2.0;
const KAGE_YAW: f32 = -PI / 2.0;

fn cam(pos: Vec3, target: Vec3, fov: f32) -> Camera {
    let mut c = Camera::look_at(pos, target);
    c.fov_y = fov.to_radians();
    c
}

// ---- poses (angles in radians; see chars.rs) ----
fn ready() -> Pose {
    stand().limb(ARM_L, [0.9, 0.1, 1.2]).limb(ARM_R, [1.0, 0.15, 1.2]).limb(LEG_L, [0.55, 0.15, 0.7]).limb(LEG_R, [-0.35, 0.15, 0.5]).with(&[(LEAN, 0.12), (SW_P, 0.5), (SW_Q, 0.05)])
}
fn hand_on_hilt() -> Pose {
    stand().limb(ARM_R, [0.55, 0.1, 1.0]).limb(ARM_L, [0.5, 0.2, 0.9]).limb(LEG_L, [0.3, 0.1, 0.3]).limb(LEG_R, [-0.2, 0.1, 0.2]).with(&[(SW_P, -0.2), (SW_Q, 0.2), (LEAN, 0.08)])
}
fn draw_pose() -> Pose {
    stand().limb(ARM_R, [-0.5, 0.35, 0.5]).limb(ARM_L, [0.5, 0.2, 0.9]).limb(LEG_L, [0.75, 0.2, 0.9]).limb(LEG_R, [-0.55, 0.25, 0.35]).with(&[(LEAN, 0.3), (TWIST, 0.35), (SW_P, 0.0), (SW_Q, 1.25)])
}
fn run_a() -> Pose {
    stand().limb(LEG_L, [0.95, 0.05, 1.15]).limb(LEG_R, [-0.85, 0.05, 0.25]).limb(ARM_L, [-0.6, 0.1, 0.9]).limb(ARM_R, [-0.9, 0.1, 0.5]).with(&[(LEAN, 0.55), (SW_P, 0.1), (SW_Q, 2.95)])
}
fn run_b() -> Pose {
    stand().limb(LEG_L, [-0.85, 0.05, 0.25]).limb(LEG_R, [0.95, 0.05, 1.15]).limb(ARM_L, [0.9, 0.1, 0.9]).limb(ARM_R, [-0.5, 0.1, 0.6]).with(&[(LEAN, 0.55), (SW_P, 0.1), (SW_Q, 2.95)])
}
fn strike() -> Pose {
    stand().limb(LEG_L, [1.15, 0.1, 0.35]).limb(LEG_R, [-0.95, 0.2, 0.05]).limb(ARM_R, [1.45, 0.0, 0.15]).limb(ARM_L, [-0.4, 0.9, 0.6]).with(&[(LEAN, 0.5), (SW_P, 0.02), (SW_Q, 0.0)])
}
fn parry_high() -> Pose {
    ready().limb(ARM_R, [2.5, 0.3, 0.5]).limb(ARM_L, [2.3, 0.1, 0.7]).with(&[(LEAN, -0.1), (SW_P, 0.15), (SW_Q, 1.35)])
}
fn low_slash() -> Pose {
    ready().limb(ARM_R, [0.6, 0.9, 0.4]).limb(LEG_L, [0.9, 0.5, 1.0]).limb(LEG_R, [-0.7, 0.5, 0.3]).with(&[(LEAN, 0.6), (SW_P, -0.35), (SW_Q, 1.1), (TWIST, -0.3)])
}
fn jump_pose() -> Pose {
    stand().limb(LEG_L, [1.3, 0.15, 1.8]).limb(LEG_R, [0.7, 0.15, 1.4]).limb(ARM_R, [3.0, 0.2, 0.4]).limb(ARM_L, [2.2, 0.6, 0.5]).with(&[(HY, 2.1), (LEAN, 0.2), (SW_P, 1.1), (SW_Q, 0.3)])
}
fn spin_pose() -> Pose {
    ready().limb(ARM_R, [1.5, 1.2, 0.2]).limb(ARM_L, [1.2, 1.3, 0.3]).with(&[(TWIST, 1.1), (LEAN, 0.2), (SW_P, 0.0), (SW_Q, 1.6)])
}
fn slash_down() -> Pose {
    strike().limb(ARM_R, [2.4, 0.1, 0.3]).with(&[(SW_P, -0.9), (SW_Q, 0.0), (LEAN, 0.7)])
}
fn after_pose() -> Pose {
    stand().with(&[(LEAN, 0.12), (HEAD, 0.0)])
}

fn mio_at(x: f32, z: f32, pose: Pose) -> Option<Place> {
    Some(Place { pos: Vec3::new(x, 0.0, z), yaw: MIO_YAW, pose })
}
fn kage_at(x: f32, z: f32, pose: Pose) -> Option<Place> {
    Some(Place { pos: Vec3::new(x, 0.0, z), yaw: KAGE_YAW, pose })
}

fn shake(fq: usize, amp: f32) -> Vec3 {
    Vec3::new(h(fq as u32 * 7 + 1) - 0.5, h(fq as u32 * 7 + 2) - 0.5, h(fq as u32 * 7 + 3) - 0.5) * 2.0 * amp
}

/// What is in each 3D shot. The camera is chosen after the fighters are placed (some shots follow a hand or a blade).
fn view(id: u8, t: f32, fq: usize, pm: &Option<chars::Posed>, pk: &Option<chars::Posed>) -> Camera {
    let (t0, t1, _, _) = CUTS.iter().find(|c| matches!(c.3, Kind::Shot(i) if i == id)).copied().unwrap();
    let u = (t - t0) / (t1 - t0);
    let _ = (pm, pk);
    let mid = Vec3::new(0.0, 1.2, 0.0);
    match id {
        0 => cam(Vec3::new(0.0, lerp(0.7, 1.1, u), lerp(9.0, 6.6, u)), Vec3::new(0.0, 1.5, 0.0), 42.0),
        1 => cam(Vec3::new(-2.1 - 0.3 * u, 1.2, 2.3), Vec3::new(-4.0, 1.1, 0.0), 36.0),
        2 => cam(Vec3::new(2.1 + 0.3 * u, 1.2, -2.3), Vec3::new(4.0, 1.1, 0.0), 36.0),
        3 => cam(Vec3::new(lerp(-2.3, -2.7, u), 0.15, 1.7), Vec3::new(-4.0, 0.3, 0.0), 50.0),
        4 => cam(Vec3::new(lerp(2.3, 2.7, u), 0.15, -1.7), Vec3::new(4.0, 0.3, 0.0), 50.0),
        5 => cam(Vec3::new(0.0, 0.7, lerp(7.5, 6.5, u)), Vec3::new(0.0, 1.2, 0.0), 42.0),
        6 => cam(Vec3::new(0.0, 0.8, lerp(7.5, 6.0, u)), Vec3::new(0.0, 1.1, 0.0), 40.0),
        7 => cam(Vec3::new(0.0, lerp(1.0, 1.3, u), lerp(7.0, 3.8, u)) + shake(fq, 0.01 + 0.05 * u * u), mid, lerp(48.0, 38.0, u)),
        8 => cam(Vec3::new(lerp(0.3, 1.0, u), 0.25, 3.0), mid, 45.0),
        9 => cam(Vec3::new(0.0, 6.5, 1.6), Vec3::new(0.0, 0.7, 0.0), 45.0),
        10 => cam(Vec3::new(2.6, 1.8, 4.6), mid, 45.0),
        11 => cam(Vec3::new(-0.5, 0.25, 9.0), Vec3::new(0.0, 1.9, 0.0), 55.0),
        12 => cam(Vec3::new(0.0, 0.15, 3.2), Vec3::new(0.0, 1.1, 0.0), 70.0),
        13 => cam(Vec3::new(lerp(-2.0, 2.0, u), 1.4, 3.6), mid, 45.0),
        14 => cam(Vec3::new(-4.3, 1.5, 1.6), Vec3::new(0.5, 1.0, 0.0), 45.0),
        15 => cam(Vec3::new(0.0, 1.3, 2.3), Vec3::new(0.0, 1.3, 0.0), 38.0),
        16 => cam(Vec3::new(1.2, 0.3, 4.0), Vec3::new(0.0, 2.0, 0.0), 55.0),
        17 => cam(Vec3::new(0.0, 1.6, 12.0), Vec3::new(0.0, 2.0, 0.0), 45.0),
        18 => {
            let a = lerp(0.2, 1.0, (t - 20.0) / 2.0);
            cam(Vec3::new(a.sin() * 4.8, 1.0, a.cos() * 4.8), mid, 45.0)
        }
        _ => cam(Vec3::new(0.0, lerp(0.8, 1.0, u), lerp(3.4, 2.7, u)), Vec3::new(0.0, 1.3, 0.0), 42.0),
    }
}

fn fighters(id: u8, t: f32) -> (Option<Place>, Option<Place>, Option<f32>, f32) {
    let (t0, t1, _, _) = CUTS.iter().find(|c| matches!(c.3, Kind::Shot(i) if i == id)).copied().unwrap();
    let u = (t - t0) / (t1 - t0);
    let g = |keys: &[(f32, Pose)], snap: f32| chars::track(keys, t, snap);
    match id {
        0 => (mio_at(-4.0, 0.0, g(&[(6.0, stand()), (7.3, stand()), (7.5, ready())], 0.3)), kage_at(4.0, 0.0, g(&[(6.0, stand()), (7.4, stand()), (7.6, ready())], 0.3)), None, 0.0),
        1 => (mio_at(-4.0, 0.0, g(&[(8.0, stand()), (8.1, hand_on_hilt())], 0.2)), kage_at(4.0, 0.0, stand()), None, 0.0),
        2 => (mio_at(-4.0, 0.0, hand_on_hilt()), kage_at(4.0, 0.0, g(&[(8.5, stand()), (8.6, hand_on_hilt())], 0.2)), None, 0.0),
        3 => (mio_at(-4.0, 0.0, ready().lerp(&hand_on_hilt(), 0.5)), kage_at(4.0, 0.0, ready()), None, 0.0),
        4 => (mio_at(-4.0, 0.0, ready()), kage_at(4.0, 0.0, ready().lerp(&hand_on_hilt(), 0.5)), None, 0.0),
        5 => (
            mio_at(-3.5, 0.0, g(&[(11.0, ready()), (11.4, ready()), (11.55, draw_pose())], 0.12)),
            kage_at(3.5, 0.0, g(&[(11.0, ready()), (11.5, ready()), (11.65, draw_pose())], 0.12)),
            None,
            0.0,
        ),
        6 => {
            let k = ease_in(clamp01((t - 12.0) / 1.0));
            let ph = |x: f32| if ((x * 3.0) as i32) % 2 == 0 { run_a() } else { run_b() };
            let p = ph(t * 5.0);
            (mio_at(lerp(-4.0, -2.2, k), 0.0, p), kage_at(lerp(4.0, 2.2, k), 0.0, p), None, 0.8)
        }
        7 => {
            let k = ease_in(clamp01((t - 13.0) / 0.85));
            let ph = if (t * 14.0) as i32 % 2 == 0 { run_a() } else { run_b() };
            let p = if t > 13.85 { strike() } else { ph };
            (mio_at(lerp(-2.2, -0.55, k), 0.0, p), kage_at(lerp(2.2, 0.55, k), 0.0, p), None, 0.9)
        }
        8 => (mio_at(-0.55, 0.0, strike()), kage_at(0.55, 0.0, strike()), Some(0.5 * u + 0.1), 0.0),
        9 => (mio_at(-0.55, 0.0, strike()), kage_at(0.55, 0.0, strike()), Some(0.6 + u * 0.3), 0.0),
        10 => (mio_at(-0.6, 0.0, parry_high()), kage_at(0.6, 0.0, slash_down()), Some(0.3), 0.0),
        11 => (mio_at(-1.5, 0.0, jump_pose()), kage_at(1.4, 0.0, low_slash()), None, 0.4),
        12 => (mio_at(-0.9, 0.0, parry_high()), kage_at(0.9, 0.0, low_slash()), Some(0.2), 0.0),
        13 => (mio_at(-0.8, 0.0, spin_pose().lerp(&strike(), u)), kage_at(0.9, 0.0, parry_high()), None, 0.5),
        14 => (mio_at(-1.0, 0.0, strike()), kage_at(1.0, 0.0, parry_high()), Some(0.2 + u), 0.0),
        15 => (mio_at(-0.5, 0.0, strike()), kage_at(0.5, 0.0, strike()), Some(0.1 + u * 0.3), 0.0),
        16 => (mio_at(-1.0, 0.0, low_slash()), kage_at(1.3, 0.0, jump_pose()), None, 0.4),
        17 => {
            // both leap apart
            let k = ease_out(u);
            (mio_at(lerp(-1.0, -4.5, k), 0.0, jump_pose()), kage_at(lerp(1.0, 4.5, k), 0.0, jump_pose()), None, 0.0)
        }
        18 => {
            // slow motion: crouch, then the last lunge
            let (m, k) = (g(&[(20.0, ready()), (21.2, ready()), (21.55, strike())], 0.35), g(&[(20.0, ready()), (21.2, ready()), (21.55, strike())], 0.35));
            let s = ease_in(clamp01((t - 21.2) / 0.4));
            (mio_at(lerp(-2.4, -0.6, s), 0.0, m), kage_at(lerp(2.4, 0.6, s), 0.0, k), None, 0.0)
        }
        _ => {
            // aftermath: they have passed each other
            let sway = (t * 2.0).sin() * 0.02;
            (
                Some(Place { pos: Vec3::new(1.0, 0.0, 0.0), yaw: MIO_YAW, pose: after_pose().with(&[(OX, sway), (OZ, 0.0), (HEAD, 0.0)]) }),
                Some(Place { pos: Vec3::new(-1.0, 0.0, 0.0), yaw: KAGE_YAW, pose: after_pose() }),
                None,
                0.0,
            )
        }
    }
}

fn build_scene() -> (Scene, usize, usize) {
    let mut s = Scene::empty(Camera::look_at(Vec3::new(0.0, 1.5, 8.0), Vec3::new(0.0, 1.2, 0.0)), Vec3::new(0.0, 1.2, 0.0));
    // skyline: towers around the arena
    for i in 0..26u32 {
        let a = i as f32 / 26.0 * 2.0 * PI + h(i) * 0.1;
        let r = 46.0 + 10.0 * h(i + 50);
        let (w, hh) = (2.5 + 4.0 * h(i + 100), 6.0 + 22.0 * h(i + 150) * h(i + 151));
        s.objects.push(Object { name: format!("tower{i}"), mesh: scene::cube(), model: Mat4::from_translation(Vec3::new(a.sin() * r, hh / 2.0, -a.cos() * r)) * Mat4::from_scale(Vec3::new(w, hh, w)), class: 2 });
    }
    // arena rim
    s.objects.push(Object { name: "rim".into(), mesh: scene::torus(16.0, 0.12, 64, 6), model: Mat4::from_translation(Vec3::new(0.0, 0.1, 0.0)) * Mat4::from_rotation_x(PI / 2.0), class: 0 });
    s.objects.push(Object { name: "rim2".into(), mesh: scene::torus(8.0, 0.06, 48, 6), model: Mat4::from_translation(Vec3::new(0.0, 0.05, 0.0)) * Mat4::from_rotation_x(PI / 2.0), class: 2 });
    let a = chars::add_fighter(&mut s, Who::Mio);
    let b = chars::add_fighter(&mut s, Who::Kage);
    (s, a, b)
}

fn sparks(cells: &mut [Cell], cam: &Camera, at: Vec3, k: f32, seed: u32) {
    let Some((cx, cy, _)) = project(cam, Vec4::new(at.x, at.y, at.z, 1.0)) else { return };
    for i in 0..30u32 {
        let ang = h(seed + i * 5) * 2.0 * PI;
        let len = (3.0 + 16.0 * h(seed + i * 5 + 1)) * (0.3 + k);
        let (dx, dy) = (ang.cos(), ang.sin());
        let ch = crate::film::line_char(dx, dy);
        let mut r = 1.0;
        while r < len {
            let (x, y) = ((cx + dx * r).floor() as i32, (cy + dy * r / 2.0).floor() as i32);
            let f = r / len;
            put(cells, x, y, ch, mix3(WHITE, ORANGE, f), f < 0.5);
            r += 0.6;
        }
    }
}

fn eye_cel(who: Who, variant: u8, t: f32, tl: f32) -> (Vec<Cell>, Eye) {
    let mut cells = vec![BLANK; COLS * ROWS];
    let mio = who == Who::Mio;
    let col: Col = if mio { CYAN } else { ORANGE };
    // blink at local time 1.0 (variant 0), eye narrows when the beat hits
    let blink = if variant == 0 { 1.0 - 1.0 * smooth(1.0 - ((tl - 1.05).abs() / 0.12).min(1.0)) } else { 1.0 };
    let wake = if variant == 0 { smooth(tl / 0.6) } else { 1.0 };
    let (look, open_max) = match (mio, variant) {
        (true, 0) => ((lerp(-0.7, 0.5, smooth((tl - 0.4) / 0.5)), 0.0), 0.95),
        (true, 1) => ((0.6, 0.0), 1.0),
        (true, _) => ((0.0, -0.1), 1.15),
        (false, 0) => ((lerp(0.6, -0.4, smooth((tl - 0.5) / 0.4)), 0.0), 0.62),
        (false, 1) => ((-0.7, 0.0), 0.5),
        (false, _) => ((0.0, -0.05), 0.85),
    };
    let e = Eye {
        cx: COLS as f32 / 2.0 + if mio { -2.0 } else { 2.0 },
        cy: 22.0,
        w: (if variant == 2 { 62.0 } else { 54.0 }) * (1.0 + 0.05 * tl),
        hgt: (if variant == 2 { 34.0 } else { 29.0 }) * (1.0 + 0.05 * tl),
        open: open_max * blink * wake,
        look,
        col,
        flip: !mio,
        brow: if mio { -0.15 } else { 0.45 },
        iris: if variant == 2 { 0.7 } else if mio { 0.95 } else { 0.8 },
        glint: if variant == 0 && tl < 0.6 { 0.0 } else { 1.0 },
    };
    cel::draw_eye(&mut cells, &e);
    // thin speed hatch behind the eye, drifting, in the eye colour
    for i in 0..26u32 {
        let y = (h(i * 3) * ROWS as f32) as i32;
        let len = 6 + (h(i * 3 + 1) * 30.0) as i32;
        let x0 = ((h(i * 3 + 2) * (COLS as f32 + 30.0) + t * (20.0 + 30.0 * h(i))) % (COLS as f32 + 40.0)) as i32 - 20;
        for k in 0..len {
            let (x, yy) = (x0 + k, y);
            if x >= 0 && yy >= 0 && (x as usize) < COLS && (yy as usize) < ROWS && cells[yy as usize * COLS + x as usize].ch == ' ' && !(10..=34).contains(&yy) {
                put(&mut cells, x, yy, '─', scale3(col, 0.18 + 0.2 * h(i + 8)), false);
            }
        }
    }
    (cells, e)
}

fn paper_amount(f: usize) -> f32 {
    let t = f as f32 / FPS as f32;
    IMPACTS.iter().any(|&(a, d)| t >= a && t < a + d) as u8 as f32
}

fn white_amount(t: f32) -> f32 {
    if t < 21.5 {
        0.0
    } else if t < 22.0 {
        ((t - 21.5) / 0.5).powi(2)
    } else if t < 22.45 {
        1.0 - (t - 22.0) / 0.45
    } else if (24.0..24.1).contains(&t) {
        1.0 - (t - 24.0) / 0.1
    } else if (24.5..24.58).contains(&t) {
        1.0 - (t - 24.5) / 0.08
    } else {
        0.0
    }
}

fn fade_amount(t: f32) -> f32 {
    if t < 0.3 {
        1.0 - t / 0.3
    } else if t > 29.3 {
        (t - 29.3) / 0.7
    } else {
        0.0
    }
}

pub fn prerender(atlas: &Atlas, opts: &Options) -> Vec<FilmFrame> {
    let total = (SECS * FPS as f32) as usize;
    let mut frames = Vec::with_capacity(total);
    let (mut scene, fm, fk) = build_scene();
    let mut renderer = Renderer::default();
    let mut cur = usize::MAX;
    let mut base = vec![BLANK; COLS * ROWS];
    for f in 0..total {
        let t = f as f32 / FPS as f32;
        let ci = CUTS.iter().rposition(|c| t >= c.0).unwrap_or(0);
        if ci != cur {
            cur = ci;
            renderer = Renderer::default();
        }
        let (c0, _, step, kind) = CUTS[ci];
        let f0 = (c0 * FPS as f32).round() as usize;
        let fq = f0 + (f - f0) / step * step;
        let paper = paper_amount(f) > 0.0;
        if fq == f {
            let tq = fq as f32 / FPS as f32;
            let tl = tq - c0;
            base = vec![BLANK; COLS * ROWS];
            match kind {
                Kind::Void => {
                    let vc = cam(Vec3::new(0.0, 1.3, lerp(11.0, 8.5, tq / 2.0)), Vec3::new(0.0, 1.4, 0.0), 45.0);
                    cel::grid(&mut base, &vc, scale3(CYAN, 0.22), 0.3);
                    let k = ease_out(clamp01(tq / 1.3));
                    let n = (COLS as f32 * k) as i32;
                    for i in 0..n {
                        let x = COLS as i32 / 2 - n / 2 + i;
                        put(&mut base, x, 24, '─', scale3(CYAN, 0.35 + 0.65 * (1.0 - (i as f32 / n as f32 * 2.0 - 1.0).abs())), true);
                    }
                    if tq > 1.3 {
                        let m = ((tq - 1.3) * 30.0).min(5.0) as i32;
                        for j in -m..=m {
                            put(&mut base, COLS as i32 / 2, 24 + j, '│', scale3(CYAN, 1.0 - j.abs() as f32 / 6.0), true);
                        }
                    }
                    cel::embers(&mut base, tq, 60, CYAN, WHITE, 0.5);
                }
                Kind::EyeM(v) | Kind::EyeK(v) => {
                    let who = if matches!(kind, Kind::EyeM(_)) { Who::Mio } else { Who::Kage };
                    let (cells, e) = eye_cel(who, v, tq, tl);
                    base = cells;
                    let beat = (tl * 2.0).fract();
                    if v >= 1 || tl > 1.4 {
                        cel::eye_shine(&mut base, &e, if v == 2 { clamp01(1.0 - tl * 3.0) } else { (1.0 - beat * 4.0).max(0.0) * 0.0 + clamp01(1.0 - (tl - 1.5).abs() * 5.0) });
                    }
                    if v == 2 {
                        speed_lines(&mut base, fq as u32, 40, 0.4, if who == Who::Mio { CYAN } else { ORANGE }, 0.8);
                    }
                    cel::embers(&mut base, tq, 24, if who == Who::Mio { CYAN } else { ORANGE }, WHITE, 1.0);
                }
                Kind::Shot(id) => {
                    scene.light_dir = Vec3::new(0.4, 0.8, 0.5).normalize();
                    let (m, k, sp, speed) = fighters(id, tq);
                    for (pl, first) in [(&m, fm), (&k, fk)] {
                        if pl.is_none() {
                            chars::hide(&mut scene, first);
                        }
                    }
                    let pm = m.as_ref().map(|p| chars::place(&mut scene, fm, Who::Mio, p.pos, p.yaw, &p.pose, tq));
                    let pk = k.as_ref().map(|p| chars::place(&mut scene, fk, Who::Kage, p.pos, p.yaw, &p.pose, tq));
                    let camera = view(id, tq, fq, &pm, &pk);
                    let frame = renderer.render(&scene, &camera, atlas, opts, COLS, ROWS);
                    // backdrop: moon, grid, then drop everything the geometry covers
                    let mut back = vec![BLANK; COLS * ROWS];
                    let moon_dir = Vec3::new(-0.12, 0.34, -1.0).normalize();
                    cel::moon(&mut back, &camera, moon_dir, 11.0, [190.0, 215.0, 255.0]);
                    let gk = if matches!(id, 3 | 4 | 8 | 12 | 14 | 16) { 0.0 } else { 0.6 };
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
                    if let (Some(a), Some(b), Some(kk)) = (&pm, &pk, sp) {
                        sparks(&mut base, &camera, (a.tip + b.tip) * 0.5, kk, fq as u32 * 17);
                    }
                    let close = matches!(id, 3 | 4 | 8 | 11 | 12 | 14 | 15 | 16);
                    if speed > 0.0 {
                        speed_lines(&mut base, fq as u32, if close { 16 } else { 50 }, 0.32 + 0.1 * h(fq as u32), [200.0, 225.0, 255.0], speed);
                    }
                    cel::embers(&mut base, tq, if close { 10 } else { 40 }, ORANGE, CYAN, 1.0);
                    if paper {
                        speed_lines(&mut base, fq as u32 + 3, if close { 35 } else { 90 }, 0.1, WHITE, 1.0);
                    }
                    if id == 19 {
                        let slope = -0.55;
                        let sh = clamp01((tq - 23.0) / 0.5);
                        if sh > 0.0 {
                            base = cel::split(&base, slope, ease_out(sh) * 14.0);
                            cel::embers(&mut base, tq, 60, WHITE, ORANGE, 2.0);
                        }
                        let k = clamp01((tq - 22.5) / 0.4);
                        if k > 0.0 {
                            cel::slash(&mut base, slope, k, WHITE);
                        }
                    }
                }
                Kind::Title => {
                    let camera = cam(Vec3::new(0.0, lerp(1.0, 1.5, tq / 30.0), lerp(9.0, 7.0, (tq - 24.0) / 6.0)), Vec3::new(0.0, 1.8, 0.0), 55.0);
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
    for tx in TEXTS.iter() {
        cel::draw_txt(&mut cells, tx, t);
    }
    if t >= 26.6 {
        cel::small_text(&mut cells, "AN ASCII ANIME", 38, WHITE, smooth((t - 26.6) / 0.3));
    }
    if t >= 28.0 {
        let blink = if t < 28.6 { ((t * 12.0) as i32 % 2) as f32 } else { 1.0 };
        cel::small_text(&mut cells, "COMING SOON", 40, CYAN, smooth((t - 28.0) / 0.3) * (0.4 + 0.6 * blink));
    }
    let p = if paper { 1.0 } else { 0.0 };
    let w = white_amount(t);
    let fd = fade_amount(t);
    for c in cells.iter_mut() {
        if c.ch != ' ' {
            let col = mix3([c.fg.0 as f32, c.fg.1 as f32, c.fg.2 as f32], INK, p);
            c.fg = rgb(scale3(mix3(col, [255.0; 3], w), 1.0 - fd));
        }
    }
    let bg = mix3(mix3([0.0; 3], PAPER, p), [255.0; 3], w);
    FilmFrame { cells, bg: rgb(scale3(bg, 1.0 - fd)) }
}
