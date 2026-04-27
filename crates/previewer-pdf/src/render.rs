use pdfium_render::prelude::*;

use crate::document::{Document, Error};

/// An owned RGBA8 page raster. Mirrors the shape of
/// `previewer_image::DecodedImage` so the app can plumb both into the same
/// Cairo paint path.
#[derive(Debug, Clone)]
pub struct RenderedPage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl RenderedPage {
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

impl Document {
    /// Render a page at the given scale (1.0 = native PDF points). Returns an
    /// owned RGBA8 buffer.
    pub fn render_page(&self, page_index: u32, scale: f64) -> Result<RenderedPage, Error> {
        let pages = self.inner().pages();
        let page = pages.get(page_index as i32)?;

        let target_w = (page.width().value as f64 * scale).round().max(1.0) as i32;
        let target_h = (page.height().value as f64 * scale).round().max(1.0) as i32;
        let config = PdfRenderConfig::new()
            .set_target_width(target_w)
            .set_target_height(target_h);
        let bitmap = page.render_with_config(&config)?;

        let width = bitmap.width() as u32;
        let height = bitmap.height() as u32;
        // pdfium-render normalises whatever bitmap format pdfium chose into
        // straight RGBA8 — works across BGRA / BGR / etc. without us guessing.
        let pixels = bitmap.as_rgba_bytes();

        Ok(RenderedPage {
            width,
            height,
            pixels,
        })
    }
}
