//! Baselines (DESIGN.md section 5): `tone` ramp and `edge` (Sobel on depth + normals, dominant angle per cell).
use crate::glyphs::RAMP;
use crate::raster::{GBuffer, NO_OBJ};

/// Baseline output: one char per cell and whether it is an edge glyph (for colour).
pub struct BaselineOut {
    pub chars: Vec<char>,
    pub edge: Vec<bool>,
    pub obj: Vec<u16>,
}

fn tone_char(light: f32, cov: f32) -> char {
    if cov < 0.25 {
        return ' ';
    }
    let r: Vec<char> = RAMP.chars().collect();
    r[((light.clamp(0.0, 0.999)) * r.len() as f32) as usize]
}

fn cell_obj(gb: &GBuffer, cx: usize, cy: usize) -> u16 {
    let i = gb.idx(cx * gb.sx + gb.sx / 2, cy * gb.sy + gb.sy / 2);
    gb.obj[i]
}

/// Classic brightness ramp ` .:-=+*#%@` from mean light per cell.
pub fn tone(gb: &GBuffer) -> BaselineOut {
    let n = gb.cols * gb.rows;
    let mut out = BaselineOut { chars: vec![' '; n], edge: vec![false; n], obj: vec![NO_OBJ; n] };
    for cy in 0..gb.rows {
        for cx in 0..gb.cols {
            let i = cy * gb.cols + cx;
            out.chars[i] = tone_char(gb.cell_light(cx, cy), gb.cell_coverage(cx, cy));
            out.obj[i] = cell_obj(gb, cx, cy);
        }
    }
    out
}

/// Edge-angle baseline: Sobel on 1/depth and view normals, structure tensor per cell, angle -> `- / | \`.
pub fn edge(gb: &GBuffer) -> BaselineOut {
    let (w, h) = (gb.w, gb.h);
    // Scalar-field channels: inverse depth (0 for background) and the 3 normal components.
    let ch = |x: usize, y: usize| -> [f32; 4] {
        let i = y * w + x;
        if gb.obj[i] == NO_OBJ {
            [0.0; 4]
        } else {
            [1.0 / gb.depth[i].max(0.1), gb.normal[i].x * 0.5, gb.normal[i].y * 0.5, gb.normal[i].z * 0.5]
        }
    };
    let mut tens = vec![[0.0f32; 3]; w * h];
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let mut t = [0.0f32; 3];
            let n: Vec<[f32; 4]> = [(-1i32, -1i32), (0, -1), (1, -1), (-1, 0), (0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)]
                .iter()
                .map(|&(dx, dy)| ch((x as i32 + dx) as usize, (y as i32 + dy) as usize))
                .collect();
            #[allow(clippy::needless_range_loop)]
            for c in 0..4 {
                let gx = (n[2][c] + 2.0 * n[5][c] + n[8][c]) - (n[0][c] + 2.0 * n[3][c] + n[6][c]);
                let gy = (n[6][c] + 2.0 * n[7][c] + n[8][c]) - (n[0][c] + 2.0 * n[1][c] + n[2][c]);
                let wgt = if c == 0 { 4.0 } else { 1.0 };
                t[0] += wgt * gx * gx;
                t[1] += wgt * gy * gy;
                t[2] += wgt * gx * gy;
            }
            tens[y * w + x] = t;
        }
    }
    let n = gb.cols * gb.rows;
    let mut out = BaselineOut { chars: vec![' '; n], edge: vec![false; n], obj: vec![NO_OBJ; n] };
    const THRESH: f32 = 0.35;
    for cy in 0..gb.rows {
        for cx in 0..gb.cols {
            let (mut jxx, mut jyy, mut jxy, mut mx) = (0.0, 0.0, 0.0, 0.0f32);
            for y in cy * gb.sy..(cy + 1) * gb.sy {
                for x in cx * gb.sx..(cx + 1) * gb.sx {
                    let t = tens[y * w + x];
                    jxx += t[0];
                    jyy += t[1];
                    jxy += t[2];
                    mx = mx.max((t[0] + t[1]).sqrt());
                }
            }
            let i = cy * gb.cols + cx;
            out.obj[i] = cell_obj(gb, cx, cy);
            if mx > THRESH {
                // Dominant gradient angle; the line runs perpendicular to it.
                let theta = 0.5 * (2.0 * jxy).atan2(jxx - jyy);
                let phi = (theta + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::PI);
                let b = ((phi / (std::f32::consts::PI / 4.0)).round() as i32) % 4;
                out.chars[i] = ['-', '\\', '|', '/'][b as usize];
                out.edge[i] = true;
            } else {
                out.chars[i] = tone_char(gb.cell_light(cx, cy), gb.cell_coverage(cx, cy));
            }
        }
    }
    out
}
