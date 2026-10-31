//! Scene: procedural meshes, the `chapel` and `shapes` scenes, an OBJ loader and the camera.
//!
//! Conventions: right-handed, +y up, camera looks down -z at yaw 0. Closed meshes have counter-clockwise
//! (outward) winding, so a triangle is front-facing when its normal points at the camera.
//! Terminal cells are about 1 wide : 2 high; `Camera::aspect` accounts for that so a cube looks square.
use glam::{Mat3, Mat4, Vec3};
use std::collections::HashMap;

/// Triangle mesh in object space.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub verts: Vec<Vec3>,
    /// Triangles as vertex indices, counter-clockwise seen from outside.
    pub tris: Vec<[u32; 3]>,
    /// Optional per-vertex smooth normals (object space). Empty = flat shading from the face normal.
    pub normals: Vec<Vec3>,
}

impl Mesh {
    /// Unnormalised face normal (object space).
    pub fn face_normal(&self, t: usize) -> Vec3 {
        let [a, b, c] = self.tris[t];
        let (a, b, c) = (self.verts[a as usize], self.verts[b as usize], self.verts[c as usize]);
        (b - a).cross(c - a)
    }

    /// Signed volume (positive when winding is outward on a closed mesh).
    pub fn signed_volume(&self) -> f32 {
        (0..self.tris.len())
            .map(|t| {
                let [a, b, c] = self.tris[t];
                self.verts[a as usize].dot(self.verts[b as usize].cross(self.verts[c as usize])) / 6.0
            })
            .sum()
    }

    fn flip_all(&mut self) {
        for t in &mut self.tris {
            t.swap(1, 2);
        }
    }

    /// For convex shapes: flip each triangle so it faces away from the centroid.
    fn fix_winding_convex(&mut self) {
        let c = self.verts.iter().copied().sum::<Vec3>() / self.verts.len() as f32;
        for t in 0..self.tris.len() {
            let [a, b, d] = self.tris[t];
            let mid = (self.verts[a as usize] + self.verts[b as usize] + self.verts[d as usize]) / 3.0;
            if self.face_normal(t).dot(mid - c) < 0.0 {
                self.tris[t].swap(1, 2);
            }
        }
    }

    /// Area-weighted average of face normals at shared vertices.
    pub fn smooth_normals(&mut self) {
        let mut n = vec![Vec3::ZERO; self.verts.len()];
        for t in 0..self.tris.len() {
            let fnrm = self.face_normal(t);
            for &i in &self.tris[t] {
                n[i as usize] += fnrm;
            }
        }
        self.normals = n.into_iter().map(|v| v.normalize_or_zero()).collect();
    }

