use std::path::{Path, PathBuf};

use pdfium_render::prelude::*;
use previewer_core::{ArrowEnds, Color as CoreColor, Point as CorePoint, StrokeStyle};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("pdfium error: {0}")]
    Pdfium(#[from] PdfiumError),

    #[error("lopdf error: {0}")]
    Lopdf(#[from] lopdf::Error),

    #[error("annotation type not yet supported: {0}")]
    UnsupportedAnnotation(String),
}

/// An open PDF document. Held by a 'static Pdfium binding so this struct
/// itself is owned and freely movable.
pub struct Document {
    pdf: PdfDocument<'static>,
    path: PathBuf,
    /// Annotations that pdfium-render's API can't author cleanly; injected
    /// post-save via lopdf. `/Ink` and `/FreeText` (latter needs `/AP` for
    /// viewer compatibility); `/Circle` and `/Line` (no creators in 0.9).
    pub(crate) pending_ink: Vec<PendingInk>,
    pub(crate) pending_freetext: Vec<PendingFreeText>,
    pub(crate) pending_rect: Vec<PendingRect>,
    pub(crate) pending_ellipse: Vec<PendingEllipse>,
    pub(crate) pending_line: Vec<PendingLine>,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingInk {
    pub page: u32,
    pub strokes: Vec<Vec<CorePoint>>,
    pub color: CoreColor,
    pub width: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingFreeText {
    pub page: u32,
    pub bbox: previewer_core::Rect,
    pub text: String,
    pub font: previewer_core::FontSpec,
    pub color: CoreColor,
}

/// `/Square` annotation queued for the lopdf write pass. We could let
/// pdfium author this one, but routing it via lopdf lets us emit
/// `/Border [0 0 W [dash array]]` for solid / dashed / dotted in one
/// place.
#[derive(Debug, Clone)]
pub(crate) struct PendingRect {
    pub page: u32,
    pub bbox: previewer_core::Rect,
    pub stroke_color: CoreColor,
    pub stroke_width: f64,
    pub stroke_style: StrokeStyle,
    pub fill: Option<CoreColor>,
}

/// `/Circle` annotation queued for the lopdf write pass — pdfium-render 0.9
/// has no creator for it.
#[derive(Debug, Clone)]
pub(crate) struct PendingEllipse {
    pub page: u32,
    pub bbox: previewer_core::Rect,
    pub stroke_color: CoreColor,
    pub stroke_width: f64,
    pub stroke_style: StrokeStyle,
    pub fill: Option<CoreColor>,
}

/// `/Line` annotation (with optional arrowheads at either end) queued for
/// the lopdf write pass.
#[derive(Debug, Clone)]
pub(crate) struct PendingLine {
    pub page: u32,
    pub from: CorePoint,
    pub to: CorePoint,
    pub color: CoreColor,
    pub width: f64,
    pub style: StrokeStyle,
    pub ends: ArrowEnds,
}

impl Document {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref().to_path_buf();
        // Read the file into memory rather than letting pdfium keep a file
        // handle open. This is essential when we save back to the same path:
        // an open mmap/handle would race with the in-place write and corrupt
        // trailing pages.
        let bytes = std::fs::read(&path)?;
        let pdf = crate::pdfium().load_pdf_from_byte_vec(bytes, None)?;
        Ok(Self {
            pdf,
            path,
            pending_ink: Vec::new(),
            pending_freetext: Vec::new(),
            pending_rect: Vec::new(),
            pending_ellipse: Vec::new(),
            pending_line: Vec::new(),
        })
    }

    pub fn page_count(&self) -> u32 {
        self.pdf.pages().len() as u32
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Internal: expose the path for crate-private modules.
    pub(crate) fn _path_internal(&self) -> &Path {
        &self.path
    }

    /// Internal: expose the underlying pdfium handle to other modules in this
    /// crate (rendering, search). Not public outside the crate.
    pub(crate) fn inner(&self) -> &PdfDocument<'static> {
        &self.pdf
    }

    pub(crate) fn inner_mut(&mut self) -> &mut PdfDocument<'static> {
        &mut self.pdf
    }
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("path", &self.path)
            .field("page_count", &self.page_count())
            .finish()
    }
}
