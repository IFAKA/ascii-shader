//! Interactive terminal app (DESIGN.md "Interactive app"): crossterm, alternate screen, diff writes, truecolour.
use crate::compose::{BLANK, Cell};
use crate::glyphs::Atlas;
use crate::metrics::Metrics;
use crate::render::{Mode, Options, Renderer};
use crate::scene::Scene;
use crossterm::{
    cursor, event::{self, Event, KeyCode, KeyEventKind, KeyModifiers}, execute, queue,
    style::{Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Write};
use std::time::{Duration, Instant};

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), SetAttribute(Attribute::Reset), cursor::Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn hud_cells(text: &str, cols: usize) -> Vec<Cell> {
    let mut v: Vec<Cell> = text.chars().take(cols).map(|ch| Cell { ch, fg: (255, 255, 255), bold: true }).collect();
    v.resize(cols, BLANK);
    v
}

pub fn run(scene: &Scene, atlas: &Atlas, mut opts: Options, forced: Option<(usize, usize)>) -> io::Result<()> {
    terminal::enable_raw_mode()?;
    let _guard = Guard;
    let mut out = io::BufWriter::with_capacity(1 << 20, io::stdout());
    execute!(out, EnterAlternateScreen, cursor::Hide, terminal::Clear(terminal::ClearType::All))?;

    let mut cam = scene.cam0;
    let mut renderer = Renderer::default();
    let mut metrics = Metrics::default();
    let mut screen: Vec<Cell> = vec![];
    let (mut tcols, mut trows) = (0usize, 0usize);
    let (mut auto, mut hud) = (false, true);
    let mut bench_frame = 120usize;
    let mut ms_avg = 0.0f64;
    let mut flicker_live = 0.0f64;
    let mut frames_in_window = 0;
    let mut last = Instant::now();
    let mut fps = 0.0f64;

    loop {
        let (mut c, mut r) = terminal::size()?;
        if let Some((fc, fr)) = forced {
            c = fc as u16;
            r = fr as u16 + 1;
        }
        if (c as usize, r as usize) != (tcols, trows) {
            tcols = c as usize;
            trows = r as usize;
            screen.clear();
            execute!(out, terminal::Clear(terminal::ClearType::All))?;
            metrics = Metrics::default();
        }
        let grid_rows = trows.saturating_sub(1).max(1);

        // Input (drain everything queued).
        while event::poll(Duration::ZERO)? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Release {
                    continue;
                }
                let fwd = cam.forward_flat();
                let right = cam.right();
                match k.code {
                    KeyCode::Char('x') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                    KeyCode::Char('w') => cam.pos += fwd * 0.25,
                    KeyCode::Char('s') => cam.pos -= fwd * 0.25,
                    KeyCode::Char('a') => cam.pos -= right * 0.25,
                    KeyCode::Char('d') => cam.pos += right * 0.25,
                    KeyCode::Left => cam.yaw += 0.05,
                    KeyCode::Right => cam.yaw -= 0.05,
                    KeyCode::Up => cam.pitch = (cam.pitch + 0.05).min(1.4),
                    KeyCode::Down => cam.pitch = (cam.pitch - 0.05).max(-1.4),
                    KeyCode::Char('q') => cam.yaw += 0.15,
                    KeyCode::Char('e') => cam.yaw -= 0.15,
                    KeyCode::Char(' ') => auto = !auto,
                    KeyCode::Char(d @ '1'..='5') => {
                        opts.mode = Mode::ALL[d as usize - '1' as usize];
                        metrics = Metrics::default();
                    }
                    KeyCode::Char('C') | KeyCode::Char('v') => opts.color = opts.color.next(),
                    KeyCode::Char('m') => opts.toggle_marks(),
                    KeyCode::Char('[') => opts.marks.density = (opts.marks.density - 0.1).max(0.0),
                    KeyCode::Char(']') => opts.marks.density += 0.1,
                    KeyCode::Char('h') => hud = !hud,
                    _ => {}
                }
                if k.code == KeyCode::Char('c') && !k.modifiers.contains(KeyModifiers::CONTROL) {
                    opts.color = opts.color.next();
                }
            }
        }
        if auto {
            cam = scene.bench_camera(bench_frame);
            bench_frame = if bench_frame >= 239 { 0 } else { bench_frame + 1 };
        }

        let t0 = Instant::now();
        let frame = renderer.render(scene, &cam, atlas, &opts, tcols.max(1), grid_rows);
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        ms_avg = if ms_avg == 0.0 { ms } else { ms_avg * 0.9 + ms * 0.1 };
        let chars: Vec<char> = frame.cells.iter().map(|c| c.ch).collect();
        metrics.push(&chars, frame.ids.as_deref());
        frames_in_window += 1;
        if frames_in_window >= 60 {
            flicker_live = metrics.flicker_pct();
            metrics = Metrics::default();
            frames_in_window = 0;
        }
        let dt = last.elapsed().as_secs_f64();
        last = Instant::now();
        fps = if fps == 0.0 { 1.0 / dt } else { fps * 0.9 + 0.1 / dt };

        // Build the screen buffer: frame rows, then the HUD row.
        let mut next = frame.cells;
        let hud_text = if hud {
            format!(" {} | {:.1} ms | {:.0} fps | flicker {:.2}% | {:?} d={:.1} ", opts.mode.name(), ms_avg, fps, flicker_live, opts.marks.style, opts.marks.density)
        } else {
            String::new()
        };
        next.extend(hud_cells(&hud_text, tcols));
        if screen.len() != next.len() {
            screen = vec![Cell { ch: '\u{0}', ..BLANK }; next.len()];
        }
        let mut cur: Option<(Color, bool)> = None;
        queue!(out, SetBackgroundColor(Color::Black))?;
        for (i, (n, o)) in next.iter().zip(screen.iter()).enumerate() {
            if n == o {
                continue;
            }
            queue!(out, cursor::MoveTo((i % tcols) as u16, (i / tcols) as u16))?;
            let col = Color::Rgb { r: n.fg.0, g: n.fg.1, b: n.fg.2 };
            if cur != Some((col, n.bold)) {
                queue!(out, SetAttribute(if n.bold { Attribute::Bold } else { Attribute::NormalIntensity }), SetForegroundColor(col))?;
                cur = Some((col, n.bold));
            }
            queue!(out, Print(n.ch))?;
        }
        out.flush()?;
        screen = next;

        // Cap at ~60 fps.
        let spent = t0.elapsed();
        if spent < Duration::from_millis(16) {
            std::thread::sleep(Duration::from_millis(16) - spent);
        }
    }
}
