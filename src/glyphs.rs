//! Glyph atlas: rasterise candidate glyphs from a real monospace font into GW x GH coverage masks, with the ink
//! pixel list and a distance transform of the ink (DESIGN.md section 2). Glyphs the font lacks get analytic masks.
use std::collections::HashMap;

pub const GW: usize = 8;
pub const GH: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum LineSet {
    Ascii,
    Box,
}

const ASCII_SET: &str = "-_|/\\()<>^v',.`+~=[]TL7J";
const BOX_SET: &str = "─│╱╲╳┌┐└┘├┤┬┴┼╭╮╯╰▁▔▏▕_‾',.";
/// Scan-line glyphs used by shallow runs (5 heights, top to bottom), with fallbacks if the font lacks them.
pub const RUN_SET: &str = "⎺⎻─⎼⎽▔‾_▁";
pub const RAMP: &str = " .:-=+*#%@";
pub const MARK_CHARS: &str = ".:;-/|\\";

impl LineSet {
    /// Candidate line glyphs of this set.
    pub fn chars(self) -> Vec<char> {
        match self {
            LineSet::Ascii => ASCII_SET.chars().collect(),
            LineSet::Box => BOX_SET.chars().collect(),
        }
    }
}

/// One glyph: coverage mask (row-major GW x GH, 0..1), ink pixels `(x, y, coverage)` and, per pixel, the
/// distance in pixels to the nearest ink pixel (coverage >= 0.5 counts as ink for the distance field).
#[derive(Clone, Debug)]
pub struct Glyph {
    pub ch: char,
    pub mask: Vec<f32>,
    pub ink: Vec<(u8, u8, f32)>,
    /// Distance (pixels, unclamped) from each mask pixel to the nearest ink pixel. `INFINITY` for blank glyphs.
    pub dist: Vec<f32>,
    /// True if the font had no glyph and an analytic mask was drawn.
    pub analytic: bool,
}

impl Glyph {
    fn from_mask(ch: char, mask: Vec<f32>, analytic: bool) -> Glyph {
        let mut ink = vec![];
        for y in 0..GH {
            for x in 0..GW {
                let c = mask[y * GW + x];
                if c >= 0.1 {
                    ink.push((x as u8, y as u8, c));
                }
            }
        }
        let core: Vec<(usize, usize)> = (0..GH).flat_map(|y| (0..GW).map(move |x| (x, y))).filter(|&(x, y)| mask[y * GW + x] >= 0.5).collect();
        let mut dist = vec![f32::INFINITY; GW * GH];
        for y in 0..GH {
            for x in 0..GW {
                for &(ix, iy) in &core {
                    let (dx, dy) = (x as f32 - ix as f32, y as f32 - iy as f32);
                    dist[y * GW + x] = dist[y * GW + x].min((dx * dx + dy * dy).sqrt());
                }
            }
        }
        Glyph { ch, mask, ink, dist, analytic }
    }
}

pub struct Atlas {
    pub glyphs: Vec<Glyph>,
    index: HashMap<char, usize>,
    /// Chars the font lacked (drawn analytically).
    pub missing: Vec<char>,
}

/// Every char any mode may draw (used to build the atlas once).
pub fn all_chars() -> Vec<char> {
    let mut v: Vec<char> = vec![' '];
    for s in [ASCII_SET, BOX_SET, RUN_SET, RAMP, MARK_CHARS] {
        v.extend(s.chars());
    }
    v.sort();
    v.dedup();
    v
}

fn seg_dist(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 == 0.0 { 0.0 } else { (((px - a.0) * dx + (py - a.1) * dy) / l2).clamp(0.0, 1.0) };
    ((px - a.0 - t * dx).powi(2) + (py - a.1 - t * dy).powi(2)).sqrt()
}

