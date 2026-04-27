use pdfium_render::prelude::*;
use previewer_core::Rect;

use crate::document::{Document, Error};

#[derive(Debug, Clone, PartialEq)]
pub struct TextMatch {
    pub page: u32,
    /// Bounding box in PDF points (origin bottom-left).
    pub bbox: Rect,
}

impl Document {
    /// Search every page for `query` (case-insensitive substring) and return
    /// the bounding box of each match in **image-space** (top-left origin,
    /// PDF points = pixels at native render scale).
    ///
    /// pdfium splits a single match into N segments — one per glyph run in
    /// the PDF's content stream (e.g. each kerned character can be its own
    /// segment in PDFs produced by Word / LibreOffice). We union those
    /// segments into a single bounding box per match so the caller sees one
    /// `TextMatch` per occurrence, not one per glyph.
    pub fn find_text(&self, query: &str) -> Result<Vec<TextMatch>, Error> {
        let mut matches = Vec::new();
        let pages = self.inner().pages();
        for (page_idx, page) in pages.iter().enumerate() {
            let page_h = page.height().value as f64;
            let text = page.text()?;
            let options = PdfSearchOptions::new()
                .match_case(false)
                .match_whole_word(false);
            let search = text.search(query, &options)?;
            while let Some(segments) = search.find_next() {
                // Union all segment bounds (in PDF points) into one rect.
                let mut union: Option<(f64, f64, f64, f64)> = None; // l, top, r, bottom
                for segment in segments.iter() {
                    let b = segment.bounds();
                    let l = b.left().value as f64;
                    let r = b.right().value as f64;
                    let t = b.top().value as f64;
                    let bot = b.bottom().value as f64;
                    union = Some(match union {
                        Some((cl, ct, cr, cb)) => (cl.min(l), ct.max(t), cr.max(r), cb.min(bot)),
                        None => (l, t, r, bot),
                    });
                }
                if let Some((l, t, r, bot)) = union {
                    matches.push(TextMatch {
                        page: page_idx as u32,
                        bbox: Rect::new(l, page_h - t, r - l, t - bot),
                    });
                }
            }
        }
        Ok(matches)
    }
}
