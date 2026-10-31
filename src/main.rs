//! ascii-shader: a terminal renderer that draws 3D scenes with lines and marks. See DESIGN.md.
#![allow(dead_code)] // public API surface is built ahead of its users (lines/marks agents)
mod baseline;
mod compose;
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
    #[arg(long, value_enum, default_value = "stipple")]
    marks: MarkStyle,
    #[arg(long, value_enum, default_value = "tron")]
    color: ColorMode,
    /// Render N frames of the bench camera path and write them to --out
    #[arg(long)]
    dump: Option<usize>,
    #[arg(long, default_value = "out")]
    out: String,
    /// With --dump: also write PNGs
    #[arg(long)]
    png: bool,
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
    opts.lines.keep_margin = cli.keep_margin;
    opts.lines.keep_tol = cli.keep_tol;
    opts.marks.style = cli.marks;
    opts.marks.density = cli.density;
    opts.marks.f = cli.f;
    opts.marks.n_levels = cli.levels;
    opts.marks.w_max = cli.w_max;
    opts.marks.stride = cli.stride.max(1);

    if cli.bench {
        bench(&scene, &atlas, &opts, cli.cols.unwrap_or(160), cli.rows.unwrap_or(50));
    } else if let Some(n) = cli.dump {
        dump(&scene, &atlas, &opts, n, cli.cols.unwrap_or(160), cli.rows.unwrap_or(50), &cli.out, cli.png);
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
        for f in 0..240 {
            let cam = scene.bench_camera(f);
            let t = Instant::now();
            let frame = r.render(scene, &cam, atlas, &opts, cols, rows);
            total += t.elapsed().as_secs_f64() * 1000.0;
            let chars: Vec<char> = frame.cells.iter().map(|c| c.ch).collect();
            m.push(&chars, frame.ids.as_deref());
        }
        let respell = if m.has_ids { format!("{:.2}%", m.respell_pct()) } else { "n/a".into() };
        println!("mode={} ms={:.2} changed={:.2}% flicker={:.2}% respell={}", mode.name(), total / 240.0, m.changed_pct(), m.flicker_pct(), respell);
    }
}

#[allow(clippy::too_many_arguments)]
fn dump(scene: &Scene, atlas: &Atlas, opts: &Options, n: usize, cols: usize, rows: usize, out: &str, png: bool) {
    std::fs::create_dir_all(out).unwrap_or_else(|e| fail(format!("{out}: {e}")));
    let mut r = Renderer::default();
    for i in 0..n {
        let cam = scene.bench_camera(120 + i);
        let frame = r.render(scene, &cam, atlas, opts, cols, rows);
        let base = format!("{out}/frame_{i:03}");
        std::fs::write(format!("{base}.txt"), compose::to_text(&frame.cells, cols)).unwrap_or_else(|e| fail(e));
        if png {
            compose::write_png(&format!("{base}.png"), &frame.cells, cols, atlas).unwrap_or_else(|e| fail(e));
        }
    }
    eprintln!("wrote {n} frame(s) to {out}/");
}
