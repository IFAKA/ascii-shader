# ascii-shader design

A terminal renderer that draws 3D scenes with **lines and marks**, not brightness ramps, and that does not
flicker when the camera moves.

Sources (read for ideas; copy no code):
- Riki Yanai, "Trying to make a structural ASCII shader" (2026): https://rikiworld.com/isocontour-church/devlog.html
  Idea taken: the **typist**. Follow each line, pick characters whose ink sits on the line, and keep last frame's
  spelling of the same piece of line while it still fits (hysteresis). He measured 14.15% -> 1.77% respell.
- Garcia et al., "Coherent Mark-based Stylization of 3D Scenes at the Compositing Stage", Eurographics 2021.
  Idea taken: **anchor points** from object-space Voronoi cells read from a G-buffer. Each mark has a 3D identity,
  so it moves with the surface. Fractalization keeps screen density constant. Area weights fade marks at edges.
- Alex Harri, "ASCII characters are not pixels" (2026): glyphs have shape, compare shapes, not only ink amount.
- Acerola, "I Tried Turning Games Into Text": the common edge ASCII baseline (Sobel angle per tile).

## Pipeline per frame

```
scene (meshes, camera)
  -> raster G-buffer (CPU, SX x SY pixels per terminal cell; default 4 x 8)
       depth, object id, object-space position, view normal, light (0..1)
  -> LINES   : mesh feature edges -> visible pieces -> per-cell path segments -> typist -> line glyph per cell
  -> MARKS   : G-buffer -> anchor points (Garcia) -> one mark glyph per anchor -> cell owner
  -> COMPOSE : line glyph wins over mark glyph; empty otherwise
  -> output  : terminal (diff writes) | dump .txt/.png | bench metrics
```

### 1. G-buffer (src/raster.rs)
- Software rasterizer, triangles, perspective-correct object-space position, z-buffer. rayon over rows/tiles.
- Light: Lambert from one directional light + 0.15 ambient. Stored per pixel.
- Terminal cell aspect: a cell is about 1 wide : 2 high. The projection must use that, so a cube looks square.

### 2. Lines: feature edges + typist (src/lines.rs)
Edges come from the **mesh**, not from image edge detection. So every line has a stable id (the mesh edge index).
- Precompute per mesh: unique edges with their 1 or 2 faces. Kinds:
  - crease: dihedral angle > `crease_deg` (default 35). Cylinder facets (30 deg) are not creases.
  - boundary: one face.
  - silhouette (per frame): one adjacent face front-facing, the other back-facing.
- Visibility: sample each candidate edge every 0.25 cell, test against the depth buffer (min of 3x3 pixel
  neighbourhood + bias). Consecutive visible samples form a **piece** `(edge_id, t0, t1, screen polyline)`.
- Cell segments: clip pieces to cells. Each cell gets a list of `(edge_id, t_mid, segment in cell-local [0,1]^2)`.
- Glyph atlas (src/glyphs.rs): rasterize candidate glyphs from a real monospace font (fontdue, default
  `/System/Library/Fonts/Menlo.ttc`, flag `--font`) into a GW x GH = 8 x 16 coverage mask. Precompute each mask's
  ink pixel list and a distance transform of its ink.
- Line glyph sets (flag `--lineset`):
  - `ascii`  : `- _ | / \ ( ) < > ^ v ' , . ` + ~ = [ ] T L 7 J`
  - `box` (default): `─ │ ╱ ╲ ╳ ┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼ ╭ ╮ ╯ ╰ ▁ ▔ ▏ ▕ _ ‾ ' , . `
- Cost of glyph g in a cell (all in glyph pixel units):
  `cost = mean_ink(dist(ink pixel, nearest segment)) + mean_path(dist(path sample, nearest ink)) + prior(g)`
  Ink pixels weighted by coverage. Distances clamped at 4 px. Empty cell list -> no line glyph.
- **Spelling memory**: key = `(edge_id, round(t_mid * edge_len_in_cells))`, i.e. *this piece of this edge*.
  If the key's previous glyph has `cost <= best_cost + keep_margin` and `cost <= keep_tol`, keep it.
  So when an edge slides one cell, its spelling slides with it instead of being re-decided.
  If a cell holds pieces of several edges, the memory key is the sorted pair of the two longest edge ids.

