//! Compose and colour (DESIGN.md section 4): line glyph > mark glyph > blank, colour modes, PNG output.
use crate::baseline::BaselineOut;
use crate::glyphs::{Atlas, GH, GW};
use crate::lines::LineOutput;
use crate::marks::MarkOutput;
use crate::raster::{GBuffer, NO_OBJ};
use crate::scene::Scene;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: (u8, u8, u8),
    pub bold: bool,
}

pub const BLANK: Cell = Cell { ch: ' ', fg: (0, 0, 0), bold: false };

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorMode {
    None,
    Tron,
    Light,
}

impl ColorMode {
    pub fn next(self) -> ColorMode {
        match self {
            ColorMode::None => ColorMode::Tron,
            ColorMode::Tron => ColorMode::Light,
            ColorMode::Light => ColorMode::None,
        }
    }
}

const CYAN: (u8, u8, u8) = (0, 230, 255);
const ORANGE: (u8, u8, u8) = (255, 140, 0);
const TEAL: (u8, u8, u8) = (0, 105, 110);
const BLUE: (u8, u8, u8) = (60, 120, 215);
const WHITE: (u8, u8, u8) = (255, 250, 235);
const GREY: (u8, u8, u8) = (204, 204, 204);

/// What kind of ink a cell is, for colouring.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Line,
    Mark,
}

fn color(mode: ColorMode, scene: &Scene, gb: &GBuffer, cx: usize, cy: usize, obj: u16, kind: Kind) -> (u8, u8, u8) {
    match mode {
        ColorMode::None => GREY,
        ColorMode::Tron => match kind {
            Kind::Line if obj != NO_OBJ && scene.objects[obj as usize].class == 1 => ORANGE,
            Kind::Line if obj != NO_OBJ && scene.objects[obj as usize].class == 2 => BLUE,
            Kind::Line if obj != NO_OBJ && scene.objects[obj as usize].class == 3 => WHITE,
            Kind::Line => CYAN,
            Kind::Mark => TEAL,
        },
        ColorMode::Light => {
            let v = (60.0 + 195.0 * gb.cell_light(cx, cy).clamp(0.0, 1.0)) as u8;
            (v, v, v)
        }
    }
}

/// Line glyph beats mark glyph; empty otherwise. Either layer may be absent (modes `lines` / `marks`).
pub fn compose(scene: &Scene, gb: &GBuffer, lines: Option<&LineOutput>, marks: Option<&MarkOutput>, mode: ColorMode) -> Vec<Cell> {
    let (cols, rows) = (gb.cols, gb.rows);
    let mut out = vec![BLANK; cols * rows];
    for cy in 0..rows {
        for cx in 0..cols {
            let i = cy * cols + cx;
            if let Some(Some(ch)) = lines.map(|l| l.glyphs[i]) {
                let o = lines.map(|l| l.obj[i]).unwrap_or(NO_OBJ);
                out[i] = Cell { ch, fg: color(mode, scene, gb, cx, cy, o, Kind::Line), bold: true };
            } else if let Some(Some(ch)) = marks.map(|m| m.glyphs[i]) {
                let o = marks.map(|m| m.obj[i]).unwrap_or(NO_OBJ);
                out[i] = Cell { ch, fg: color(mode, scene, gb, cx, cy, o, Kind::Mark), bold: false };
            }
        }
    }
    out
}

/// Colour a baseline (tone/edge) output.
pub fn from_baseline(scene: &Scene, gb: &GBuffer, b: &BaselineOut, mode: ColorMode) -> Vec<Cell> {
    let mut out = vec![BLANK; gb.cols * gb.rows];
    for cy in 0..gb.rows {
        for cx in 0..gb.cols {
            let i = cy * gb.cols + cx;
            if b.chars[i] != ' ' {
                let kind = if b.edge[i] { Kind::Line } else { Kind::Mark };
                out[i] = Cell { ch: b.chars[i], fg: color(mode, scene, gb, cx, cy, b.obj[i], kind), bold: b.edge[i] };
            }
        }
    }
    out
}

/// Plain text of a cell grid (rows joined by newline, trailing spaces kept so the grid is rectangular).
pub fn to_text(cells: &[Cell], cols: usize) -> String {
    let mut s = String::with_capacity(cells.len() + cells.len() / cols.max(1));
    for row in cells.chunks(cols) {
        s.extend(row.iter().map(|c| c.ch));
        s.push('\n');
    }
    s
}

/// Draw the grid with the atlas glyphs (8 x 16 px per cell, white-on-black tinted by fg) into a PNG file.
pub fn write_png(path: &str, cells: &[Cell], cols: usize, atlas: &Atlas) -> Result<(), String> {
    let rows = cells.len() / cols;
    let (w, h) = (cols * GW, rows * GH);
    let mut rgb = vec![0u8; w * h * 3];
    for (i, c) in cells.iter().enumerate() {
        let Some(g) = atlas.get(c.ch) else { continue };
        let (ox, oy) = ((i % cols) * GW, (i / cols) * GH);
        for y in 0..GH {
            for x in 0..GW {
                let cov = (g.mask[y * GW + x] * if c.bold { 1.25 } else { 1.0 }).min(1.0);
                let o = ((oy + y) * w + ox + x) * 3;
                rgb[o] = (c.fg.0 as f32 * cov) as u8;
                rgb[o + 1] = (c.fg.1 as f32 * cov) as u8;
                rgb[o + 2] = (c.fg.2 as f32 * cov) as u8;
            }
        }
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(&rgb).map_err(|e| e.to_string())
}
