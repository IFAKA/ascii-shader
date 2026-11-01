//! ascii-shader: a terminal renderer that draws 3D scenes with lines and marks. See DESIGN.md.
#![allow(dead_code)] // public API surface is built ahead of its users (lines/marks agents)
mod baseline;
mod compose;
mod cel;
mod fighter2d;
mod film;
mod pv;
mod glyphs;
mod lines;
mod marks;
mod metrics;
mod raster;
mod render;
mod scene;
mod term;

use clap::Parser;
use compose::ColorMode;
use glyphs::{Atlas, LineSet};
use marks::MarkStyle;
use metrics::Metrics;
use render::{Mode, Options, Renderer};
use scene::Scene;
use std::time::Instant;

#[derive(Parser)]
#[command(about = "Terminal ASCII renderer with lines and marks")]
struct Cli {
    #[arg(long, value_enum, default_value = "struct")]
    mode: Mode,
    /// chapel | shapes
    #[arg(long, default_value = "chapel")]
    scene: String,
    /// Load a Wavefront OBJ instead of a built-in scene
    #[arg(long)]
    obj: Option<String>,
    #[arg(long, default_value = glyphs::DEFAULT_FONT)]
    font: String,
    #[arg(long, value_enum, default_value = "box")]
    lineset: LineSet,
    /// Runs: edges within this screen slope (cell units) of horizontal/vertical are spelled one cell per column/row (0 = off)
    #[arg(long, default_value_t = 0.5)]
    run_slope: f32,
    /// Do not spell the slopes between shallow and steep as runs (per-cell scoring instead)
    #[arg(long)]
    no_diag_runs: bool,
    /// Remove 1-2 cell stubs whose edge chain is shorter than this many cells (0 = off)
    #[arg(long, default_value_t = 3.0)]
    stub_chain: f32,
    /// Run level hysteresis, in levels
    #[arg(long, default_value_t = 0.6)]
    run_keep: f32,
    /// Keep corner/tee glyphs in straight one-direction cells (default off = masked)
    #[arg(long)]
    no_straight_mask: bool,
    /// Scan-line glyphs of shallow runs
    #[arg(long, value_enum, default_value = "scan")]
    runset: lines::RunSet,
    #[arg(long, value_enum, default_value = "stipple")]
    marks: MarkStyle,
    #[arg(long, value_enum, default_value = "tron")]
    color: ColorMode,
    /// Render N frames of the bench camera path and write them to --out
    #[arg(long)]
    dump: Option<usize>,
    #[arg(long, default_value = "out")]
    out: String,
    /// With --dump: first bench camera frame (default 120)
    #[arg(long, default_value_t = 120)]
    dump_start: usize,
    /// With --dump: also write PNGs
    #[arg(long)]
    png: bool,
    /// Render the 30 s anime short ORB to this MP4 (needs ffmpeg); the frames are terminal cells
    #[arg(long)]
    film: Option<String>,
    /// Play the 30 s anime short ORB in this terminal (needs 160x45 cells)
    #[arg(long)]
    play_film: bool,
    /// With --story pvshort: write the list of visual events (JSON) that the sound is built from
    #[arg(long)]
    film_events: Option<String>,
    /// Which film: pv (ONE CUT teaser, default), pvshort (10 s social short) or orb
    #[arg(long, default_value = "pv")]
    story: String,
    /// Write PNG stills of the film at these times, "DIR:t1,t2,..."
    #[arg(long)]
    film_stills: Option<String>,
    /// WAV soundtrack for --film (muxed) or --play-film (afplay/ffplay)
    #[arg(long)]
    audio: Option<String>,
    /// Run every mode for 240 frames at 160x50 and print metrics
    #[arg(long)]
    bench: bool,
    #[arg(long)]
    cols: Option<usize>,
    #[arg(long)]
    rows: Option<usize>,
    /// Mark density multiplier
    #[arg(long, default_value_t = 2.0)]
    density: f32,
    /// Turn off parallel-edge suppression
    #[arg(long)]
    no_parallel_cull: bool,
    /// Turn off level-of-detail culling of short edges
    #[arg(long)]
    no_lod: bool,
    #[arg(long, default_value_t = 35.0)]
    crease_deg: f32,
    #[arg(long, default_value_t = 2.0)]
    keep_margin: f32,
    #[arg(long, default_value_t = 5.0)]
    keep_tol: f32,
    /// Mark anchor grid base frequency
    #[arg(long, default_value_t = 0.4)]
    f: f32,
    /// Mark levels n (k in -n..=n)
    #[arg(long, default_value_t = 1)]
    levels: i32,
    #[arg(long, default_value_t = 1.0)]
    w_max: f32,
    /// Mark pixel stride
    #[arg(long, default_value_t = 1)]
    stride: usize,
    /// G-buffer pixels per cell, x
    #[arg(long, default_value_t = 4)]
    sx: usize,
    /// G-buffer pixels per cell, y
    #[arg(long, default_value_t = 8)]
    sy: usize,
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

fn main() {
    let cli = Cli::parse();
    let scene = if let Some(p) = &cli.obj {
        let src = std::fs::read_to_string(p).unwrap_or_else(|e| fail(format!("{p}: {e}")));
        Scene::from_obj(scene::parse_obj(&src).unwrap_or_else(|e| fail(format!("{p}: {e}"))))
    } else {
        Scene::by_name(&cli.scene).unwrap_or_else(|| fail(format!("unknown scene {}", cli.scene)))
    };
    let atlas = Atlas::load(&cli.font, &glyphs::all_chars()).unwrap_or_else(|e| fail(e));
    if !atlas.missing.is_empty() {
        let (a, n): (Vec<char>, Vec<char>) = atlas.missing.iter().partition(|&&c| atlas.get(c).is_some_and(|g| g.analytic));
        eprintln!("font lacks {} glyphs; analytic masks drawn for {:?}; no mask for {:?}", atlas.missing.len(), a, n);
    }
    let mut opts = Options { mode: cli.mode, color: cli.color, sx: cli.sx.max(1), sy: cli.sy.max(1), ..Options::default() };
    opts.lines.lineset = cli.lineset;
    opts.lines.crease_deg = cli.crease_deg;
    opts.lines.run_slope = cli.run_slope;
    opts.lines.diag_runs = !cli.no_diag_runs;
    opts.lines.stub_chain = cli.stub_chain;
    opts.lines.runset = cli.runset;
    opts.lines.straight_mask = !cli.no_straight_mask;
    opts.lines.run_keep = cli.run_keep;
    opts.lines.parallel_cull = !cli.no_parallel_cull;
    opts.lines.lod = !cli.no_lod;
    opts.lines.keep_margin = cli.keep_margin;
    opts.lines.keep_tol = cli.keep_tol;
    opts.marks.style = cli.marks;
    opts.marks.density = cli.density;
    opts.marks.f = cli.f;
    opts.marks.n_levels = cli.levels;
    opts.marks.w_max = cli.w_max;
    opts.marks.stride = cli.stride.max(1);

    if let Some(p) = &cli.film_events {
        if cli.story != "pvshort" {
            fail("--film-events needs --story pvshort");
        }
        std::fs::write(p, pv::short::events_json()).unwrap_or_else(|e| fail(format!("{p}: {e}")));
    }
    if cli.film.is_some() || cli.play_film || cli.film_stills.is_some() {
        eprintln!("drawing the film...");
        let frames = match cli.story.as_str() {
            "orb" => film::prerender(&atlas, &opts),
            "pvshort" => pv::short::prerender(&atlas, &opts),
            "poses" => fighter2d::pose_sheet(),
            _ => pv::prerender(&atlas, &opts),
        };
        if let Some(spec) = &cli.film_stills {
            let (dir, ts) = spec.split_once(':').unwrap_or((spec, "0"));
            let times: Vec<f32> = ts.split(',').filter_map(|x| x.parse().ok()).collect();
            film::write_stills(&frames, &cli.font, dir, &times).unwrap_or_else(|e| fail(e));
        }
        if let Some(out) = &cli.film {
            film::write_video(&frames, &cli.font, out, cli.audio.as_deref()).unwrap_or_else(|e| fail(e));
        }
        if cli.play_film {
            film::play(&frames, cli.audio.as_deref()).unwrap_or_else(|e| fail(e));
        }
    } else if cli.film_events.is_some() {
        // events only, nothing to draw
    } else if cli.bench {
        bench(&scene, &atlas, &opts, cli.cols.unwrap_or(160), cli.rows.unwrap_or(50));
    } else if let Some(n) = cli.dump {
        dump(&scene, &atlas, &opts, cli.dump_start, n, cli.cols.unwrap_or(160), cli.rows.unwrap_or(50), &cli.out, cli.png);
    } else if let Err(e) = term::run(&scene, &atlas, opts, cli.cols.zip(cli.rows)) {
        fail(e);
    }
}

fn bench(scene: &Scene, atlas: &Atlas, base: &Options, cols: usize, rows: usize) {
    for mode in Mode::ALL {
        let opts = Options { mode, ..base.clone() };
        let mut r = Renderer::default();
        let mut m = Metrics::default();
        // warm-up (thread pool, caches); not measured and not fed to metrics
        let mut warm = Renderer::default();
        warm.render(scene, &scene.bench_camera(120), atlas, &opts, cols, rows);
        let mut total = 0.0;
        let mut classes: Vec<(Vec<char>, Vec<u8>)> = vec![];
        let mut all_chars: Vec<Vec<char>> = vec![];
        for f in 0..240 {
            let cam = scene.bench_camera(f);
            let t = Instant::now();
            let frame = r.render(scene, &cam, atlas, &opts, cols, rows);
            total += t.elapsed().as_secs_f64() * 1000.0;
            let chars: Vec<char> = frame.cells.iter().map(|c| c.ch).collect();
            m.push(&chars, frame.ids.as_deref());
            m.look(&chars, cols);
            if std::env::var_os("ASCII_PAIRS").is_some() {
                all_chars.push(chars.clone());
            }
            if mode == Mode::Struct && std::env::var_os("ASCII_CLASSIFY").is_some() {
                classes.push((chars.clone(), cell_classes(&chars, frame.ids.as_deref().unwrap_or(&[]))));
            }
        }
        if mode == Mode::Struct && std::env::var_os("ASCII_PAIRS").is_some() {
            // Debug: most common A -> B -> A glyph triples.
            let mut c = std::collections::HashMap::<(char, char), usize>::new();
            let rows = all_chars[0].len() / cols;
            for (wi, w) in all_chars.windows(3).enumerate() {
                for i in 0..w[0].len() {
                    if w[0][i] == w[2][i] && w[0][i] != w[1][i] {
                        *c.entry((w[0][i], w[1][i])).or_default() += 1;
                        if w[0][i] == '─' && w[1][i] == '⎼' && c[&('─', '⎼')] % 25 == 1 {
                            println!("  sample: window at frame {} row {} col {}", wi, i / cols, i % cols);
                            let (r0, c0) = (i / cols, i % cols);
                            for r in r0.saturating_sub(2)..(r0 + 3).min(rows) {
                                let line: Vec<String> = w.iter().map(|f| (c0.saturating_sub(10)..(c0 + 11).min(cols)).map(|x| f[r * cols + x]).collect::<String>()).collect();
                                println!("    {}", line.join(" | "));
                            }
                        }
                    }
                }
            }
            let mut v: Vec<_> = c.into_iter().collect();
            v.sort_by_key(|x| std::cmp::Reverse(x.1));
            println!("  flicker pairs (A,B): {:?}", &v[..v.len().min(14)]);
        }
        if !classes.is_empty() {
            classify_flicker(&classes, cols, rows);
        }
        let respell = if m.has_ids { format!("{:.3}%", m.respell_pct()) } else { "n/a".into() };
        let (combs, holes, crosses) = m.look_avg();
        let flat = if m.has_ids { format!("{:.3}%", m.respell_flat_pct()) } else { "n/a".into() };
        println!(
            "mode={} ms={:.2} changed={:.2}% flicker={:.3}% respell={} respell_flat={} combs={:.2} holes={:.2} crosses={:.2} cells={:.1} dholes={:.2} stubs={:.2}",
            mode.name(), total / 240.0, m.changed_pct(), m.flicker_pct(), respell, flat, combs, holes, crosses, m.cells_avg(), m.diag_avg().0, m.diag_avg().1
        );
    }
}

/// Per cell: 0 blank, 1 line glyph, 2 mark glyph (from the persistent ids whose glyph is the one shown).
fn cell_classes(chars: &[char], ids: &[(u64, usize, char)]) -> Vec<u8> {
    let mut c = vec![0u8; chars.len()];
    for &(k, i, g) in ids {
        if chars[i] == g {
            c[i] = if k >> 63 == 1 { if c[i] == 0 { 2 } else { c[i] } } else { 1 };
        }
    }
    c
}

/// Debug (env ASCII_CLASSIFY): break struct flicker cells down by what A (t-1 = t+1) and B (t) were, and
/// whether a line was in the 3x3 neighbourhood in any of the three frames.
fn classify_flicker(classes: &[(Vec<char>, Vec<u8>)], cols: usize, rows: usize) {
    const N: [&str; 3] = ["blank", "line", "mark"];
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    let mut total = 0usize;
    for t in 1..classes.len() - 1 {
        let (ca, cb, cc) = (&classes[t - 1].0, &classes[t].0, &classes[t + 1].0);
        let (a, b, c) = (&classes[t - 1].1, &classes[t].1, &classes[t + 1].1);
        for y in 0..rows {
            for x in 0..cols {
                let i = y * cols + x;
                if ca[i] != cc[i] || ca[i] == cb[i] {
                    continue;
                }
                // same class does not mean same char; count only cells where the chars differ is done via class pair,
                // so mark<->mark and line<->line swaps show up as equal-class pairs
                let near_line = [a, b, c].iter().any(|f| {
                    (y.saturating_sub(1)..=(y + 1).min(rows - 1)).any(|yy| (x.saturating_sub(1)..=(x + 1).min(cols - 1)).any(|xx| (yy, xx) != (y, x) && f[yy * cols + xx] == 1))
                });
                *counts.entry(format!("{}<->{} near_line={}", N[a[i] as usize], N[b[i] as usize], near_line)).or_default() += 1;
                total += 1;
            }
        }
    }
    println!("  flicker cells: {total}");
    for (k, v) in counts {
        println!("    {k}: {v}");
    }
}

#[allow(clippy::too_many_arguments)]
fn dump(scene: &Scene, atlas: &Atlas, opts: &Options, start: usize, n: usize, cols: usize, rows: usize, out: &str, png: bool) {
    std::fs::create_dir_all(out).unwrap_or_else(|e| fail(format!("{out}: {e}")));
    let mut r = Renderer::default();
    for i in 0..n {
        let cam = scene.bench_camera(start + i);
        let frame = r.render(scene, &cam, atlas, opts, cols, rows);
        let base = format!("{out}/frame_{i:03}");
        std::fs::write(format!("{base}.txt"), compose::to_text(&frame.cells, cols)).unwrap_or_else(|e| fail(e));
        if png {
            compose::write_png(&format!("{base}.png"), &frame.cells, cols, atlas).unwrap_or_else(|e| fail(e));
        }
    }
    eprintln!("wrote {n} frame(s) to {out}/");
}