/// Analytic masks for box-drawing and block chars (used only when the font lacks them).
fn analytic_mask(ch: char) -> Option<Vec<f32>> {
    let (cx, cy) = (GW as f32 / 2.0, GH as f32 / 2.0);
    let (w, h) = (GW as f32, GH as f32);
    // Segments in pixel space, y down.
    let (l, r, u, d) = (((0.0, cy), (cx, cy)), ((cx, cy), (w, cy)), ((cx, 0.0), (cx, cy)), ((cx, cy), (cx, h)));
    let segs: Vec<((f32, f32), (f32, f32))> = match ch {
        '─' => vec![l, r],
        '│' => vec![u, d],
        '┌' | '╭' => vec![r, d],
        '┐' | '╮' => vec![l, d],
        '└' | '╰' => vec![r, u],
        '┘' | '╯' => vec![l, u],
        '├' => vec![u, d, r],
        '┤' => vec![u, d, l],
        '┬' => vec![l, r, d],
        '┴' => vec![l, r, u],
        '┼' => vec![l, r, u, d],
        '╱' => vec![((w, 0.0), (0.0, h))],
        '╲' => vec![((0.0, 0.0), (w, h))],
        '╳' => vec![((w, 0.0), (0.0, h)), ((0.0, 0.0), (w, h))],
        _ => vec![],
    };
    let rect: Option<(f32, f32, f32, f32)> = match ch {
        '▁' => Some((0.0, h - 2.0, w, h)),
        '▔' => Some((0.0, 0.0, w, 2.0)),
        '⎺' => Some((0.0, 0.0, w, 2.0)),
        '⎻' => Some((0.0, 4.0, w, 6.0)),
        '⎼' => Some((0.0, 11.0, w, 13.0)),
        '⎽' => Some((0.0, h - 2.0, w, h)),
        '▏' => Some((0.0, 0.0, 1.0, h)),
        '▕' => Some((w - 1.0, 0.0, w, h)),
        _ => None,
    };
    if segs.is_empty() && rect.is_none() {
        return None;
    }
    let mut m = vec![0.0f32; GW * GH];
    for y in 0..GH {
        for x in 0..GW {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut c: f32 = 0.0;
            for &(a, b) in &segs {
                c = c.max((1.5 - seg_dist(px, py, a, b)).clamp(0.0, 1.0));
            }
            if let Some((x0, y0, x1, y1)) = rect && px >= x0 && px <= x1 && py >= y0 && py <= y1 {
                c = 1.0;
            }
            m[y * GW + x] = c;
        }
    }
    Some(m)
}

impl Atlas {
    /// Build an atlas for `chars` from the font at `path` (a .ttc uses collection index 0).
    pub fn load(path: &str, chars: &[char]) -> Result<Atlas, String> {
        let data = std::fs::read(path).map_err(|e| format!("font {path}: {e}"))?;
        let font = fontdue::Font::from_bytes(data, fontdue::FontSettings { collection_index: 0, ..Default::default() })
            .map_err(|e| format!("font {path}: {e}"))?;
        // Choose the pixel size so that the line height is exactly GH pixels.
        let probe = font.horizontal_line_metrics(100.0).ok_or("font has no line metrics")?;
        let px = 100.0 * GH as f32 / probe.new_line_size;
        let lm = font.horizontal_line_metrics(px).ok_or("font has no line metrics")?;
        let baseline = ((GH as f32 - (lm.ascent - lm.descent)) / 2.0 + lm.ascent).round() as i32;
        let mut glyphs = vec![];
        let mut missing = vec![];
        for &ch in chars {
            let has = font.lookup_glyph_index(ch) != 0 || ch == ' ';
            let mut mask = vec![0.0f32; GW * GH];
            let mut analytic = false;
            if has {
                let (m, bmp) = font.rasterize(ch, px);
                let x0 = ((GW as f32 - m.advance_width) / 2.0).round() as i32 + m.xmin;
                let top = baseline - (m.ymin + m.height as i32);
                for by in 0..m.height {
                    for bx in 0..m.width {
                        let (gx, gy) = (x0 + bx as i32, top + by as i32);
                        if gx >= 0 && gy >= 0 && (gx as usize) < GW && (gy as usize) < GH {
                            mask[gy as usize * GW + gx as usize] = bmp[by * m.width + bx] as f32 / 255.0;
                        }
                    }
                }
            } else {
                missing.push(ch);
                if let Some(a) = analytic_mask(ch) {
                    mask = a;
                    analytic = true;
                }
            }
            glyphs.push(Glyph::from_mask(ch, mask, analytic));
        }
        let index = glyphs.iter().enumerate().map(|(i, g)| (g.ch, i)).collect();
        Ok(Atlas { glyphs, index, missing })
    }

    pub fn get(&self, ch: char) -> Option<&Glyph> {
        self.index.get(&ch).map(|&i| &self.glyphs[i])
    }

