//! One-frame pipeline shared by the terminal app, `--dump` and `--bench`: G-buffer -> layers -> compose.
use crate::baseline;
use crate::compose::{self, Cell, ColorMode};
use crate::glyphs::{Atlas, LineSet};
use crate::lines::{self, LineParams, LineState};
use crate::marks::{self, MarkParams, MarkState, MarkStyle};
use crate::raster::{self, GBuffer};
use crate::scene::{Camera, Scene};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    #[value(name = "struct")]
    Struct,
    Lines,
    Marks,
    Tone,
    Edge,
}

impl Mode {
    pub const ALL: [Mode; 5] = [Mode::Struct, Mode::Lines, Mode::Marks, Mode::Tone, Mode::Edge];
    pub fn name(self) -> &'static str {
        match self {
            Mode::Struct => "struct",
            Mode::Lines => "lines",
            Mode::Marks => "marks",
            Mode::Tone => "tone",
            Mode::Edge => "edge",
        }
    }
}

/// All tunable options.
#[derive(Clone, Debug)]
pub struct Options {
    pub mode: Mode,
    pub color: ColorMode,
    pub lines: LineParams,
    pub marks: MarkParams,
    pub sx: usize,
    pub sy: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { mode: Mode::Struct, color: ColorMode::Tron, lines: LineParams::default(), marks: MarkParams::default(), sx: 4, sy: 8 }
    }
}

impl Options {
    pub fn set_lineset(&mut self, l: LineSet) {
        self.lines.lineset = l;
    }
    pub fn toggle_marks(&mut self) {
        self.marks.style = if self.marks.style == MarkStyle::Stipple { MarkStyle::Hatch } else { MarkStyle::Stipple };
    }
}

pub struct Frame {
    pub cells: Vec<Cell>,
    pub cols: usize,
    pub rows: usize,
    /// Persistent ids `(key, cell, glyph)` from lines and marks (marks keys have the top bit set); None for baselines.
    pub ids: Option<Vec<(u64, usize, char)>>,
    pub gbuffer: GBuffer,
}

#[derive(Default)]
pub struct Renderer {
    pub line_state: LineState,
    pub mark_state: MarkState,
}

impl Renderer {
    pub fn render(&mut self, scene: &Scene, cam: &Camera, atlas: &Atlas, opts: &Options, cols: usize, rows: usize) -> Frame {
        let gb = raster::rasterize(scene, cam, cols, rows, opts.sx, opts.sy);
        let mut ids = None;
        let cells = match opts.mode {
            Mode::Tone => compose::from_baseline(scene, &gb, &baseline::tone(&gb), opts.color),
            Mode::Edge => compose::from_baseline(scene, &gb, &baseline::edge(&gb), opts.color),
            m => {
                let lo = (m != Mode::Marks).then(|| lines::render_lines(scene, cam, &gb, atlas, &opts.lines, &mut self.line_state, cols, rows));
                let mo = (m != Mode::Lines).then(|| marks::render_marks(&gb, cam, &opts.marks, &mut self.mark_state, cols, rows));
                let mut v = lo.as_ref().map(|l| l.ids.clone()).unwrap_or_default();
                if let Some(mk) = &mo {
                    v.extend(mk.ids.iter().map(|&(k, c, g)| (k | (1 << 63), c, g)));
                }
                ids = Some(v);
                compose::compose(scene, &gb, lo.as_ref(), mo.as_ref(), opts.color)
            }
        };
        Frame { cells, cols, rows, ids, gbuffer: gb }
    }
}