    pub fn bounds(&self) -> (Vec3, Vec3) {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for v in &self.verts {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
        (lo, hi)
    }
}

/// Unit cube centred at the origin (side 1). 8 shared vertices, 12 triangles.
pub fn cube() -> Mesh {
    let verts = (0..8)
        .map(|i| Vec3::new(if i & 1 == 0 { -0.5 } else { 0.5 }, if i & 2 == 0 { -0.5 } else { 0.5 }, if i & 4 == 0 { -0.5 } else { 0.5 }))
        .collect();
    let quads = [[0, 1, 3, 2], [4, 5, 7, 6], [0, 1, 5, 4], [2, 3, 7, 6], [0, 2, 6, 4], [1, 3, 7, 5]];
    let mut tris = vec![];
    for q in quads {
        tris.push([q[0], q[1], q[2]]);
        tris.push([q[0], q[2], q[3]]);
    }
    let mut m = Mesh { verts, tris, normals: vec![] };
    m.fix_winding_convex();
    m
}

/// Unit-radius icosphere with smooth normals.
pub fn icosphere(subdiv: u32) -> Mesh {
    let t = (1.0 + 5f32.sqrt()) / 2.0;
    let mut verts: Vec<Vec3> = [
        (-1., t, 0.), (1., t, 0.), (-1., -t, 0.), (1., -t, 0.), (0., -1., t), (0., 1., t),
        (0., -1., -t), (0., 1., -t), (t, 0., -1.), (t, 0., 1.), (-t, 0., -1.), (-t, 0., 1.),
    ]
    .iter()
    .map(|&(x, y, z)| Vec3::new(x, y, z).normalize())
    .collect();
    let mut tris: Vec<[u32; 3]> = vec![
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11], [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9], [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ];
    for _ in 0..subdiv {
        let mut cache: HashMap<(u32, u32), u32> = HashMap::new();
        let mut next = vec![];
        let mut mid = |a: u32, b: u32, verts: &mut Vec<Vec3>| -> u32 {
            let key = (a.min(b), a.max(b));
            *cache.entry(key).or_insert_with(|| {
                verts.push(((verts[a as usize] + verts[b as usize]) * 0.5).normalize());
                verts.len() as u32 - 1
            })
        };
        for [a, b, c] in tris {
            let (ab, bc, ca) = (mid(a, b, &mut verts), mid(b, c, &mut verts), mid(c, a, &mut verts));
            next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        tris = next;
    }
    let normals = verts.clone();
    let mut m = Mesh { verts, tris, normals };
    m.fix_winding_convex();
    m
}

/// Torus arc in the xy plane (axis +z). `arc` = sweep angle (2*PI = full torus, PI = arch).
/// `major` = ring radius, `minor` = tube radius.
pub fn torus_arc(major: f32, minor: f32, arc: f32, nu: usize, nv: usize) -> Mesh {
    let full = (arc - std::f32::consts::TAU).abs() < 1e-4;
    let rings = if full { nu } else { nu + 1 };
    let mut verts = vec![];
    for i in 0..rings {
        let u = arc * i as f32 / nu as f32;
        let (s, c) = u.sin_cos();
        for j in 0..nv {
            let v = std::f32::consts::TAU * j as f32 / nv as f32;
            let (sv, cv) = v.sin_cos();
            let radial = Vec3::new(c, s, 0.0);
            verts.push(radial * (major + minor * cv) + Vec3::Z * (minor * sv));
        }
    }
    let mut tris = vec![];
    for i in 0..nu {
        let i1 = (i + 1) % rings;
        for j in 0..nv {
            let j1 = (j + 1) % nv;
            let (a, b, c, d) = ((i * nv + j) as u32, (i1 * nv + j) as u32, (i1 * nv + j1) as u32, (i * nv + j1) as u32);
            tris.push([a, b, c]);
            tris.push([a, c, d]);
        }
    }
    let mut m = Mesh { verts, tris, normals: vec![] };
    // Orient outward: the first triangle's normal must point away from its tube centre.
    let [a, b, c] = m.tris[0];
    let mid = (m.verts[a as usize] + m.verts[b as usize] + m.verts[c as usize]) / 3.0;
    let ring_c = Vec3::new(mid.x, mid.y, 0.0).normalize_or_zero() * major;
    if m.face_normal(0).dot(mid - ring_c) < 0.0 {
        m.flip_all();
    }
    m.smooth_normals();
    m
}

pub fn torus(major: f32, minor: f32, nu: usize, nv: usize) -> Mesh {
    torus_arc(major, minor, std::f32::consts::TAU, nu, nv)
}

/// Arch = half torus in the xy plane, spanning x in [-major, major], top at y = major.
pub fn arch(major: f32, minor: f32, nu: usize, nv: usize) -> Mesh {
    torus_arc(major, minor, std::f32::consts::PI, nu, nv)
}

/// Stone rib: a rectangular cross-section (`depth` radial, `width` along z) swept along a half circle of
/// radius `major` in the xy plane, `n` segments, closed ends. Segments are small, so only the four long
/// edges and the two end caps are feature edges.
pub fn arch_rib(major: f32, depth: f32, width: f32, n: usize) -> Mesh {
    let (hd, hw) = (depth * 0.5, width * 0.5);
    let corners = [(-hd, -hw), (hd, -hw), (hd, hw), (-hd, hw)];
    let mut verts = vec![];
    for i in 0..=n {
        let (s, c) = (std::f32::consts::PI * i as f32 / n as f32).sin_cos();
        for (dr, z) in corners {
            verts.push(Vec3::new(c, s, 0.0) * (major + dr) + Vec3::Z * z);
        }
    }
    let mut tris = vec![];
    for i in 0..n {
        for k in 0..4 {
            let (a, b) = ((i * 4 + k) as u32, (i * 4 + (k + 1) % 4) as u32);
            let (c, d) = (a + 4, b + 4);
            tris.push([a, b, d]);
            tris.push([a, d, c]);
        }
    }
    for base in [0u32, (n * 4) as u32] {
        tris.push([base, base + 1, base + 2]);
        tris.push([base, base + 2, base + 3]);
    }
    let mut m = Mesh { verts, tris, normals: vec![] };
    // Side k = 0 lies on the z = -width/2 face, so its outward normal has z < 0. Caps both face -y.
    let nt = m.tris.len();
    if m.face_normal(0).z > 0.0 {
        for t in 0..nt - 4 {
            m.tris[t].swap(1, 2);
        }
    }
    for t in nt - 4..nt {
        if m.face_normal(t).y > 0.0 {
            m.tris[t].swap(1, 2);
        }
    }
    m
}

/// Cylinder along +y from 0 to `h`. Smooth side normals, flat caps (separate vertices).
pub fn cylinder(r: f32, h: f32, n: usize) -> Mesh {
    frustum(r, r, h, n)
}

/// Tapered cylinder along +y from 0 (radius `r0`) to `h` (radius `r1`). Use a tiny `r1` for a cone.
pub fn frustum(r0: f32, r1: f32, h: f32, n: usize) -> Mesh {
    let mut verts = vec![];
    for i in 0..n {
        let a = std::f32::consts::TAU * i as f32 / n as f32;
        verts.push(Vec3::new(r0 * a.cos(), 0.0, r0 * a.sin()));
    }
    for i in 0..n {
        let p = verts[i];
        verts.push(Vec3::new(p.x * r1 / r0, h, p.z * r1 / r0));
    }
    let mut tris = vec![];
    for i in 0..n {
        let j = (i + 1) % n;
        let (a, b, c, d) = (i as u32, j as u32, (n + j) as u32, (n + i) as u32);
        tris.push([a, b, c]);
        tris.push([a, c, d]);
    }
    for (y, base) in [(0.0, 2 * n), (h, 3 * n + 1)] {
        let center = base as u32;
        verts.push(Vec3::new(0.0, y, 0.0));
        let k = if y == 0.0 { 1.0 } else { r1 / r0 };
        for i in 0..n {
            let p = verts[i];
            verts.push(Vec3::new(p.x * k, y, p.z * k));
        }
        for i in 0..n {
            let (a, b) = (center + 1 + i as u32, center + 1 + ((i + 1) % n) as u32);
            tris.push([center, a, b]);
        }
    }
    let mut m = Mesh { verts, tris, normals: vec![] };
    m.fix_winding_convex();
    m.smooth_normals();
    m
}

/// Plane in xz centred at the origin, normal +y, `n` x `n` quads.
pub fn plane(width: f32, depth: f32, n: usize) -> Mesh {
    let mut verts = vec![];
    for i in 0..=n {
        for j in 0..=n {
            verts.push(Vec3::new((i as f32 / n as f32 - 0.5) * width, 0.0, (j as f32 / n as f32 - 0.5) * depth));
        }
    }
    let mut tris = vec![];
    for i in 0..n {
        for j in 0..n {
            let a = (i * (n + 1) + j) as u32;
            let (b, c, d) = (a + 1, a + (n + 1) as u32, a + (n + 1) as u32 + 1);
            tris.push([a, b, c]);
            tris.push([b, d, c]);
        }
    }
    Mesh { verts, tris, normals: vec![] }
}

/// Parse a Wavefront OBJ (only `v` and `f`; `f` may use v/vt/vn and negative indices; polygons are fanned).
pub fn parse_obj(src: &str) -> Result<Mesh, String> {
    let mut m = Mesh::default();
    for (ln, line) in src.lines().enumerate() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let v: Vec<f32> = it.take(3).map(|s| s.parse::<f32>().map_err(|e| format!("line {}: {e}", ln + 1))).collect::<Result<_, _>>()?;
                if v.len() < 3 {
                    return Err(format!("line {}: short vertex", ln + 1));
                }
                m.verts.push(Vec3::new(v[0], v[1], v[2]));
            }
            Some("f") => {
                let idx: Vec<u32> = it
                    .map(|s| {
                        let i: i64 = s.split('/').next().unwrap_or("").parse().map_err(|e| format!("line {}: {e}", ln + 1))?;
                        let n = m.verts.len() as i64;
                        let r = if i < 0 { n + i } else { i - 1 };
                        if r < 0 || r >= n { Err(format!("line {}: index out of range", ln + 1)) } else { Ok(r as u32) }
                    })
                    .collect::<Result<_, _>>()?;
                for k in 1..idx.len().saturating_sub(1) {
                    m.tris.push([idx[0], idx[k], idx[k + 1]]);
                }
            }
            _ => {}
        }
    }
    if m.tris.is_empty() {
        return Err("no faces in OBJ".into());
    }
    Ok(m)
}

