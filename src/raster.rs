//! CPU rasteriser producing the G-buffer (DESIGN.md section 1). Triangles are binned into one band per cell row
//! and the bands are filled in parallel with rayon.
use crate::scene::{Camera, Scene};
use glam::{Mat4, Vec2, Vec3, Vec4};
use rayon::prelude::*;

/// Object id meaning "no object" (background).
pub const NO_OBJ: u16 = u16::MAX;

/// G-buffer: `w = cols * sx` by `h = rows * sy` pixels, row-major, y down. All per-pixel Vecs have `w * h` items.
/// Pixel `(x, y)` belongs to cell `(x / sx, y / sy)`.
pub struct GBuffer {
    /// Terminal columns / rows.
    pub cols: usize,
    pub rows: usize,
    /// Pixels per cell (SX wide, SY high). Default 4 x 8 gives square pixels.
    pub sx: usize,
    pub sy: usize,
    /// Pixel width / height of the buffer.
    pub w: usize,
    pub h: usize,
    /// Linear view-space depth (distance along the view axis). `INFINITY` where empty.
    pub depth: Vec<f32>,
    /// Object index into `Scene::objects`, or `NO_OBJ`.
    pub obj: Vec<u16>,
    /// Triangle index inside that object's mesh (valid when `obj != NO_OBJ`).
    pub tri: Vec<u32>,
    /// Object-space position of the surface point (zero when empty).
    pub pos: Vec<Vec3>,
    /// View-space unit normal, flipped to face the camera (zero when empty).
    pub normal: Vec<Vec3>,
    /// Light 0..1: Lambert from the scene's directional light + 0.15 ambient (0 when empty).
    pub light: Vec<f32>,
    /// Camera matrices used: world->view, view->clip, world->clip.
    pub view: Mat4,
    pub proj: Mat4,
    pub view_proj: Mat4,
    pub cam_pos: Vec3,
    /// World-space size of one pixel (x, y) at depth 1; multiply by depth for the size at that depth.
    pub pixel_size: Vec2,
}

impl GBuffer {
    pub fn idx(&self, x: usize, y: usize) -> usize {
        y * self.w + x
    }

    /// Project a world point to pixel coordinates `(x, y, linear depth)`; `None` if behind the near plane.
    pub fn project_world(&self, p: Vec3) -> Option<Vec3> {
        let c = self.view_proj * p.extend(1.0);
        if c.w < Camera::NEAR {
            return None;
        }
        Some(Vec3::new((c.x / c.w * 0.5 + 0.5) * self.w as f32, (0.5 - c.y / c.w * 0.5) * self.h as f32, c.w))
    }

    /// Project an object-space point of `scene.objects[obj]`.
    pub fn project_obj(&self, scene: &Scene, obj: usize, p: Vec3) -> Option<Vec3> {
        self.project_world(scene.objects[obj].model.transform_point3(p))
    }

    /// Mean light over the pixels of a cell that hit an object (0 if none).
    pub fn cell_light(&self, cx: usize, cy: usize) -> f32 {
        let (mut s, mut n) = (0.0, 0);
        for y in cy * self.sy..(cy + 1) * self.sy {
            for x in cx * self.sx..(cx + 1) * self.sx {
                let i = self.idx(x, y);
                if self.obj[i] != NO_OBJ {
                    s += self.light[i];
                    n += 1;
                }
            }
        }
        if n == 0 { 0.0 } else { s / n as f32 }
    }

    /// Fraction of a cell's pixels covered by objects.
    pub fn cell_coverage(&self, cx: usize, cy: usize) -> f32 {
        let mut n = 0;
        for y in cy * self.sy..(cy + 1) * self.sy {
            for x in cx * self.sx..(cx + 1) * self.sx {
                if self.obj[self.idx(x, y)] != NO_OBJ {
                    n += 1;
                }
            }
        }
        n as f32 / (self.sx * self.sy) as f32
    }
}

#[derive(Clone, Copy)]
struct Px {
    depth: f32,
    obj: u16,
    tri: u32,
    /// Barycentrics of the original (unclipped) triangle.
    bary: Vec3,
}

struct ScreenTri {
    x: [f32; 3],
    y: [f32; 3],
    invw: [f32; 3],
    bary: [Vec3; 3],
    obj: u16,
    tri: u32,
    area: f32,
}

#[derive(Clone, Copy)]
struct CV {
    clip: Vec4,
    bary: Vec3,
}

fn clip_near(poly: &[CV]) -> Vec<CV> {
    let near = Camera::NEAR;
    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (a.clip.w - near, b.clip.w - near);
        if da >= 0.0 {
            out.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            out.push(CV { clip: a.clip.lerp(b.clip, t), bary: a.bary.lerp(b.bary, t) });
        }
    }
    out
}

