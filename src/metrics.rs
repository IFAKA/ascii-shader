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
            let cur: HashMap<u64, char> = ids.iter().map(|&(k, _, g)| (k, g)).collect();
            for (k, g) in &cur {
                if let Some(pg) = self.prev_ids.get(k) {
                    self.respell_den += 1;
                    if pg != g {
                        self.respell += 1;
                    }
                }
            }
            self.prev_ids = cur;
        }
        self.prev2 = self.prev.take();
        self.prev = Some(chars.to_vec());
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
