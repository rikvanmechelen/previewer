use std::path::Path;

use pdfium_render::prelude::*;
use previewer_core::{Annotation, Color as CoreColor, Rect as CoreRect};

use crate::document::{Document, Error, PendingEllipse, PendingLine, PendingRect};

impl Document {
    /// Add an annotation to the document. Coordinates in `ann` are
    /// **image-space** (top-left origin); we flip Y here to convert to
    /// PDF user space (bottom-left origin).
    ///
    /// Currently supports `Rect` (→ `/Square`), `Highlight`, and `FreeText`
    /// via pdfium's native annotation API. `Ellipse` (`/Circle`) and
    /// `Arrow` (`/Line`) are not yet supported (pdfium-render 0.9 lacks
    /// creation methods for them); future work will inject those via lopdf
    /// post-processing.
    pub fn add_annotation(&mut self, ann: &Annotation) -> Result<(), Error> {
        let page_idx = ann.page() as i32;

        // FreeText needs a font token, which requires a separate &mut doc
        // borrow that conflicts with the page borrow below. Pre-fetch.
        let helvetica = if matches!(ann, Annotation::FreeText { .. }) {
            Some(self.inner_mut().fonts_mut().helvetica())
        } else {
            None
        };

        let pages = self.inner_mut().pages_mut();
        let mut page = pages.get(page_idx)?;
        let page_h = page.height().value as f64;
        let annots = &mut page.annotations_mut();

        match ann {
            Annotation::Rect {
                page,
                bbox,
                stroke,
                fill,
            } => {
                // Route through lopdf so we can emit `/Border` with the
                // dash array — pdfium's `create_square_annotation` can't.
                self.pending_rect.push(PendingRect {
                    page: *page,
                    bbox: *bbox,
                    stroke_color: stroke.color,
                    stroke_width: stroke.width,
                    stroke_style: stroke.style,
                    fill: *fill,
                });
                return Ok(());
            }
            Annotation::Highlight { bbox, color, .. } => {
                let pdf_rect = image_to_pdf_rect(*bbox, page_h);
                let mut a = annots.create_highlight_annotation()?;
                a.set_bounds(pdf_rect)?;
                // /Highlight needs /QuadPoints in **top-left, top-right,
                // bottom-left, bottom-right** order (Adobe / Okular
                // convention). pdfium-render's `PdfQuadPoints::from_rect`
                // emits BL, BR, TR, TL (counterclockwise) which Okular
                // refuses to render. Build the quad ourselves.
                let quad = PdfQuadPoints::new(
                    pdf_rect.left(),
                    pdf_rect.top(),
                    pdf_rect.right(),
                    pdf_rect.top(),
                    pdf_rect.left(),
                    pdf_rect.bottom(),
                    pdf_rect.right(),
                    pdf_rect.bottom(),
                );
                a.attachment_points_mut()
                    .create_attachment_point_at_end(quad)?;
                let _ = a.set_stroke_color(core_to_pdf_color(*color));
            }
            Annotation::FreeText {
                page,
                position,
                text,
                font,
                color,
                is_placeholder,
            } => {
                // Skip placeholders — these are an in-app UX hint
                // ("Enter some text") and shouldn't bake into the saved PDF.
                if *is_placeholder {
                    return Ok(());
                }
                // Queue for the lopdf pass. We write a real `/FreeText`
                // annotation with an `/AP` Form XObject so viewers (Okular
                // included) render the text reliably AND we can extract it
                // back on next open for further editing.
                let _ = (helvetica, &annots); // unused for FreeText
                let (w, h) = previewer_render::freetext_bbox_size(text, font);
                self.pending_freetext
                    .push(crate::document::PendingFreeText {
                        page: *page,
                        bbox: CoreRect::new(position.x, position.y, w, h),
                        text: text.clone(),
                        font: font.clone(),
                        color: *color,
                    });
                return Ok(());
            }
            Annotation::Ellipse {
                page,
                bbox,
                stroke,
                fill,
            } => {
                // pdfium-render 0.9 has no `/Circle` creator; queue for lopdf.
                self.pending_ellipse.push(PendingEllipse {
                    page: *page,
                    bbox: *bbox,
                    stroke_color: stroke.color,
                    stroke_width: stroke.width,
                    stroke_style: stroke.style,
                    fill: *fill,
                });
                return Ok(());
            }
            Annotation::Arrow {
                page,
                from,
                to,
                stroke,
                ends,
            } => {
                // Same story — no `/Line` creator in pdfium-render 0.9.
                self.pending_line.push(PendingLine {
                    page: *page,
                    from: *from,
                    to: *to,
                    color: stroke.color,
                    width: stroke.width,
                    style: stroke.style,
                    ends: *ends,
                });
                return Ok(());
            }
            Annotation::Stamp { bbox, image, .. } => {
                let pdf_rect = image_to_pdf_rect(*bbox, page_h);
                let mut a = annots.create_stamp_annotation()?;
                a.set_bounds(pdf_rect)?;

                // Build a `DynamicImage` from the RGBA bytes.
                let rgba =
                    image::RgbaImage::from_raw(image.width, image.height, image.pixels.clone())
                        .ok_or_else(|| {
                            Error::UnsupportedAnnotation(
                                "stamp pixel buffer size doesn't match width × height".into(),
                            )
                        })?;
                let dyn_img = image::DynamicImage::ImageRgba8(rgba);

                // PDF origin is bottom-left, so the image's lower-left maps
                // to (bbox.x, page_h - bbox.y - bbox.height).
                let llx = PdfPoints::new(bbox.x as f32);
                let lly = PdfPoints::new((page_h - bbox.y - bbox.height) as f32);
                let w_pt = PdfPoints::new(bbox.width as f32);
                let h_pt = PdfPoints::new(bbox.height as f32);
                a.objects_mut()
                    .create_image_object(llx, lly, &dyn_img, Some(w_pt), Some(h_pt))?;
            }
            Annotation::Ink {
                page,
                strokes,
                color,
                width,
            } => {
                // pdfium-render 0.9 doesn't expose `FPDFAnnot_AddInkStroke`
                // through its high-level API, so queue ink for the lopdf pass
                // that runs in `Document::save`.
                self.pending_ink.push(crate::document::PendingInk {
                    page: *page,
                    strokes: strokes.clone(),
                    color: *color,
                    width: *width,
                });
                return Ok(());
            }
        }
        Ok(())
    }

    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        let path = path.as_ref();
        // Pass 1: pdfium writes everything it natively supports.
        self.inner().save_to_file(path)?;
        // Pass 2: lopdf injects annotations pdfium's API can't author cleanly.
        let pending_ink = std::mem::take(&mut self.pending_ink);
        let pending_freetext = std::mem::take(&mut self.pending_freetext);
        let pending_rect = std::mem::take(&mut self.pending_rect);
        let pending_ellipse = std::mem::take(&mut self.pending_ellipse);
        let pending_line = std::mem::take(&mut self.pending_line);
        crate::lopdf_pass::inject(
            path,
            &pending_ink,
            &pending_freetext,
            &pending_rect,
            &pending_ellipse,
            &pending_line,
        )?;
        Ok(())
    }
}

fn image_to_pdf_rect(bbox: CoreRect, page_h: f64) -> PdfRect {
    let left = bbox.x as f32;
    let right = (bbox.x + bbox.width) as f32;
    let top = (page_h - bbox.y) as f32;
    let bottom = (page_h - bbox.y - bbox.height) as f32;
    PdfRect::new(
        PdfPoints::new(bottom),
        PdfPoints::new(left),
        PdfPoints::new(top),
        PdfPoints::new(right),
    )
}

fn core_to_pdf_color(c: CoreColor) -> PdfColor {
    PdfColor::new(c.r, c.g, c.b, c.a)
}
