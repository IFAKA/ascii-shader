//! Metrics (DESIGN.md "Metrics"): changed, flicker (A -> B -> A), respell over persistent ids.
use std::collections::HashMap;

/// Number of cells whose char differs between two frames.
pub fn count_changed(prev: &[char], cur: &[char]) -> usize {
    prev.iter().zip(cur).filter(|(a, b)| a != b).count()
}

/// Number of cells with `a == c != b` (frame t-1 == t+1 != t).
pub fn count_flicker(a: &[char], b: &[char], c: &[char]) -> usize {
    (0..a.len()).filter(|&i| a[i] == c[i] && a[i] != b[i]).count()
}

const COMB: &str = "┘└┐┌┬┴├┤╷╵╴╶";
const HORIZ: &str = "─⎺⎻⎼⎽_‾▁▔╴╶┌┐└┘┬┴├┤┼";
const VERT: &str = "│╷╵┌┐└┘├┤┬┴┼";
const SCAN: &str = "⎺⎻─⎼⎽";

/// Look counts of one frame: `(combs, holes, crosses)`.
/// combs: runs of 3+ identical corner/tee/stub glyphs, or 3+ `|` / `│`, side by side in a row.
/// holes: a line glyph followed (same row) by 1-2 blank cells while the line goes on one row up or down after the gap
/// (shallow), or the same down a column (steep). The glyph before the gap must stand free (no glyph diagonally or
/// beside it), so two strands passing each other are not a hole.
/// crosses: `┼` cells.
pub fn look_counts(chars: &[char], cols: usize) -> (usize, usize, usize) {
    let rows = chars.len() / cols;
    let at = |r: i32, c: i32| -> char {
        if r < 0 || c < 0 || r as usize >= rows || c as usize >= cols { ' ' } else { chars[r as usize * cols + c as usize] }
    };
    let (mut combs, mut holes, mut crosses) = (0, 0, 0);
    for r in 0..rows as i32 {
        let mut c = 0;
        while c < cols as i32 {
            let ch = at(r, c);
            let mut n = 1;
            while c + n < cols as i32 && at(r, c + n) == ch {
                n += 1;
            }
            if n >= 3 && (COMB.contains(ch) || ch == '|' || ch == '│') {
                combs += 1;
            }
            c += n;
        }
    }
    // A shallow line spelled in a row ends and restarts one row up or down after 1-2 blank cells. The end glyph must
    // sit on the side of the cell the line leaves towards (`⎼⎽_▁` low, `⎺⎻‾▔` high, `─` either), and both sides must
    // be runs of 2+ (so two separate lines side by side do not count).
    const LOW: &str = "─⎼⎽_▁┘└┐┌┬┴├┤┼";
    const HIGH: &str = "─⎺⎻‾▔┘└┐┌┬┴├┤┼";
    for r in 0..rows as i32 {
        for c in 0..cols as i32 {
            let ch = at(r, c);
            if ch == '┼' {
                crosses += 1;
            }
            // box-drawing only: ascii `-` `|` belong to marks
            if HORIZ.contains(ch) && HORIZ.contains(at(r, c - 1)) {
                // The end glyph must really end its line: nothing diagonal or vertical touches it above or below
                // (else it is one strand passing another, not a break).
                let ends_here = [(-1, 0), (1, 0), (-1, 1), (1, 1)].iter().all(|&(dr, dc)| at(r + dr, c + dc) == ' ');
                'h: for g in 1..=2 {
                    if at(r, c + g) != ' ' {
                        break;
                    }
                    for dr in [-1, 1] {
                        let leave_ok = if dr == 1 { LOW.contains(ch) } else { HIGH.contains(ch) };
                        let resume = at(r + dr, c + g + 1);
                        let resume_ok = if dr == 1 { HIGH.contains(resume) } else { LOW.contains(resume) };
                        if leave_ok && resume_ok && ends_here && HORIZ.contains(at(r + dr, c + g + 2)) && (1..=g).all(|k| at(r + dr, c + k) == ' ') {
                            holes += 1;
                            break 'h;
                        }
                    }
                }
            }
            if VERT.contains(ch) && VERT.contains(at(r - 1, c)) {
                let ends_here = [(0, -1), (0, 1), (1, -1), (1, 1)].iter().all(|&(dr, dc)| at(r + dr, c + dc) == ' ');
                'v: for g in 1..=2 {
                    if at(r + g, c) != ' ' {
                        break;
                    }
                    for dc in [-1, 1] {
                        if ends_here && (1..=g).all(|k| at(r + k, c + dc) == ' ') && VERT.contains(at(r + g + 1, c + dc)) && VERT.contains(at(r + g + 2, c + dc)) {
                            holes += 1;
                            break 'v;
                        }
                    }
                }
            }
        }
    }
    (combs, holes, crosses)
}