/// A mesh instance with a transform.
#[derive(Clone, Debug)]
pub struct Object {
    pub name: String,
    pub mesh: Mesh,
    /// Object space -> world space.
    pub model: Mat4,
    /// Colour class for the tron palette: 0 = cyan, 1 = orange.
    pub class: u8,
}

impl Object {
    /// Matrix for transforming object-space normals to world space.
    pub fn normal_matrix(&self) -> Mat3 {
        Mat3::from_mat4(self.model).inverse().transpose()
    }
}

/// Camera: position plus yaw/pitch. Yaw 0 looks down -z.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// Vertical field of view, radians.
    pub fov_y: f32,
}

impl Camera {
    pub const NEAR: f32 = 0.05;
    pub const FAR: f32 = 300.0;

    pub fn look_at(pos: Vec3, target: Vec3) -> Camera {
        let d = (target - pos).normalize();
        Camera { pos, yaw: (-d.x).atan2(-d.z), pitch: d.y.asin(), fov_y: 60f32.to_radians() }
    }
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(-sy * cp, sp, -cy * cp)
    }
    /// Horizontal forward (ignores pitch), for walking.
    pub fn forward_flat(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(-sy, 0.0, -cy)
    }
    pub fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Y).normalize()
    }
    pub fn view(&self) -> Mat4 {
        let f = self.forward();
        let r = f.cross(Vec3::Y).normalize();
        let u = r.cross(f);
        Mat4::from_cols(
            glam::Vec4::new(r.x, u.x, -f.x, 0.0),
            glam::Vec4::new(r.y, u.y, -f.y, 0.0),
            glam::Vec4::new(r.z, u.z, -f.z, 0.0),
            glam::Vec4::new(-r.dot(self.pos), -u.dot(self.pos), f.dot(self.pos), 1.0),
        )
    }
    /// Physical width/height of the image: a cell is 1 wide : 2 high, so `cols / (2 * rows)`.
    pub fn aspect(cols: usize, rows: usize) -> f32 {
        cols as f32 / (2.0 * rows as f32)
    }
    pub fn proj(&self, cols: usize, rows: usize) -> Mat4 {
        // Right-handed perspective; clip.w = view-space distance along the view axis.
        let t = (self.fov_y * 0.5).tan();
        let (n, f) = (Self::NEAR, Self::FAR);
        Mat4::from_cols(
            glam::Vec4::new(1.0 / (t * Self::aspect(cols, rows)), 0.0, 0.0, 0.0),
            glam::Vec4::new(0.0, 1.0 / t, 0.0, 0.0),
            glam::Vec4::new(0.0, 0.0, -(f + n) / (f - n), -1.0),
            glam::Vec4::new(0.0, 0.0, -2.0 * f * n / (f - n), 0.0),
        )
    }
}

