# ascii-shader

![demo](docs/demo.gif)

A Rust terminal renderer that draws 3D scenes with thin lines and small marks instead of brightness ramps like `.:-=+*#`. It spells the edges of a mesh with line glyphs (`|`, `_`, `/`, box-drawing characters) and keeps a memory of each spelling, so the picture stays calm when the camera moves.

## Run

```
cargo run --release
```

Keys: `W A S D` move, arrows look, `Q`/`E` turn, `space` toggles auto orbit, `1`-`5` mode
(struct, lines, marks, tone, edge), `c` or `v` colour, `m` marks style, `[` `]` mark density,
`h` HUD (mode, ms per frame, fps, live flicker), `x` or `Esc` quit.

Useful flags (see `--help` for all):
- `--mode struct|lines|marks|tone|edge`
- `--scene chapel|shapes`, or `--obj file.obj` (Wavefront, `v` and `f` only)
- `--font`, `--lineset`, `--color`, `--density`
- `--dump N --out DIR [--png] [--dump-start K]` writes frames of the bench camera path as text and PNG
- `--bench` runs every mode for 240 frames at 160x50 and prints metrics (`make bench`)

## How it works

Full spec in [DESIGN.md](DESIGN.md).
- A CPU rasterizer makes a G-buffer (depth, object, triangle, position, normal, light).
- Feature edges of the mesh are projected and a "typist" picks one glyph per cell to spell each edge.
- The typist has memory: every edge keeps its spelling from frame to frame, and only changes it when the new choice is clearly better (hysteresis).
- Nearly horizontal or vertical edges are spelled as runs, one cell per column or row, so they slide instead of crawl.
- Shading comes from anchor marks: points fixed in 3D space (after Garcia et al.), drawn as stipple or other marks where the light needs them. They move with the surface, not with the screen.
- Flicker is measured: a cell flickers when frame t-1 equals frame t+1 but frame t differs (A, B, A).

## Results

Chapel scene, 160x50, 240 frames of a slow orbit, `make bench`:

| mode | flicker | changed |
|------|---------|---------|
| struct (lines + marks) | 0.027% | 4.35% |
| tone (brightness ramp) | 0.188% | 3.81% |
| edge (edge detection) | 0.320% | 3.34% |

Struct flickers about 7x less than tone and 12x less than edge. Frame time is under 10 ms on one machine (Apple silicon).

![still](docs/still1.png)

## Credits

Ideas from:
- Riki Yanai, isocontour-church devlog: https://rikiworld.com/isocontour-church/devlog.html
- Garcia et al., "Coherent Mark-based Stylization of 3D Scenes at the Compositing Stage", Eurographics 2021.

No code was copied from either source. Ideas only.

## License

MIT, see [LICENSE](LICENSE).
