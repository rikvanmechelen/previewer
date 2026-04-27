//! Test fixture builders. Generates PDF files programmatically using pdfium
//! itself so the repo stays free of binary blobs.
//!
//! Cargo compiles each integration test file as a separate binary, so a
//! helper used by only some of them looks unused per-binary; `dead_code` is
//! allowed here.

#![allow(dead_code)]

use std::path::Path;

use pdfium_render::prelude::*;
use previewer_pdf::pdfium;

pub fn write_blank_pdf(path: &Path, pages: u32) {
    let pdfium = pdfium();
    let mut doc = pdfium.create_new_pdf().unwrap();
    let doc_pages = doc.pages_mut();
    for _ in 0..pages {
        doc_pages
            .create_page_at_end(PdfPagePaperSize::a4())
            .unwrap();
    }
    doc.save_to_file(path).unwrap();
}

/// PDF with `pages` blank pages plus one extra page containing `marker_text`.
/// The marker text is sized large enough that pdfium's text extraction picks
/// it up reliably.
pub fn write_pdf_with_text(path: &Path, blank_pages: u32, marker_text: &str) {
    let pdfium = pdfium();
    let mut doc = pdfium.create_new_pdf().unwrap();
    {
        let doc_pages = doc.pages_mut();
        for _ in 0..blank_pages {
            doc_pages
                .create_page_at_end(PdfPagePaperSize::a4())
                .unwrap();
        }
        let mut page = doc_pages
            .create_page_at_end(PdfPagePaperSize::a4())
            .unwrap();
        let font = doc.fonts_mut().helvetica();
        let mut text_obj =
            PdfPageTextObject::new(&doc, marker_text, font, PdfPoints::new(36.0)).unwrap();
        text_obj
            .translate(PdfPoints::new(72.0), PdfPoints::new(720.0))
            .unwrap();
        page.objects_mut().add_text_object(text_obj).unwrap();
    }
    doc.save_to_file(path).unwrap();
}
