//! STUB. To implement: DESIGN.md section 3 (Garcia et al. anchor points from object-space Voronoi cells, level
//! weights, area weights, density with per-anchor hysteresis, stipple/hatch glyphs, cell conflict). Signatures final.
use crate::raster::{GBuffer, NO_OBJ};
use crate::scene::Camera;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum MarkStyle {
    Stipple,
    Hatch,
}

/// Memory across frames (per-anchor hysteresis: threshold state, glyph band, cell owner).
#[derive(Default)]
pub struct MarkState {}

#[derive(Clone, Debug)]
pub struct MarkParams {
    /// Base grid frequency of the anchor grid (object-space units^-1, before the 2^level scaling).
    pub f: f32,
    /// Levels k in [-n, n].
    pub n_levels: i32,
    /// Maximum anchor weight after the Hermite step.
    pub w_max: f32,
    /// Overall density multiplier (anchor draws if darkness * density > r).
    pub density: f32,
    pub style: MarkStyle,
    /// Use every `stride`-th G-buffer pixel in x and y.
    pub stride: usize,
}

impl Default for MarkParams {
    fn default() -> Self {
        MarkParams { f: 1.0, n_levels: 1, w_max: 1.0, density: 1.0, style: MarkStyle::Stipple, stride: 1 }
    }
}

pub struct MarkOutput {
    /// `cols * rows` mark glyph per cell, or None.
    pub glyphs: Vec<Option<char>>,
    /// Object index per mark cell (`NO_OBJ` where none).
    pub obj: Vec<u16>,
    /// `(anchor id, cell index, glyph)` for the respell metric.
    pub ids: Vec<(u64, usize, char)>,
    /// Anchor weight 0..1 per cell (0 where none).
    pub weight: Vec<f32>,
}

pub fn render_marks(_gb: &GBuffer, _cam: &Camera, _params: &MarkParams, _state: &mut MarkState, cols: usize, rows: usize) -> MarkOutput {
    MarkOutput { glyphs: vec![None; cols * rows], obj: vec![NO_OBJ; cols * rows], ids: vec![], weight: vec![0.0; cols * rows] }
}