/// Scene = objects + light + a start camera + an orbit target for the bench camera path.
#[derive(Clone, Debug)]
pub struct Scene {
    pub objects: Vec<Object>,
    /// Direction the light travels FROM the surface TO the light (unit).
    pub light_dir: Vec3,
    pub cam0: Camera,
    pub orbit_target: Vec3,
}

impl Scene {
    fn add(&mut self, name: &str, mesh: Mesh, model: Mat4, class: u8) {
        self.objects.push(Object { name: name.into(), mesh, model, class });
    }

    pub fn empty(cam0: Camera, orbit_target: Vec3) -> Scene {
        Scene { objects: vec![], light_dir: Vec3::new(0.4, 0.8, 0.5).normalize(), cam0, orbit_target }
    }

    pub fn by_name(name: &str) -> Option<Scene> {
        match name {
            "chapel" => Some(Self::chapel()),
            "shapes" => Some(Self::shapes()),
            _ => None,
        }
    }

    /// Floor, two rows of columns (with bases and capitals), arches across the nave, an altar and a sphere.
    pub fn chapel() -> Scene {
        let mut s = Scene::empty(Camera::look_at(Vec3::new(0.0, 1.8, 3.0), Vec3::new(0.0, 2.0, -12.0)), Vec3::new(0.0, 1.8, -4.0));
        s.add("floor", plane(24.0, 44.0, 1), Mat4::from_translation(Vec3::new(0.0, 0.0, -10.0)), 0);
        let col = cylinder(0.35, 4.0, 16);
        let boxm = cube();
        let slab = cylinder(0.5, 0.3, 16);
        let arch_m = arch_rib(3.0, 0.3, 0.4, 32);
        for k in 0..4 {
            let z = -2.0 - 4.0 * k as f32;
            for side in [-3.0f32, 3.0] {
                s.add("column", col.clone(), Mat4::from_translation(Vec3::new(side, 0.0, z)), 0);
                s.add("base", slab.clone(), Mat4::from_translation(Vec3::new(side, 0.0, z)) * Mat4::from_scale(Vec3::new(1.0, 1.0, 1.0)), 0);
                s.add("capital", slab.clone(), Mat4::from_translation(Vec3::new(side, 3.87, z)) * Mat4::from_scale(Vec3::new(1.1, 1.0, 1.1)), 0);
            }
            s.add("arch", arch_m.clone(), Mat4::from_translation(Vec3::new(0.0, 4.0, z)), 1);
        }
        s.add("altar", boxm.clone(), Mat4::from_translation(Vec3::new(0.0, 0.5, -17.0)) * Mat4::from_scale(Vec3::new(2.0, 1.0, 1.0)), 1);
        s.add("orb", icosphere(3), Mat4::from_translation(Vec3::new(0.0, 1.55, -17.0)) * Mat4::from_scale(Vec3::splat(0.5)), 1);
        s
    }

