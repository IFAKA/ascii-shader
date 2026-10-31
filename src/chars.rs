//! Two swordfighters built from primitives. A `Pose` is a handful of angles; `place` turns it into object transforms.
use crate::film::{clamp01, ease_out, lerp};
use crate::scene::{self, Mesh, Object, Scene};
use glam::{Mat4, Quat, Vec3};

pub const HY: usize = 0; // hip height
pub const OX: usize = 1; // local offset right
pub const OZ: usize = 2; // local offset forward
pub const LEAN: usize = 3; // torso forward lean
pub const TWIST: usize = 4; // torso twist about up
pub const ARM_L: usize = 5; // swing, abduct, bend (x3)
pub const ARM_R: usize = 8;
pub const LEG_L: usize = 11;
pub const LEG_R: usize = 14;
pub const SW_P: usize = 17; // sword pitch (up +)
pub const SW_Q: usize = 18; // sword yaw (0 forward, + to the right)
pub const HEAD: usize = 19; // head turn
pub const N: usize = 20;

#[derive(Clone, Copy, Debug)]
pub struct Pose(pub [f32; N]);

impl Pose {
    pub fn lerp(&self, o: &Pose, t: f32) -> Pose {
        let mut r = [0.0; N];
        for (i, v) in r.iter_mut().enumerate() {
            *v = lerp(self.0[i], o.0[i], t);
        }
        Pose(r)
    }
    /// Change some values of this pose.
    pub fn with(mut self, set: &[(usize, f32)]) -> Pose {
        for &(i, v) in set {
            self.0[i] = v;
        }
        self
    }
    pub fn limb(mut self, at: usize, v: [f32; 3]) -> Pose {
        self.0[at..at + 3].copy_from_slice(&v);
        self
    }
}

pub fn stand() -> Pose {
    let mut p = [0.0; N];
    p[HY] = 0.98;
    Pose(p).limb(ARM_L, [0.05, 0.12, 0.2]).limb(ARM_R, [0.1, 0.14, 0.3]).limb(LEG_L, [0.03, 0.07, 0.05]).limb(LEG_R, [-0.03, 0.07, 0.05]).with(&[(SW_P, -1.35), (SW_Q, 0.1)])
}

/// Hold-and-snap: each key waits, then the pose snaps to the next one in `snap` seconds (anticipation, then a held drawing).
pub fn track(keys: &[(f32, Pose)], t: f32, snap: f32) -> Pose {
    let mut cur = keys[0].1;
    for w in keys.windows(2) {
        if t >= w[1].0 {
            cur = w[1].1;
        } else if t >= w[0].0 {
            let k = ease_out(clamp01((t - w[0].0) / snap));
            return w[0].1.lerp(&w[1].1, k);
        } else {
            break;
        }
    }
    cur
}

pub const PARTS: usize = 21;
const HIDDEN: Vec3 = Vec3::new(0.0, -500.0, 0.0);

#[derive(Clone, Copy, PartialEq)]
pub enum Who {
    Mio,
    Kage,
}

fn tapered(r0: f32, r1: f32) -> Mesh {
    scene::frustum(r0, r1, 1.0, 6)
}

/// Add the parts of one fighter to the scene; returns the index of the first part.
pub fn add_fighter(s: &mut Scene, who: Who) -> usize {
    let first = s.objects.len();
    let class = if who == Who::Mio { 0 } else { 1 };
    let (tw, th) = if who == Who::Mio { (0.16, 0.18) } else { (0.2, 0.22) };
    let meshes: Vec<(&str, Mesh, u8)> = vec![
        ("torso", tapered(tw * 0.8, th), class),
        ("head", scene::icosphere(1), class),
        ("uarm_l", tapered(0.052, 0.045), class),
        ("farm_l", tapered(0.045, 0.036), class),
        ("uarm_r", tapered(0.052, 0.045), class),
        ("farm_r", tapered(0.045, 0.036), class),
        ("thigh_l", tapered(0.085, 0.065), class),
        ("shin_l", tapered(0.062, 0.045), class),
        ("thigh_r", tapered(0.085, 0.065), class),
        ("shin_r", tapered(0.062, 0.045), class),
        ("blade", scene::cube(), 3),
        ("guard", scene::cube(), 3),
        ("hair0", tapered(0.07, 0.004), class),
        ("hair1", tapered(0.07, 0.004), class),
        ("hair2", tapered(0.07, 0.004), class),
        ("hair3", tapered(0.07, 0.004), class),
        ("hair4", tapered(0.07, 0.004), class),
        ("tail0", tapered(0.05, 0.004), class),
        ("tail1", tapered(0.05, 0.004), class),
        ("tail2", tapered(0.04, 0.004), class),
        ("pelvis", tapered(0.15, 0.16), class),
    ];
    for (n, m, c) in meshes {
        s.objects.push(Object { name: format!("{}_{}", if who == Who::Mio { "mio" } else { "kage" }, n), mesh: m, model: Mat4::from_translation(HIDDEN), class: c });
    }
    first
}