/// Rasterise `scene` seen from `cam` into a `cols x rows` cell grid with `sx x sy` pixels per cell.
pub fn rasterize(scene: &Scene, cam: &Camera, cols: usize, rows: usize, sx: usize, sy: usize) -> GBuffer {
    let (w, h) = (cols * sx, rows * sy);
    let view = cam.view();
    let proj = cam.proj(cols, rows);
    let view_proj = proj * view;
    let (wf, hf) = (w as f32, h as f32);

    // Triangle setup, parallel over objects.
    let per_obj: Vec<Vec<ScreenTri>> = scene
        .objects
        .par_iter()
        .enumerate()
        .map(|(oi, o)| {
            let mvp = view_proj * o.model;
            let clips: Vec<Vec4> = o.mesh.verts.iter().map(|v| mvp * v.extend(1.0)).collect();
            let mut out = vec![];
            for (ti, t) in o.mesh.tris.iter().enumerate() {
                let c = [clips[t[0] as usize], clips[t[1] as usize], clips[t[2] as usize]];
                if c.iter().all(|v| v.w < Camera::NEAR) {
                    continue;
                }
                let poly: Vec<CV> = (0..3).map(|k| CV { clip: c[k], bary: Vec3::from_array(std::array::from_fn(|j| (j == k) as u8 as f32)) }).collect();
                let poly = if c.iter().any(|v| v.w < Camera::NEAR) { clip_near(&poly) } else { poly };
                if poly.len() < 3 {
                    continue;
                }
                let sv: Vec<(f32, f32, f32)> =
                    poly.iter().map(|v| ((v.clip.x / v.clip.w * 0.5 + 0.5) * wf, (0.5 - v.clip.y / v.clip.w * 0.5) * hf, 1.0 / v.clip.w)).collect();
                for k in 1..poly.len() - 1 {
                    let (a, b, d) = (0, k, k + 1);
                    let area = (sv[b].0 - sv[a].0) * (sv[d].1 - sv[a].1) - (sv[b].1 - sv[a].1) * (sv[d].0 - sv[a].0);
                    if area.abs() < 1e-7 {
                        continue;
                    }
                    out.push(ScreenTri {
                        x: [sv[a].0, sv[b].0, sv[d].0],
                        y: [sv[a].1, sv[b].1, sv[d].1],
                        invw: [sv[a].2, sv[b].2, sv[d].2],
                        bary: [poly[a].bary, poly[b].bary, poly[d].bary],
                        obj: oi as u16,
                        tri: ti as u32,
                        area,
                    });
                }
            }
            out
        })
        .collect();
    let tris: Vec<ScreenTri> = per_obj.into_iter().flatten().collect();

    // Bin to bands (one per cell row).
    let mut bins: Vec<Vec<u32>> = vec![vec![]; rows];
    for (i, t) in tris.iter().enumerate() {
        let y0 = t.y.iter().cloned().fold(f32::INFINITY, f32::min);
        let y1 = t.y.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let x0 = t.x.iter().cloned().fold(f32::INFINITY, f32::min);
        let x1 = t.x.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        if y1 < 0.0 || y0 >= hf || x1 < 0.0 || x0 >= wf {
            continue;
        }
        let b0 = (y0.max(0.0) as usize) / sy;
        let b1 = ((y1.min(hf - 1.0)) as usize) / sy;
        for bin in &mut bins[b0..=b1.min(rows - 1)] {
            bin.push(i as u32);
        }
    }

    let mut pxs = vec![Px { depth: f32::INFINITY, obj: NO_OBJ, tri: 0, bary: Vec3::ZERO }; w * h];
    pxs.par_chunks_mut(w * sy).zip(bins.par_iter()).enumerate().for_each(|(band, (chunk, bin))| {
        let y_base = band * sy;
        for &ti in bin {
            let t = &tris[ti as usize];
            let x0 = t.x.iter().cloned().fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
            let x1 = (t.x.iter().cloned().fold(f32::NEG_INFINITY, f32::max).ceil().min(wf - 1.0)).max(0.0) as usize;
            let y0 = (t.y.iter().cloned().fold(f32::INFINITY, f32::min).floor().max(y_base as f32)) as usize;
            let y1 = (t.y.iter().cloned().fold(f32::NEG_INFINITY, f32::max).ceil().min((y_base + sy - 1) as f32)) as usize;
            let inv_area = 1.0 / t.area;
            for y in y0..=y1 {
                let py = y as f32 + 0.5;
                for x in x0..=x1 {
                    let pxc = x as f32 + 0.5;
                    let e = |a: usize, b: usize| ((t.x[b] - t.x[a]) * (py - t.y[a]) - (t.y[b] - t.y[a]) * (pxc - t.x[a])) * inv_area;
                    let (b0, b1, b2) = (e(1, 2), e(2, 0), e(0, 1));
                    // e(1,2) is the weight of vertex 0 etc. (signs already normalised by inv_area)
                    if b0 < -1e-6 || b1 < -1e-6 || b2 < -1e-6 {
                        continue;
                    }
                    let (l0, l1, l2) = (b0 * t.invw[0], b1 * t.invw[1], b2 * t.invw[2]);
                    let s = l0 + l1 + l2;
                    if s <= 0.0 {
                        continue;
                    }
                    let depth = 1.0 / s;
                    let p = &mut chunk[(y - y_base) * w + x];
                    if depth < p.depth {
                        let bary = (t.bary[0] * l0 + t.bary[1] * l1 + t.bary[2] * l2) * depth;
                        *p = Px { depth, obj: t.obj, tri: t.tri, bary };
                    }
                }
            }
        }
    });

    // Shading pass.
    let normal_mats: Vec<_> = scene.objects.iter().map(|o| o.normal_matrix()).collect();
    let view3 = glam::Mat3::from_mat4(view);
    let light_dir = scene.light_dir;
    let shaded: Vec<(Vec3, Vec3, f32)> = pxs
        .par_iter()
        .map(|p| {
            if p.obj == NO_OBJ {
                return (Vec3::ZERO, Vec3::ZERO, 0.0);
            }
            let o = &scene.objects[p.obj as usize];
            let tv = o.mesh.tris[p.tri as usize];
            let v = [o.mesh.verts[tv[0] as usize], o.mesh.verts[tv[1] as usize], o.mesh.verts[tv[2] as usize]];
            let pos = v[0] * p.bary.x + v[1] * p.bary.y + v[2] * p.bary.z;
            let n_obj = if o.mesh.normals.len() == o.mesh.verts.len() {
                let n = &o.mesh.normals;
                n[tv[0] as usize] * p.bary.x + n[tv[1] as usize] * p.bary.y + n[tv[2] as usize] * p.bary.z
            } else {
                (v[1] - v[0]).cross(v[2] - v[0])
            };
            let mut n = (normal_mats[p.obj as usize] * n_obj).normalize_or_zero();
            let wp = o.model.transform_point3(pos);
            if n.dot(cam.pos - wp) < 0.0 {
                n = -n;
            }
            let light = 0.15 + 0.85 * n.dot(light_dir).max(0.0);
            (pos, (view3 * n).normalize_or_zero(), light)
        })
        .collect();

    let tan_half = (cam.fov_y * 0.5).tan();
    let aspect = Camera::aspect(cols, rows);
    GBuffer {
        cols,
        rows,
        sx,
        sy,
        w,
        h,
        depth: pxs.par_iter().map(|p| p.depth).collect(),
        obj: pxs.par_iter().map(|p| p.obj).collect(),
        tri: pxs.par_iter().map(|p| p.tri).collect(),
        pos: shaded.par_iter().map(|s| s.0).collect(),
        normal: shaded.par_iter().map(|s| s.1).collect(),
        light: shaded.par_iter().map(|s| s.2).collect(),
        view,
        proj,
        view_proj,
        cam_pos: cam.pos,
        pixel_size: Vec2::new(2.0 * tan_half * aspect / w as f32, 2.0 * tan_half / h as f32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Mesh, Object, cube};

    #[test]
    fn cube_depth() {
        let obj = Object { name: "c".into(), mesh: cube(), model: Mat4::IDENTITY, class: 0 };
        let mut scene = Scene::shapes();
        scene.objects = vec![obj];
        let cam = Camera::look_at(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO);
        let gb = rasterize(&scene, &cam, 40, 20, 4, 8);
        let mid = gb.idx(gb.w / 2, gb.h / 2);
        assert_eq!(gb.obj[mid], 0);
        // front face of the unit cube is at z = 0.5, camera at 5 -> depth 4.5
        assert!((gb.depth[mid] - 4.5).abs() < 0.01, "depth {}", gb.depth[mid]);
        assert_eq!(gb.obj[0], NO_OBJ);
        assert!(gb.depth[0].is_infinite());
        // the face-on cube must look square: physical width = cols*1, height = rows*2.
        let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0, usize::MAX, 0);
        for y in 0..gb.h {
            for x in 0..gb.w {
                if gb.obj[gb.idx(x, y)] != NO_OBJ {
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                }
            }
        }
        let (pw, ph) = ((x1 - x0 + 1) as f32 / gb.sx as f32, 2.0 * (y1 - y0 + 1) as f32 / gb.sy as f32);
        assert!((pw / ph - 1.0).abs() < 0.05, "{pw} x {ph}");
        let _ = Mesh::default();
    }
}
