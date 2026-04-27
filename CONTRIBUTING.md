# Contributing to Previewer

Thanks for thinking about hacking on Previewer! Pull requests are
welcome. The bar is short:

1. **All tests must pass** on `cargo test --workspace --all-features`.
2. **Any PR that changes the UI must include screenshots** — before
   and after, ideally — pasted into the PR description.

The rest of this doc is the practical setup, the conventions we ask
PRs to follow, and the quick checklist we run before merging.

## Dev setup

```sh
# Arch (adapt the package names for your distro):
sudo pacman -S rustup gtk4 libadwaita libheif x265
rustup default stable

git clone https://github.com/rikvanmechelen/previewer
cd previewer
./scripts/fetch-pdfium.sh        # one-time pdfium binary fetch
cargo run -p previewer-app       # confirm it boots
```

`./scripts/fetch-pdfium.sh` drops a pinned `libpdfium.so` into
`vendor/pdfium/lib/`. The runtime resolves it from there during dev
(see `crates/previewer-pdf/src/lib.rs`); installed packages get a
copy at `/usr/lib/previewer/libpdfium.so` instead.

## Verifying a change

Run all three before opening a PR:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

CI runs the same set on every push.

A few of the tests touch pdfium, which isn't fully thread-safe in our
usage, so they're marked `#[serial_test::serial]`. If you add a new
PDF-touching test, follow the existing pattern.

## Screenshots in UI PRs

If your PR changes anything visible (toolbar, dialog, sidebar, draw
behaviour, …), include screenshots in the PR description. **Both
before and after** is the most useful — even a 250×250 crop of the
relevant area is fine.

Tips:
- Take the screenshot at the default Adwaita dark theme so the
  before/after compare apples to apples.
- If your change is mode-dependent (e.g. zoom level, selection
  state), capture the relevant mode.
- If the change is animated or a focus interaction, a short
  screen-recording (GIF or WebM) is welcome but not required.

A representative screenshot of the current main UI lives at
`docs/screenshots/overview.png` and is what the README hero image
points at.

## Project conventions

A few patterns to keep new code consistent:

- **TDD where it's cheap.** Pure logic crates
  (`previewer-core`, `previewer-render`, `previewer-signature`) are
  tested before the implementation lands; GTK code in
  `previewer-app` is verified by manual click-through (the GTK4 UI
  test story is still immature).
- **Annotation model is the central type.** Anything you persist or
  round-trip through PDF / sidecar JSON should funnel through
  `previewer-core::Annotation`.
- **Boundary rule.** Only `previewer-app` may import `gtk` /
  `libadwaita`. Only `previewer-pdf` may import `pdfium-render` /
  `lopdf`. Everything else stays headless and fast to test.
- **Comments.** Explain *why*, not *what*. The "what" is the code
  itself; the "why" is the surprising constraint or the past
  incident.

## Commit messages

Conventional-commit-ish prefixes are appreciated but not strict:
`feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`.

For commit bodies: lead with what changed and why; if you fixed a
bug, link the symptom (a screenshot or a steps-to-reproduce) so a
future maintainer can recognise the regression if it ever comes
back.

## License

By contributing you agree that your contributions are licensed under
[GPL-3.0-or-later](LICENSE), the same license as Previewer itself.
