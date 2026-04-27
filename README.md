# Previewer

A native GNOME image and PDF viewer for Linux that lets you **annotate
documents and sign PDFs with a real saved signature** — built to feel
at home next to GTK4 + libadwaita apps and to behave like macOS Preview
when you want to mark up a contract.

![Previewer showing a sample PDF with the page thumbnails sidebar
open, the dark Adwaita theme, and stroke controls in the
toolbar.](docs/screenshots/overview.png)

## Highlights

### ✍️ Sign PDFs with a saved signature

Previewer keeps a personal **signature library** in
`$XDG_DATA_HOME/previewer/signatures/`. You can add signatures two
ways:

- **Draw with mouse / trackpad** in the built-in canvas. Strokes are
  smoothed and stored as crisp vectors.
- **Import a transparent PNG** — Previewer auto-trims to the alpha
  bounding box, so a phone-snapped sig on a white background still
  works after a quick background-removal pass.

Once a signature is in your library, click the pen icon, pick the sig,
and drag a rectangle on any page to stamp it. Drawn signatures land as
real PDF `/Ink` annotations; imported PNGs become `/Stamp`
annotations. Either way, you can resize, reposition, and re-edit them
later — and they round-trip cleanly through Acrobat, Okular, Firefox
and macOS Preview.

### 📄 PDF viewing & annotation

- Open multi-page PDFs; smooth scrolling, **fit-to-width by default**
  (the middle zoom button toggles between fit-width and 100%).
- HiDPI-aware: pages re-rasterise at the display's pixel ratio so
  text stays sharp.
- Optional **page-thumbnails sidebar** (toggle from the toolbar or
  press <kbd>F9</kbd>). Click any thumbnail to jump.
- **Full-text search** — inline search bar, highlighted matches,
  prev/next navigation. <kbd>Ctrl</kbd>+<kbd>F</kbd> focuses the
  bar.
- **Annotation tools**: rectangles, ellipses, lines, single- and
  double-headed arrows, highlighter, free-form text, ink (handled
  through the signature drawing path).
- **Live stroke controls** in the toolbar — colour, width, and
  solid / dashed / dotted style — apply to whatever shape you draw
  next *or* the shape you currently have selected.
- **Live text controls** — pick from Helvetica / Times / Courier,
  set size and colour. The inline editor matches the final glyph
  size at any zoom level.
- **Save As** with a one-time "are you sure you want to overwrite the
  original?" prompt the first time you save in place per session.
- **Undo / Redo** (<kbd>Ctrl</kbd>+<kbd>Z</kbd> / <kbd>Ctrl</kbd>+
  <kbd>Shift</kbd>+<kbd>Z</kbd>) with smart coalescing — dragging a
  size spinner up by 10 ticks counts as one undo step, not ten.
- PDFs save with **real `/Annot` objects**, not flattened pixels. They
  remain editable in any compliant viewer.

### 🖼️ Image viewing & annotation

- PNG, JPEG, WebP, and HEIC.
- Same shape / text / highlight tools as PDFs.
- Annotations persist alongside the image as a `<file>.previewer.json`
  sidecar, so the original pixels stay untouched.

### Workflow polish

- Two-tier toolbar: file / view actions on top, drawing tools below.
- Settings remembered between launches (sidebar visibility today;
  more to come).
- `Ctrl`+wheel zoom, `R` rotates 90°, click-drag to pan, click-drag
  on a handle to resize a selected shape.
- Native Wayland and X11 (via XWayland).

## Install

### Arch Linux

```sh
cd packaging/arch
makepkg -si
```

The PKGBUILD copies the worktree (or, for tagged releases, downloads
a release tarball) into the build sandbox, fetches the pinned
`libpdfium.so` via `scripts/fetch-pdfium.sh`, then builds and installs
to `/usr/bin/previewer` plus the standard XDG locations.

### Debian / Ubuntu

```sh
cargo install cargo-deb
./scripts/fetch-pdfium.sh
cargo deb -p previewer-app --no-strip
sudo dpkg -i target/debian/previewer-app_*.deb
```

Runtime dependencies declared in the package: `libgtk-4-1 (>= 4.16)`,
`libadwaita-1-0 (>= 1.6)`, `libheif1`.

### Build from source (any distro)

```sh
# System deps (Arch shown; adapt for your distro):
sudo pacman -S rustup gtk4 libadwaita libheif x265
rustup default stable

git clone https://github.com/rikvanmechelen/previewer
cd previewer
./scripts/fetch-pdfium.sh        # one-time pdfium fetch
cargo run -p previewer-app -- ~/Documents/contract.pdf
```

Detailed packaging notes: [`packaging/README.md`](packaging/README.md).

## Keyboard shortcuts

| Action | Shortcut |
|---|---|
| Open file | <kbd>Ctrl</kbd>+<kbd>O</kbd> |
| Save | <kbd>Ctrl</kbd>+<kbd>S</kbd> |
| Save As | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> |
| Find | <kbd>Ctrl</kbd>+<kbd>F</kbd> |
| Toggle sidebar | <kbd>F9</kbd> |
| Zoom in / out / 100% | <kbd>Ctrl</kbd>+<kbd>+</kbd> / <kbd>Ctrl</kbd>+<kbd>-</kbd> / <kbd>Ctrl</kbd>+<kbd>0</kbd> |
| Rotate 90° | <kbd>R</kbd> |
| Undo / Redo | <kbd>Ctrl</kbd>+<kbd>Z</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> |
| Delete selected annotation | <kbd>Delete</kbd> / <kbd>Backspace</kbd> |
| Cancel current draft | <kbd>Esc</kbd> |
| Page nav | <kbd>Page Up</kbd> / <kbd>Page Down</kbd> |

## Architecture

A six-crate Cargo workspace; only the GTK crate touches GUI code, so
~80% of logic is unit-testable without a display server.

| Crate | Role |
|---|---|
| `previewer-core` | Annotation model, geometry, undo stack, settings, sidecars. Pure Rust. |
| `previewer-image` | PNG / JPEG / WebP / HEIC decode → RGBA. |
| `previewer-render` | Cairo paint of annotations + zoom/rotate transform + selection chrome. |
| `previewer-pdf` | Wraps `pdfium-render`; lopdf post-pass for `/Ink`, `/FreeText`, `/Square`, `/Circle`, `/Line`. |
| `previewer-signature` | Stroke smoothing, PNG import with alpha-bbox auto-trim, on-disk library. |
| `previewer-app` | GTK4 + libadwaita + relm4 — the only crate with GTK code. |

PDF round-trip is the keystone: tests open a fixture PDF, write
annotations, save, re-parse with `lopdf`, and assert on the parsed
object tree (rather than diffing bytes, which would be brittle).

## Contributing

Contributions are welcome — see [`CONTRIBUTING.md`](CONTRIBUTING.md).
TL;DR: tests must pass (`cargo test --workspace`), and PRs that
change the UI need to include before / after screenshots.

## License

Previewer is released under [GPL-3.0-or-later](LICENSE).

Vendored Pdfium is BSD-3-Clause; bundled toolbar action icons are
CC0. See `LICENSE` for the full breakdown.