/// Accumulates the three metrics over a sequence of frames.
#[derive(Default)]
pub struct Metrics {
    prev: Option<Vec<char>>,
    prev2: Option<Vec<char>>,
    prev_ids: HashMap<u64, char>,
    changed: usize,
    changed_den: usize,
    flicker: usize,
    flicker_den: usize,
    respell: usize,
    respell_den: usize,
    /// Respell that treats the five scan-line glyphs as one (a run stepping a level is motion, not a respell).
    respell_flat: usize,
    looks: (usize, usize, usize),
    /// Line cells summed over frames (ids with the mark bit clear whose glyph is shown).
    cells: usize,
    frames: usize,
    pub has_ids: bool,
}

impl Metrics {
    /// Feed one frame: its chars and (optionally) the persistent ids `(key, cell, glyph)`.
    pub fn push(&mut self, chars: &[char], ids: Option<&[(u64, usize, char)]>) {
        if let Some(p) = &self.prev {
            self.changed += count_changed(p, chars);
            self.changed_den += chars.len();
            if let Some(p2) = &self.prev2 {
                self.flicker += count_flicker(p2, p, chars);
                self.flicker_den += chars.len();
            }
        }
        if let Some(ids) = ids {
            self.has_ids = true;
            self.cells += ids.iter().filter(|&&(k, i, g)| k >> 63 == 0 && chars[i] == g).count();
            let cur: HashMap<u64, char> = ids.iter().map(|&(k, _, g)| (k, g)).collect();
            for (k, g) in &cur {
                if let Some(pg) = self.prev_ids.get(k) {
                    self.respell_den += 1;
                    if pg != g {
                        self.respell += 1;
                        if !(SCAN.contains(*pg) && SCAN.contains(*g)) {
                            self.respell_flat += 1;
                        }
                    }
                }
            }
            self.prev_ids = cur;
        }
        self.prev2 = self.prev.take();
        self.prev = Some(chars.to_vec());
    }

    /// Feed the look counts of one frame.
    pub fn look(&mut self, chars: &[char], cols: usize) {
        let (a, b, c) = look_counts(chars, cols);
        self.looks.0 += a;
        self.looks.1 += b;
        self.looks.2 += c;
        self.frames += 1;
    }
    /// Per-frame averages `(combs, holes, crosses)`.
    pub fn look_avg(&self) -> (f64, f64, f64) {
        let n = self.frames.max(1) as f64;
        (self.looks.0 as f64 / n, self.looks.1 as f64 / n, self.looks.2 as f64 / n)
    }
    /// Average line cells per frame.
    pub fn cells_avg(&self) -> f64 {
        self.cells as f64 / self.frames.max(1) as f64
    }
    pub fn respell_flat_pct(&self) -> f64 {
        Self::pct(self.respell_flat, self.respell_den)
    }

    fn pct(n: usize, d: usize) -> f64 {
        if d == 0 { 0.0 } else { 100.0 * n as f64 / d as f64 }
    }
    pub fn changed_pct(&self) -> f64 {
        Self::pct(self.changed, self.changed_den)
    }
    pub fn flicker_pct(&self) -> f64 {
        Self::pct(self.flicker, self.flicker_den)
    }
    pub fn respell_pct(&self) -> f64 {
        Self::pct(self.respell, self.respell_den)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_counts_basic() {
        let f = |rows: &[&str]| -> (usize, usize, usize) {
            let cols = rows[0].chars().count();
            look_counts(&rows.concat().chars().collect::<Vec<_>>(), cols)
        };
        assert_eq!(f(&["┘┘┘┘ ", "     "]).0, 1);
        assert_eq!(f(&["──┘┘ "]).0, 0);
        assert_eq!(f(&["─── ┼"]).2, 1);
        // step with a gap of one blank column (the end glyph leans down, the restart is high)
        assert_eq!(f(&["⎻─⎼⎽   ", "     ⎺⎻"]).1, 1);
        assert_eq!(f(&["────", "    "]).1, 0);
        // the same shape, but the end glyph is touched by a diagonal from below: another strand, not a break
        assert_eq!(f(&["⎻─⎼⎽   ", "   ╲ ⎺⎻"]).1, 0);
    }

    #[test]
    fn flicker_aba() {
        let a = vec!['a', 'x', 'a', 'a'];
        let b = vec!['b', 'x', 'a', 'c'];
        // frames: A, B, A  -> cell 0 flickers (a b a), cell 3 does not (a c a is also A-B-A!)
        let mut m = Metrics::default();
        m.push(&a, None);
        m.push(&b, None);
        m.push(&a, None);
        // cells 0 and 3 are A->B->A, cells 1 and 2 are stable or moved honestly
        assert_eq!(m.flicker, 2);
        assert!((m.flicker_pct() - 50.0).abs() < 1e-9);
        // honest motion: a, b, c never flickers
        let mut m = Metrics::default();
        m.push(&['a'], None);
        m.push(&['b'], None);
        m.push(&['c'], None);
        assert_eq!(m.flicker, 0);
    }

    #[test]
    fn respell_counts_ids() {
        let mut m = Metrics::default();
        m.push(&['a'], Some(&[(1, 0, 'a'), (2, 0, 'b')]));
        m.push(&['a'], Some(&[(1, 0, 'a'), (2, 0, 'c'), (3, 0, 'x')]));
        assert_eq!(m.respell_den, 2);
        assert_eq!(m.respell, 1);
    }
}
