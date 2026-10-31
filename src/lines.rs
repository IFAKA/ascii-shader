//! STUB. To implement: DESIGN.md section 2 (feature edges from the mesh, visibility against the depth buffer,
//! per-cell path segments, glyph cost, typist with spelling memory). Signatures are final.
use crate::glyphs::{Atlas, LineSet};
use crate::raster::{GBuffer, NO_OBJ};
use crate::scene::{Camera, Scene};

/// Memory across frames (spelling memory: key -> last glyph, plus any per-mesh edge tables).
#[derive(Default)]
pub struct LineState {}

#[derive(Clone, Debug)]
pub struct LineParams {
    /// Dihedral angle (degrees) above which an edge is a crease. Default 35.
    pub crease_deg: f32,
    /// Keep last frame's glyph if its cost <= best cost + keep_margin (glyph pixel units).
    pub keep_margin: f32,
    /// ... and its cost <= keep_tol.
    pub keep_tol: f32,
    pub lineset: LineSet,
}

impl Default for LineParams {
    fn default() -> Self {
        LineParams { crease_deg: 35.0, keep_margin: 0.5, keep_tol: 2.5, lineset: LineSet::Box }
    }
}

pub struct LineOutput {
    /// `cols * rows` line glyph per cell, or None.
    pub glyphs: Vec<Option<char>>,
    /// Object index owning each line cell (`NO_OBJ` where no line), for colour.
    pub obj: Vec<u16>,
    /// `(persistent memory key, cell index, glyph)` for the respell metric.
    pub ids: Vec<(u64, usize, char)>,
}

#[allow(clippy::too_many_arguments)]
pub fn render_lines(
    _scene: &Scene,
    _cam: &Camera,
    _gb: &GBuffer,
    _atlas: &Atlas,
    _params: &LineParams,
    _state: &mut LineState,
    cols: usize,
    rows: usize,
) -> LineOutput {
    LineOutput { glyphs: vec![None; cols * rows], obj: vec![NO_OBJ; cols * rows], ids: vec![] }
}
