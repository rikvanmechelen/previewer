//! previewer-pdf: open, render, annotate, and save PDFs.
//!
//! Wraps `pdfium-render` (Google's Pdfium, BSD-3) for rendering and
//! annotation creation. M3 covers open + page_count + render + search;
//! annotation editing arrives in M4.
//!
//! ## Pdfium binding
//!
//! `libpdfium.so` is not packaged on Arch or Debian, so the workspace vendors
//! a known-good build at `vendor/pdfium/lib/libpdfium.so` (fetched by
//! `scripts/fetch-pdfium.sh`). At runtime we resolve the path in this order:
//!
//! 1. `PDFIUM_DYNAMIC_LIB_PATH` env var (directory containing libpdfium.so),
//! 2. workspace-relative `vendor/pdfium/lib/libpdfium.so` (dev/test),
//! 3. packaged install at `/usr/lib/previewer/libpdfium.so` (M7 .deb / Arch),
//! 4. system search path (`Pdfium::default_bindings()`).

mod annotate;
mod document;
mod extract;
mod lopdf_pass;
mod render;
mod search;

pub use document::{Document, Error};
pub use render::RenderedPage;
pub use search::TextMatch;

use std::path::PathBuf;
use std::sync::OnceLock;

use pdfium_render::prelude::Pdfium;

static PDFIUM: OnceLock<Pdfium> = OnceLock::new();

/// Lazily-initialised global `Pdfium` binding. Documents borrow from it for
/// their `'static` lifetime.
pub fn pdfium() -> &'static Pdfium {
    PDFIUM.get_or_init(|| {
        let path = resolve_pdfium_path();
        tracing::info!(path = %path.display(), "loading libpdfium");
        let bindings = Pdfium::bind_to_library(&path).expect("failed to bind to libpdfium.so");
        Pdfium::new(bindings)
    })
}

fn resolve_pdfium_path() -> PathBuf {
    if let Ok(dir) = std::env::var("PDFIUM_DYNAMIC_LIB_PATH") {
        let p = PathBuf::from(dir).join("libpdfium.so");
        if p.exists() {
            return p;
        }
    }

    // Workspace-relative fallback: crates/previewer-pdf is two levels deep,
    // so the workspace root is two parent()s up from CARGO_MANIFEST_DIR.
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let vendored = workspace_root.join("vendor/pdfium/lib/libpdfium.so");
    if vendored.exists() {
        return vendored;
    }

    // Packaged install path: the .deb / PKGBUILD drops libpdfium.so under
    // `/usr/lib/previewer/` since neither distro ships pdfium as a system
    // package. We check it explicitly so we don't have to set
    // LD_LIBRARY_PATH from the launcher.
    let packaged = PathBuf::from("/usr/lib/previewer/libpdfium.so");
    if packaged.exists() {
        return packaged;
    }

    // Final fallback: just use the bare name and let the loader search
    // LD_LIBRARY_PATH / system paths.
    PathBuf::from("libpdfium.so")
}