    /// Atlas index of a char.
    pub fn index_of(&self, ch: char) -> Option<usize> {
        self.index.get(&ch).copied()
    }
}

pub const DEFAULT_FONT: &str = "/System/Library/Fonts/Menlo.ttc";

#[cfg(test)]
mod tests {
    use super::*;

    /// Weight census of the line glyphs: ink mass, centroid and elongation (1 = thin line, 0 = blob).
    /// Prints one line per glyph, sorted by mass, and flags glyphs far from the median mass.
    /// Run: `cargo test weight_census -- --nocapture`
    #[test]
    fn weight_census() {
        let a = Atlas::load(DEFAULT_FONT, &all_chars()).unwrap();
        let mut rows: Vec<(char, f32, f32, f32, f32, bool)> = vec![];
        let mut seen = std::collections::HashSet::new();
        for ch in ASCII_SET.chars().chain(BOX_SET.chars()).chain(RUN_SET.chars()) {
            if !seen.insert(ch) {
                continue;
            }
            let Some(g) = a.get(ch) else { continue };
            let m: f32 = g.ink.iter().map(|i| i.2).sum();
            if m <= 0.0 {
                continue;
            }
            let cx = g.ink.iter().map(|i| i.2 * (i.0 as f32 + 0.5)).sum::<f32>() / m;
            let cy = g.ink.iter().map(|i| i.2 * (i.1 as f32 + 0.5)).sum::<f32>() / m;
            let (mut sxx, mut syy, mut sxy) = (0f32, 0f32, 0f32);
            for &(x, y, c) in &g.ink {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                sxx += c * dx * dx;
                syy += c * dy * dy;
                sxy += c * dx * dy;
            }
            let (tr, det) = (sxx + syy, sxx * syy - sxy * sxy);
            let disc = (tr * tr / 4.0 - det).max(0.0).sqrt();
            let (l1, l2) = (tr / 2.0 + disc, tr / 2.0 - disc);
            let ecc = if l1 > 0.0 { 1.0 - l2 / l1 } else { 0.0 };
            rows.push((ch, m / (GW * GH) as f32 * 100.0, cx / GW as f32, cy / GH as f32, ecc, g.analytic));
        }
        rows.sort_by(|p, q| p.1.partial_cmp(&q.1).unwrap());
        let med = rows[rows.len() / 2].1;
        println!("median coverage {med:.1}%");
        for (ch, cov, cx, cy, ecc, an) in &rows {
            let flag = if *cov > 1.8 * med { "HEAVY" } else if *cov < 0.5 * med { "light" } else { "" };
            println!("{ch} cov={cov:5.1}% col={cx:.2} row={cy:.2} elong={ecc:.2}{} {flag}", if *an { " analytic" } else { "" });
        }
        assert!(!rows.is_empty());
    }

    #[test]
    fn run_glyph_report() {
        let a = Atlas::load(DEFAULT_FONT, &all_chars()).unwrap();
        for ch in RUN_SET.chars().chain("│▏▕".chars()) {
            let g = a.get(ch).unwrap();
            let w: f32 = g.ink.iter().map(|i| i.2).sum();
            let cy: f32 = g.ink.iter().map(|i| i.2 * (i.1 as f32 + 0.5)).sum::<f32>() / w;
            let cx: f32 = g.ink.iter().map(|i| i.2 * (i.0 as f32 + 0.5)).sum::<f32>() / w;
            let ys: Vec<u8> = g.ink.iter().map(|i| i.1).collect();
            println!("{ch} analytic={} missing={} cy={cy:.2} cx={cx:.2} rows {:?}..{:?}", g.analytic, a.missing.contains(&ch), ys.iter().min(), ys.iter().max());
        }
    }

    #[test]
    fn atlas_has_ink() {
        let a = Atlas::load(DEFAULT_FONT, &all_chars()).unwrap();
        for ch in ['─', '/'] {
            let g = a.get(ch).unwrap();
            assert!(g.ink.len() > 4, "{ch} has no ink");
        }
        // distance field is zero on ink
        let g = a.get('/').unwrap();
        let (x, y, _) = g.ink.iter().find(|i| i.2 >= 0.5).copied().unwrap();
        assert_eq!(g.dist[y as usize * GW + x as usize], 0.0);
        assert!(a.get(' ').unwrap().ink.is_empty());
    }
}