fn seg(a: Vec3, b: Vec3, thick: f32) -> Mat4 {
    let d = b - a;
    let len = d.length().max(0.001);
    Mat4::from_translation(a) * Mat4::from_quat(Quat::from_rotation_arc(Vec3::Y, d / len)) * Mat4::from_scale(Vec3::new(thick, len, thick))
}

/// Local direction of a limb segment: swing forward `a`, abduct `b` (`side` = +1 right, -1 left).
fn dir(a: f32, b: f32, side: f32) -> Vec3 {
    let (vy, vz) = (-a.cos(), a.sin());
    Vec3::new(-vy * b.sin() * side, vy * b.cos(), vz).normalize()
}

/// Everything the camera might want to know about a posed fighter.
pub struct Posed {
    pub hand_r: Vec3,
    pub head: Vec3,
    pub tip: Vec3,
}

/// Put the fighter at `pos` facing `yaw` (0 = looks down +z) in the pose. `t` sways the hair and coat.
pub fn place(s: &mut Scene, first: usize, who: Who, pos: Vec3, yaw: f32, p: &Pose, t: f32) -> Posed {
    let world = Mat4::from_translation(pos) * Mat4::from_rotation_y(yaw);
    let q = &p.0;
    let mio = who == Who::Mio;
    let hip = Vec3::new(q[OX], q[HY], q[OZ]);
    let torso_rot = Mat4::from_rotation_y(q[TWIST]) * Mat4::from_rotation_x(q[LEAN]);
    let tr = |v: Vec3| -> Vec3 { hip + torso_rot.transform_vector3(v) };
    let chest = tr(Vec3::new(0.0, 0.5, 0.0));
    let sh_w = if mio { 0.17 } else { 0.22 };
    let (sh_r, sh_l) = (tr(Vec3::new(sh_w, 0.46, 0.0)), tr(Vec3::new(-sh_w, 0.46, 0.0)));
    let head = tr(Vec3::new(0.0, 0.66, 0.02));
    let hip_w = 0.1;
    let (hp_r, hp_l) = (hip + Vec3::new(hip_w, -0.02, 0.0), hip + Vec3::new(-hip_w, -0.02, 0.0));
    let (ua, fa) = (0.29, 0.3);
    let (ul, ll) = (0.46, 0.46);
    let arm = |sh: Vec3, at: usize, side: f32| -> (Vec3, Vec3) {
        let (a, b, c) = (q[at], q[at + 1], q[at + 2]);
        let el = sh + dir(a, b, side) * ua;
        (el, el + dir(a + c, b * 0.5, side) * fa)
    };
    let leg = |hp: Vec3, at: usize, side: f32| -> (Vec3, Vec3) {
        let (a, b, c) = (q[at], q[at + 1], q[at + 2]);
        let kn = hp + dir(a, b, side) * ul;
        (kn, kn + dir(a - c, b * 0.4, side) * ll)
    };
    let (el_l, ha_l) = arm(sh_l, ARM_L, -1.0);
    let (el_r, ha_r) = arm(sh_r, ARM_R, 1.0);
    let (kn_l, ft_l) = leg(hp_l, LEG_L, -1.0);
    let (kn_r, ft_r) = leg(hp_r, LEG_R, 1.0);
    // The pose is authored for straight legs standing at hip height `HY`; keep the lower foot on the floor unless airborne.
    let lowest = ft_l.y.min(ft_r.y);
    let lift = if q[HY] > 1.25 { 0.0 } else { -lowest };
    let lift = Vec3::new(0.0, lift, 0.0);
    let sw_dir = Vec3::new(q[SW_Q].sin() * q[SW_P].cos(), q[SW_P].sin(), q[SW_Q].cos() * q[SW_P].cos()).normalize();
    let hilt = ha_r;
    let blade_len = if mio { 1.05 } else { 1.2 };
    let tip = hilt + sw_dir * blade_len;
    let g = |v: Vec3| world.transform_point3(v + lift);
    let m = |a: Vec3, b: Vec3, thick: f32| world * Mat4::from_translation(lift) * seg(a, b, thick);
    let set = |s: &mut Scene, i: usize, mat: Mat4| s.objects[first + i].model = mat;
    set(s, 0, m(hip + Vec3::new(0.0, 0.05, 0.0), chest, 1.0));
    let head_rot = torso_rot * Mat4::from_rotation_y(q[HEAD]);
    set(s, 1, world * Mat4::from_translation(lift + head) * Mat4::from_scale(Vec3::splat(if mio { 0.125 } else { 0.135 })));
    set(s, 2, m(sh_l, el_l, 1.0));
    set(s, 3, m(el_l, ha_l, 1.0));
    set(s, 4, m(sh_r, el_r, 1.0));
    set(s, 5, m(el_r, ha_r, 1.0));
    set(s, 6, m(hp_l, kn_l, 1.0));
    set(s, 7, m(kn_l, ft_l, 1.0));
    set(s, 8, m(hp_r, kn_r, 1.0));
    set(s, 9, m(kn_r, ft_r, 1.0));
    // sword: blade is a thin box along sw_dir, guard a small crossbar
    let up_ref = if sw_dir.y.abs() > 0.95 { Vec3::X } else { Vec3::Y };
    let side = sw_dir.cross(up_ref).normalize();
    let orient = Mat4::from_quat(Quat::from_mat3(&glam::Mat3::from_cols(side, sw_dir.cross(side).normalize(), sw_dir)));
    set(s, 10, world * Mat4::from_translation(lift + hilt + sw_dir * (blade_len * 0.5 + 0.05)) * orient * Mat4::from_scale(Vec3::new(0.018, 0.05, blade_len)));
    set(s, 11, world * Mat4::from_translation(lift + hilt + sw_dir * 0.06) * orient * Mat4::from_scale(Vec3::new(0.14, 0.03, 0.03)));
    // hair: spikes (Kage) or swept fringe + long ponytail (Mio)
    let sway = (t * 5.0).sin() * 0.12;
    let hd = |v: Vec3| lift + head + head_rot.transform_vector3(v);
    let hair: [(Vec3, Vec3, f32); 5] = if mio {
        [
            (Vec3::new(0.0, 0.1, -0.05), Vec3::new(0.0, 0.5, -1.0), 0.28),
            (Vec3::new(0.07, 0.08, 0.0), Vec3::new(0.7, 0.7, -0.4), 0.2),
            (Vec3::new(-0.07, 0.08, 0.0), Vec3::new(-0.7, 0.7, -0.4), 0.2),
            (Vec3::new(0.0, 0.12, 0.05), Vec3::new(0.0, 0.6, 0.9), 0.14),
            (Vec3::new(0.0, 0.04, -0.1), Vec3::new(sway, -0.15, -1.0), 0.5),
        ]
    } else {
        [
            (Vec3::new(0.0, 0.12, 0.0), Vec3::new(0.0, 1.0, -0.3), 0.3),
            (Vec3::new(0.08, 0.1, 0.0), Vec3::new(0.7, 0.8, -0.3), 0.26),
            (Vec3::new(-0.08, 0.1, 0.0), Vec3::new(-0.7, 0.8, -0.3), 0.26),
            (Vec3::new(0.05, 0.08, -0.07), Vec3::new(0.4, 0.4, -1.0), 0.3),
            (Vec3::new(-0.05, 0.08, -0.07), Vec3::new(-0.4, 0.4, -1.0), 0.3),
        ]
    };
    for (i, (o, d, l)) in hair.iter().enumerate() {
        let a = world.transform_point3(hd(*o));
        let dd = world.transform_vector3(head_rot.transform_vector3(d.normalize()));
        set(s, 12 + i, seg(a, a + dd * *l, 1.0));
    }
    // scarf / coat tails stream behind the shoulders
    let back = Vec3::new(0.0, 0.0, -1.0);
    let tails: [(f32, f32, f32); 3] = if mio { [(0.0, 0.5, 0.55), (0.09, 0.4, 0.45), (-0.09, 0.3, 0.4)] } else { [(0.0, 0.0, 0.9), (0.14, -0.1, 0.8), (-0.14, -0.1, 0.8)] };
    for (i, (ox, oy, l)) in tails.iter().enumerate() {
        let root = tr(Vec3::new(*ox, 0.44 + *oy * 0.1 - if mio { 0.0 } else { 0.1 }, -0.1));
        let d = (torso_rot.transform_vector3(back) + Vec3::new(ox * 2.0 + sway * 1.5, -0.3 + *oy * 0.3 - lean_drop(q[LEAN]), 0.0)).normalize();
        let a = world.transform_point3(lift + root);
        let dd = world.transform_vector3(d);
        set(s, 17 + i, seg(a, a + dd * *l, 1.0));
    }
    set(s, 20, m(hip + Vec3::new(0.0, -0.06, 0.0), hip + Vec3::new(0.0, 0.1, 0.0), 1.0));
    Posed { hand_r: g(hilt), head: g(head), tip: g(tip) }
}

fn lean_drop(l: f32) -> f32 {
    l * 0.4
}

pub fn hide(s: &mut Scene, first: usize) {
    for i in 0..PARTS {
        s.objects[first + i].model = Mat4::from_translation(HIDDEN);
    }
}