### 3. Marks: anchor points (src/marks.rs), Garcia et al. 2021 section 4
- For each G-buffer pixel with an object (stride flag, default every pixel at 4 x 8):
  - `lr = log2(|grad p|)` where grad p is object-space position change per terminal cell (finite differences).
  - For levels `l_k = floor(lr) + k`, `k in [-n, n]` (default n = 1): frequency `f_k = f / 2^l_k`.
    Grid cell `c = floor(f_k * p)`. Search the 3x3x3 neighbours, jittered point `(c + ijk + rand(c + ijk)) / f_k`,
    keep the nearest. Anchor id = hash(object id, level, winning cell).
  - Level weight `w_k = exp(-(l_k - lr)^2 / (2 (0.4 n)^2))`, normalized over k.
  - Accumulate per anchor id: sum of w_k, sum of w_k * screen xy, sum of light.
- Anchor weight = accumulated area / (expected max area of its grid cell on screen), then smoothstep to `w_max`
  (paper eq. 1 and the Hermite step). Screen position = weighted mean of its pixels -> its terminal cell.
- Density: anchor id gives a stable random `r in [0,1)`. The anchor draws only if `darkness * density > r`
  (darkness = 1 - light). Use a hysteresis band of 0.05 per anchor id, so a mark does not blink at the threshold.
  This is dithering pinned to the object surface.
- Mark glyph: chosen by style flag `--marks`:
  - `stipple` : weight < 0.5 -> `.`, else by darkness band `. : ;` (band with hysteresis per anchor).
  - `hatch`   : orientation = screen direction of a fixed object-space tangent (or normal x light), bucketed into
    `- / | \` with hysteresis per anchor; weight < 0.5 -> `.`.
- Cell conflict: several anchors in one cell -> highest `weight + 0.2 if it owned this cell last frame` wins.

### 4. Compose and colour (src/compose.rs)
- Line glyph beats mark glyph. Output grid: `Vec<Cell { ch: char, fg: (u8,u8,u8), bold }>`.
- Colour flag `--color`: `none`; `tron` (default: black bg, lines in neon cyan or orange per object, marks dim
  teal); `light` (fg brightness from light).

### 5. Baselines (src/baseline.rs), for fair numbers
- `tone`: classic ramp ` .:-=+*#%@` from mean light per cell.
- `edge`: Acerola style. Sobel on depth and normal G-buffer, dominant angle per cell -> `- / | \`, tone fill elsewhere.

## Modes
`--mode struct` (default, lines + marks) | `lines` | `marks` | `tone` | `edge`

## Scenes (src/scene.rs)
Procedural meshes: cube, icosphere, torus, cylinder, plane, arch_rib (rectangular stone rib swept on a half circle). Scenes:
- `chapel` (default): floor, two rows of columns, arches between them, an altar box, a sphere.
- `shapes`: cube, sphere, torus on a floor.
- `--obj path` loads a Wavefront OBJ (v and f only).

## Interactive app (src/main.rs, src/term.rs)
- crossterm, alternate screen, raw mode, hidden cursor, diff writes (only changed cells), truecolour.
- Keys: WASD move, arrows look, Q/E turn, space toggles auto orbit, 1-5 mode, c colour, m marks style,
  [ ] density, h HUD, x quit. HUD line: mode, ms/frame, fps, live flicker %.
- Resize-aware.

## Headless tools
- `--dump N --out DIR`: render N frames of the bench camera path to `DIR/frame_000.txt` (+ `--png` renders the
  grid with the font and colours into PNGs, so an agent can look at it).
- `--bench`: fixed size 160 x 50, fixed camera path, 240 frames, every mode. One line per mode:
  `mode=struct ms=4.1 changed=3.2% flicker=0.4% respell=1.1%`.

## Metrics (src/metrics.rs)
- `changed`: % of cells whose char differs from the previous frame. Includes real motion.
- `flicker`: % of cells where frame t-1 == frame t+1 != frame t (A -> B -> A). This is the `( | (` crawl from the
  devlog, and it does not count honest motion.
- `respell` (struct only): over persistent ids (line memory keys, anchor ids) visible in two frames in a row, %
  whose glyph changed.
- Bench camera path: slow orbit 0.25 deg/frame plus a slow sideways move, so edges cross cells slowly (worst case
  for crawl).

## Done when (checks)
1. `make check` passes: build, clippy without warnings, unit tests.
2. `--bench`: struct flicker <= 1/3 of both `tone` and `edge` flicker; struct respell < 3%.
3. `--bench`: struct ms/frame < 25 at 160 x 50 in release on this Mac (12 cores).
4. Dumped frames read as a drawing: cube edges are straight runs of line glyphs, columns have two side lines,
   arches are curves. Checked by looking at dump .txt and .png.
