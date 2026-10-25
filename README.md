# paintrs

A layered paint program written in Rust with [egui](https://github.com/emilk/egui).
The same code runs as a native desktop app (macOS, Linux, Windows) and in the
browser (WebAssembly + WebGL).

**Try it in the browser:** https://ozkl.github.io/paintrs/

## Publishing

- **Web app:** every push to `main` runs the tests and, if they pass, deploys to
  GitHub Pages (`.github/workflows/pages.yml`). One-time setup:
  *Settings ▸ Pages ▸ Source: GitHub Actions*.

## Building

Requires Rust 1.95+ (`rustup`). For the web build, also install
[Trunk](https://trunkrs.dev) and the wasm target:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk
```

| Command      | What it does                                            |
|--------------|---------------------------------------------------------|
| `make run`   | Run the native app (`cargo run --release`)              |
| `make`       | Build the native release binary → `target/release/paintrs` |
| `make serve` | Web dev server with live reload → http://127.0.0.1:8080 |
| `make web`   | Web release build → `dist/` (static files, host anywhere) |
| `make test`  | Unit tests                                              |
| `make check` | Clippy for native and web                               |

The `dist/` folder is plain static files, so it can be served from GitHub Pages
or any static host. Clipboard image copy needs a secure context (HTTPS or localhost).

## Features

**Painting**
- Brush with size, hardness, opacity, flow, spacing and stroke smoothing (stabilizer)
- Pencil for pixel-perfect work (square or round, blend or overwrite)
- Eraser with hardness and strength
- Mirror painting (vertical, horizontal or both axes)
- Shift-click draws a straight line from the last stroke; Alt-click picks a color
- Right mouse button paints with the secondary color

**Shapes and fills**
- Line/Bézier curve, rectangle (rounded corners) and ellipse. Shapes stay editable
  (handles, move, style changes) until you press Enter or click elsewhere
- Outline, fill, or outline + fill; solid, dashed or dotted strokes; anti-aliasing toggle
- Bucket fill with tolerance, contiguous or global, sampling one layer or all layers
- Gradients: linear, bilinear, radial, diamond and conic; repeat or mirror;
  primary → secondary or primary → transparent; handles stay editable
- Text with live preview: any bundled or loaded TTF/OTF font, size, alignment, line spacing

**Selections and transforms**
- Rectangle, ellipse, freehand lasso, polygon lasso and magic wand
- Replace / add (Shift) / subtract (Alt) / intersect (Shift+Alt)
- Select all, invert, layer pixels, grow, shrink, feather; anti-aliased soft selections
- Every paint tool is clipped to the selection
- Move / free transform: move, scale (Shift keeps ratio), rotate (Shift snaps to 15°),
  flip, numeric entry, arrow-key nudging, smooth or nearest-neighbor resampling

**Layers**
- Unlimited layers with thumbnails, rename (double-click), visibility, lock and alpha lock
- 12 blend modes (multiply, screen, overlay, soft light, …) and opacity
- New, duplicate, delete, reorder, merge down, merge visible, flatten

**Image**
- Canvas size with a 9-point anchor, resize (nearest, bilinear, bicubic, Lanczos)
- Crop to selection, trim transparent edges, rotate 90°/180°, flip
- Filters with live preview: brightness/contrast, hue/saturation, invert,
  desaturate, sepia, posterize, threshold, Gaussian blur, sharpen, pixelate,
  noise and edge detection. Ctrl/Cmd+F repeats the last filter

**Workflow**
- Undo/redo for everything, including canvas resizes and selections.
  Memory-bounded, plus a history panel to jump to any step
- Zoom to cursor (wheel, pinch, Ctrl+scroll), pan (Space, middle mouse,
  two-finger scroll), fit / 100%, pixel grid at high zoom, custom grid
- Rulers with draggable guides (drag out of a ruler; drag back onto it to delete)
- Color picker (HSV, hex, alpha), palette (editable), recent colors, swap/reset
- Copy, cut, paste and copy merged through the system clipboard; pasted images
  land on a new layer, ready to transform
- Drag and drop images (added as layers), projects, fonts and scripts onto the window
- Dark, light or system theme; preferences are remembered
- Warns before losing unsaved work (desktop dialog, browser tab-close prompt)

**Files**
- Open PNG, JPEG, WebP, BMP, TGA, GIF, ICO
- Export PNG, JPEG (quality), WebP (lossless), BMP, TGA
- `.paintrs` project format keeps layers, names, opacity, blend modes, locks and the selection
- Imports qpaint `.qpif` files

## Scripting

*Script → Script Editor* runs [Rhai](https://rhai.rs) scripts. Rhai is a small
JavaScript-like language written in pure Rust, so scripts behave the same on
desktop and web. The function names match qpaint's API (`fillRect`, `setPixel`,
`getCanvasWidth`, `getLayerCount`, `isInSelection`, `Color(...)`, …), and paintrs
adds shapes, text, pixel reads, layers and selection control.

A script run is one undo step. An operation limit stops runaway loops from
freezing the app.

```rust
// Checkerboard
let cell = 32;
for y in range(0, height(), cell) {
    for x in range(0, width(), cell) {
        let c = if ((x / cell) + (y / cell)) % 2 == 0 { rgb(255, 255, 255) } else { rgb(0, 0, 0) };
        fillRect(x, y, cell, cell, c);
    }
}
```

The full API reference is in the script editor.

## Keyboard shortcuts

| Tool | Key | | Action | Shortcut |
|---|---|---|---|---|
| Move / Transform | V | | Undo / Redo | Ctrl+Z / Ctrl+Shift+Z (or Ctrl+Y) |
| Rectangle select | M | | Copy / Cut / Paste | Ctrl+C / Ctrl+X / Ctrl+V |
| Ellipse select | J | | Copy merged | Ctrl+Shift+C |
| Lasso | K | | Free transform | Ctrl+T |
| Magic wand | W | | Select all / Deselect / Invert | Ctrl+A / Ctrl+D / Ctrl+Shift+I |
| Brush | B | | New layer / Duplicate / Merge down | Ctrl+Shift+N / Ctrl+J / Ctrl+E |
| Pencil | N | | Fill with primary / secondary | Alt+Backspace / Ctrl+Backspace |
| Eraser | E | | Clear selected area | Delete |
| Fill | F | | Zoom in / out / fit / 100% | Ctrl+= / Ctrl+- / Ctrl+0 / Ctrl+1 |
| Gradient | G | | Toggle grid | Ctrl+' |
| Color picker | I | | Brush size | [ and ] |
| Line / Curve | L | | Swap / reset colors | X / D |
| Rectangle | R | | Apply / cancel pending edit | Enter / Esc |
| Ellipse | O | | New / Open / Save / Export | Ctrl+N / Ctrl+O / Ctrl+S / Ctrl+Shift+E |
| Text | T | | | |
| Pan | H (or hold Space) | | | |
| Zoom | Z | | | |

On macOS, use Cmd instead of Ctrl.

## How it works

- **Pixels live on the CPU** as straight-alpha RGBA per layer. Only the changed
  rectangle is recomposited and uploaded into 512×512 GPU tiles, which stay under
  WebGL texture limits. (qpaint kept layers on the GPU, which made undo and pixel
  reads expensive.)
- **Tools draw coverage, not pixels.** Every tool rasterizes into a float coverage
  buffer, and one edit session turns that into pixels from the layer's pre-edit
  state. So stroke opacity stays even where dabs overlap, selections, alpha lock
  and erasing work the same way for every tool, and in-progress shapes can be
  redrawn or cancelled.
- **Undo** stores before/after patches of just the touched rectangle for pixel
  edits, and copy-on-write snapshots (layers are `Arc`-shared) for structural
  edits. History is capped by memory, not a fixed step count.

```
src/
  main.rs       native and web entry points
  app.rs        app state, actions, shortcuts, canvas input
  ui.rs         menus, panels, dialogs, script window
  view.rs       zoom/pan, tiled textures, grid, marching ants
  doc.rs        layers, blend modes, compositing, edit sessions
  history.rs    undo/redo
  raster.rs     images, masks, coverage rasterizers (AA polygons, strokes, dabs)
  tools/        brush/pencil/eraser, shapes, fill/gradient, selection, transform, text, picker
  filters.rs    adjustments and filters
  ops.rs        rotate/flip/resize/canvas size
  io.rs         image codecs, .paintrs projects, .qpif import
  text.rs       font loading and text rasterization
  script.rs     Rhai scripting API
  platform.rs   file dialogs, downloads, clipboard, unsaved-changes guard
  icons.rs      tool icons
```

## License

MIT — see [LICENSE](LICENSE). Tool icons come from qpaint's icon atlas.

Licenses of all bundled dependencies are in
[THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt), which is also shown in the
app under *Help ▸ Third-Party Licenses*. After changing dependencies, regenerate it
with `make licenses` (needs `cargo install cargo-about --features cli`). The
allowed licenses are listed in `about.toml`, and generation fails if a new
dependency uses anything else.
