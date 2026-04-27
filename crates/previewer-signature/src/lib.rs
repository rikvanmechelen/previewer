//! previewer-signature: signature data model, stroke smoothing, and PNG import.
//!
//! Two flavours of signature live alongside each other:
//!
//! - **Vector**: drawn in the app, stored as a list of strokes (each stroke is
//!   a list of `(x, y, pressure)` points). Renders crisp at any size and saves
//!   into PDFs as `/Ink` annotations later (M6).
//! - **Raster**: imported from a PNG with alpha. Saved as `/Stamp` annotations
//!   later (M6).

mod import;
mod library;
mod stroke;

pub use import::{ImportError, ImportOptions, import_png_signature};
pub use library::{Library, LibraryError};
pub use stroke::{Signature, SignatureId, SignatureKind, Stroke, StrokePoint};
