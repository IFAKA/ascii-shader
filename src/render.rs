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
    /// Frames left in each cell's mark-free halo (set while a line is in the 3x3 neighbourhood).
    halo: Vec<u8>,
    /// Per-cell mark debounce state.
    shown: Vec<Debounce>,
}

/// Mark pops (blank, mark, blank) and dips (mark, blank, mark) are the main remaining flicker, so a mark must
/// be present `MARK_APPEAR` frames in a row before it shows, and a shown mark survives `MARK_GAP` missing frame.
const MARK_APPEAR: u8 = 2;
const MARK_GAP: u8 = 1;

#[derive(Clone, Copy, Default)]
struct Debounce {
    run: u8,
    gap: u8,
    shown: Option<char>,
}

impl Debounce {
    fn step(&mut self, raw: Option<char>) -> Option<char> {
        if raw.is_some() {
            self.run = self.run.saturating_add(1);
            self.gap = 0;
        } else {
            self.run = 0;
            self.gap = self.gap.saturating_add(1);
        }
        self.shown = match (self.shown, raw) {
            (Some(_), Some(g)) => Some(g),
            (Some(g), None) if self.gap <= MARK_GAP => Some(g),
            (None, Some(g)) if self.run >= MARK_APPEAR => Some(g),
            _ => None,
        };
        self.shown
    }
}

/// A cell stays mark-free for this many frames after its last line neighbour, so the halo does not blink.
const HALO_HOLD: u8 = 4;
/// Columns a line may be away and still keep an existing halo alive.
const HALO_STAY_X: usize = 2;
/// Columns within which a line blocks new marks (but not existing ones).
const PRE_X: usize = 2;

impl Renderer {
    /// Hand-drawn ink keeps white space next to lines: drop marks in line cells and their 8 neighbours, with a
    /// hold time so the halo does not blink when a line cell appears or disappears.
    fn apply_halo(&mut self, lines: &[Option<char>], marks: &mut [Option<char>], cols: usize, rows: usize) {
        if self.halo.len() != cols * rows {
            self.halo = vec![0; cols * rows];
        }
        for y in 0..rows {
            for x in 0..cols {
                let near = |rx: usize, ry: usize| (y.saturating_sub(ry)..=(y + ry).min(rows - 1)).any(|yy| (x.saturating_sub(rx)..=(x + rx).min(cols - 1)).any(|xx| lines[yy * cols + xx].is_some()));
                let h = self.halo[y * cols + x];
                // Enter on a line within 1 cell; once in, stay while a line is within 2 columns; then hold a few frames.
                self.halo[y * cols + x] = if near(1, 1) || (h > 0 && near(HALO_STAY_X, 1)) { HALO_HOLD } else { h.saturating_sub(1) };
            }
        }
        if self.shown.len() != cols * rows {
            // A fresh state (first frame, resize) shows marks at once.
            self.shown = vec![Debounce { run: MARK_APPEAR, ..Debounce::default() }; cols * rows];
        }
        for (i, ((m, &h), d)) in marks.iter_mut().zip(&self.halo).zip(&mut self.shown).enumerate() {
            let (x, y) = (i % cols, i / cols);
            let near2 = (y.saturating_sub(1)..=(y + 1).min(rows - 1)).any(|yy| (x.saturating_sub(PRE_X)..=(x + PRE_X).min(cols - 1)).any(|xx| lines[yy * cols + xx].is_some()));
            if h == 0 && near2 && d.shown.is_none() {
                // A line is about to arrive: a mark that would live only a frame or two does not start.
                *d = Debounce::default();
                *m = None;
                continue;
            }
            if h > 0 {
                *m = None;
                *d = Debounce::default();
                continue;
            }
            *m = d.step(*m);
        }
    }

    pub fn render(&mut self, scene: &Scene, cam: &Camera, atlas: &Atlas, opts: &Options, cols: usize, rows: usize) -> Frame {
        let gb = raster::rasterize(scene, cam, cols, rows, opts.sx, opts.sy);
        let mut ids = None;
        let cells = match opts.mode {
            Mode::Tone => compose::from_baseline(scene, &gb, &baseline::tone(&gb), opts.color),
            Mode::Edge => compose::from_baseline(scene, &gb, &baseline::edge(&gb), opts.color),
            m => {
                let lo = (m != Mode::Marks).then(|| lines::render_lines(scene, cam, &gb, atlas, &opts.lines, &mut self.line_state, cols, rows));
                let mut mo = (m != Mode::Lines).then(|| marks::render_marks(&gb, cam, &opts.marks, &mut self.mark_state, cols, rows));
                if let (Some(l), Some(mk)) = (&lo, &mut mo) {
                    self.apply_halo(&l.glyphs, &mut mk.glyphs, cols, rows);
                }
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
