# Packaging

Build distribution-ready packages for Previewer.

Both flavours bundle the vendored `libpdfium.so` to
`/usr/lib/previewer/libpdfium.so` and rely on the `resolve_pdfium_path`
fallback in `crates/previewer-pdf/src/lib.rs` to locate it at runtime —
so end users don't have to set `LD_LIBRARY_PATH`.

## Prerequisites for either flavour

```bash
./scripts/fetch-pdfium.sh        # populates vendor/pdfium/lib/libpdfium.so
cargo build --release -p previewer-app
```

## Debian / Ubuntu (.deb)

Install [`cargo-deb`](https://github.com/kornelski/cargo-deb) once:

```bash
cargo install cargo-deb
```

Build the .deb (run from the workspace root):

```bash
cargo deb -p previewer-app --no-strip
```

`--no-strip` lets the workspace's `[profile.release] strip = "symbols"`
do its thing without `cargo deb` re-stripping. The resulting package
lands at `target/debian/previewer-app_<version>_<arch>.deb`.

Install for testing in a clean Debian Trixie VM:

```bash
sudo dpkg -i target/debian/previewer-app_*.deb \
    || sudo apt-get install -f         # let apt resolve runtime deps
previewer --version
previewer ~/Documents/contract.pdf
```

Runtime deps declared in `crates/previewer-app/Cargo.toml`'s
`[package.metadata.deb]` section: `libgtk-4-1 >= 4.16`, `libadwaita-1-0
>= 1.6`, `libheif1`, plus the auto-detected `${shlibs:Depends}` /
`${misc:Depends}`.

## Arch Linux (PKGBUILD)

```bash
cd packaging/arch
makepkg -si      # builds + installs
```

The PKGBUILD has an empty `source=()` and copies the worktree this file
lives in into `$srcdir` via `rsync` in `prepare()`, so neither a git
repository nor a release tarball is required for local builds — handy
for iterating before tagging. For a published release, replace
`source=()` with the tagged tarball URL and drop the rsync step.

`makepkg` runs `scripts/fetch-pdfium.sh` if the vendored `.so` is
missing, so a fresh checkout works without manual setup. The package
declares runtime deps `gtk4 libadwaita libheif`.

## What gets installed

| Path                                                              | Source                                                  |
|-------------------------------------------------------------------|---------------------------------------------------------|
| `/usr/bin/previewer`                                              | `target/release/previewer`                              |
| `/usr/lib/previewer/libpdfium.so`                                 | `vendor/pdfium/lib/libpdfium.so`                        |
| `/usr/share/applications/org.moma.Previewer.desktop`              | `data/org.moma.Previewer.desktop`                       |
| `/usr/share/icons/hicolor/scalable/apps/org.moma.Previewer.svg`   | `data/icons/hicolor/scalable/apps/`                     |
| `/usr/share/icons/hicolor/scalable/actions/previewer-*.svg`       | `data/icons/hicolor/scalable/actions/`                  |
| `/usr/share/licenses/previewer/LICENSE` (Arch)                    | `LICENSE`                                               |

## Smoke test

After install, on a system that's never run Previewer:

```bash
previewer --version          # binary on PATH
gtk-launch org.moma.Previewer # via .desktop entry
previewer ~/Pictures/x.png   # CLI argument is honoured
previewer ~/Documents/x.pdf  # PDF works → libpdfium resolved correctly
```

In `RUST_LOG=previewer=debug previewer …`, the first
`loading libpdfium path=…` log line confirms which `.so` was picked up.
For a packaged install it should read `/usr/lib/previewer/libpdfium.so`.