    /// Cube, sphere and torus on a floor.
    pub fn shapes() -> Scene {
        let mut s = Scene::empty(Camera::look_at(Vec3::new(0.0, 2.2, 4.8), Vec3::new(0.0, 0.5, 0.0)), Vec3::new(0.0, 0.5, 0.0));
        s.add("floor", plane(14.0, 14.0, 1), Mat4::IDENTITY, 0);
        s.add("cube", cube(), Mat4::from_translation(Vec3::new(-2.3, 0.6, 0.0)) * Mat4::from_scale(Vec3::splat(1.2)), 1);
        s.add("sphere", icosphere(3), Mat4::from_translation(Vec3::new(0.0, 0.8, 0.0)) * Mat4::from_scale(Vec3::splat(0.8)), 0);
        s.add("torus", torus(0.7, 0.28, 32, 20), Mat4::from_translation(Vec3::new(2.4, 0.28, 0.0)) * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2), 1);
        s
    }

    /// A single OBJ mesh, centred and scaled to about 2.5 units, with flat shading.
    pub fn from_obj(mesh: Mesh) -> Scene {
        let (lo, hi) = mesh.bounds();
        let c = (lo + hi) * 0.5;
        let k = 2.5 / (hi - lo).max_element().max(1e-6);
        let mut s = Scene::empty(Camera::look_at(Vec3::new(0.0, 1.0, 5.0), Vec3::ZERO), Vec3::ZERO);
        s.add("obj", mesh, Mat4::from_scale(Vec3::splat(k)) * Mat4::from_translation(-c), 0);
        s
    }

    /// Bench camera path: slow orbit (0.25 deg/frame) about the orbit target plus a slow sideways drift.
    /// Frame 120 is the scene's start camera; the path covers +-30 degrees over 240 frames.
    pub fn bench_camera(&self, frame: usize) -> Camera {
        let k = frame as f32 - 120.0;
        let ang = (0.25 * k).to_radians();
        let rel = self.cam0.pos - self.orbit_target;
        let (s, c) = ang.sin_cos();
        let rel = Vec3::new(rel.x * c + rel.z * s, rel.y, -rel.x * s + rel.z * c);
        let mut cam = self.cam0;
        cam.pos = self.orbit_target + rel;
        cam.yaw = self.cam0.yaw + ang;
        cam.pos += cam.right() * (0.004 * k);
        cam
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obj_tiny() {
        let m = parse_obj("# c\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\nf -4/1/1 -3 -2\n").unwrap();
        assert_eq!(m.verts.len(), 4);
        assert_eq!(m.tris.len(), 3);
        assert_eq!(m.tris[1], [0, 2, 3]);
        assert!(parse_obj("v 0 0 0\nf 1 2 3\n").is_err());
    }

    #[test]
    fn closed_meshes_outward() {
        for m in [cube(), icosphere(2), cylinder(1.0, 2.0, 12), torus(1.0, 0.3, 16, 8), arch_rib(3.0, 0.3, 0.4, 32)] {
            assert!(m.signed_volume() > 0.0);
        }
    }
}
