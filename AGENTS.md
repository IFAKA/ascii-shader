# ascii-shader

Rust terminal renderer that draws 3D scenes with lines and marks (not brightness ramps) and does not flicker
when the camera moves. Full spec: `DESIGN.md`.

- Check (build, clippy -D warnings, tests): `make check`
- Bench (every mode, 160x50, 240 frames, one line each): `make bench`
- Look at output: `make dump` writes `out/frame_*.txt` and `.png` (out/ is gitignored); read the PNG.
- Rule: copy no code from the sources named in DESIGN.md; ideas only.

## Modules (src/)
- `scene.rs` meshes, chapel/shapes scenes, OBJ loader, `Camera` (cell aspect 1:2)
- `raster.rs` CPU G-buffer (`GBuffer`: depth, obj, tri, object-space pos, view normal, light)
- `glyphs.rs` font atlas: 8x16 masks, ink list, distance field; line sets
- `lines.rs` feature edges + typist (DESIGN section 2; stub until implemented)
- `marks.rs` anchor-point marks (DESIGN section 3; stub until implemented)
- `baseline.rs` tone and edge baselines; `compose.rs` merge + colour + PNG; `metrics.rs` changed/flicker/respell
- `render.rs` one-frame pipeline; `term.rs` interactive app; `main.rs` flags, `--bench`, `--dump`
