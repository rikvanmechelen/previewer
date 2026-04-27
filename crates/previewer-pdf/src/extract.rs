//! Extract existing PDF annotations into the editable
//! `previewer_core::Annotation` model. `/Square` and `/Highlight` come
//! from pdfium-render's iterator; `/FreeText` is parsed via lopdf because
//! pdfium's accessors crash on annotations authored by older sessions.
//! Subtypes we don't handle (Stamp, Ink, Circle, etc.) are left intact in
//! the pdfium document so a re-save preserves them.

use pdfium_render::prelude::*;
use previewer_core::{Annotation, ArrowEnds, Color, FontSpec, Point, Rect, Stroke, StrokeStyle};

use crate::Document;

impl Document {
    /// Extract all annotations the editor knows how to handle, **and remove
    /// them from the pdfium document** so re-saving doesn't duplicate them.
    /// Subtypes we don't extract are left alone — pdfium's save preserves
    /// them as-is.
    pub fn extract_annotations(&mut self) -> Vec<Annotation> {
        let mut out = Vec::new();
        let mut deletions: Vec<(usize, Vec<usize>)> = Vec::new();

        // Pull /FreeText, /Circle and /Line annotations via lopdf —
        // pdfium-render 0.9 has no creators for /Circle and /Line (so we
        // can't round-trip them through pdfium's API), and some FreeTexts
        // authored by older sessions segfault inside pdfium's accessors.
        // lopdf just parses dict bytes, so it's the safe path.
        let path = self.path().to_path_buf();
        out.extend(read_freetext_via_lopdf(&path));
        out.extend(read_rect_via_lopdf(&path));
        out.extend(read_ellipse_via_lopdf(&path));
        out.extend(read_line_via_lopdf(&path));

        // Read pass — collect extracted annotations and per-page indices to
        // delete. We don't mutate during iteration.
        {
            let pages = self.inner().pages();
            let total_pages = pages.len();
            tracing::debug!(total_pages, "extract: scanning pages");
            for page_idx in 0..total_pages {
                let Ok(page) = pages.get(page_idx) else {
                    tracing::warn!(page_idx, "extract: get page failed");
                    continue;
                };
                let page_h = page.height().value as f64;
                let annots = page.annotations();
                let len = annots.len();
                tracing::debug!(page_idx, annots = len, "extract: page has annotations");
                let mut indices: Vec<usize> = Vec::new();
                for annot_idx in 0..len {
                    tracing::debug!(page_idx, annot_idx, "extract: pre-get");
                    let Ok(ann) = annots.get(annot_idx) else {
                        tracing::debug!(page_idx, annot_idx, "extract: get failed");
                        continue;
                    };
                    let kind = annotation_kind_label(&ann);
                    tracing::debug!(page_idx, annot_idx, kind, "extract: got");
                    // FreeText / Square / Circle / "Unsupported" (covers
                    // /Line): we already pulled them via lopdf above (via
                    // `read_rect_via_lopdf`, `read_ellipse_via_lopdf`,
                    // `read_line_via_lopdf`, `read_freetext_via_lopdf`).
                    // Queue for deletion so pdfium's save doesn't duplicate
                    // them. We avoid touching their accessors directly —
                    // pdfium's FreeText accessors segfault on older
                    // annotations, and Circle/Line round-trip data lives in
                    // /Border which pdfium-render can't author anyway.
                    if matches!(
                        ann,
                        PdfPageAnnotation::FreeText(_)
                            | PdfPageAnnotation::Square(_)
                            | PdfPageAnnotation::Circle(_)
                            | PdfPageAnnotation::Unsupported(_)
                    ) {
                        indices.push(annot_idx);
                    } else if let Some(extracted) =
                        annotation_to_core(&ann, page_idx as u32, page_h)
                    {
                        out.push(extracted);
                        indices.push(annot_idx);
                    }
                    tracing::debug!(page_idx, annot_idx, "extract: post-process");
                }
                if !indices.is_empty() {
                    deletions.push((page_idx as usize, indices));
                }
            }
        }
        tracing::debug!(extracted = out.len(), "extract: read pass done");

        // Delete pass — walk indices in reverse per page so earlier ones
        // don't shift under us.
        for (page_idx, mut indices) in deletions {
            indices.sort_unstable_by(|a, b| b.cmp(a));
            let pages = self.inner_mut().pages_mut();
            let Ok(mut page) = pages.get(page_idx as i32) else {
                continue;
            };
            let annots = &mut page.annotations_mut();
            for idx in indices {
                if let Ok(ann) = annots.get(idx) {
                    let _ = annots.delete_annotation(ann);
                }
            }
        }
        tracing::debug!("extract: delete pass done");

        out
    }
}

fn annotation_kind_label(ann: &PdfPageAnnotation<'_>) -> &'static str {
    match ann {
        PdfPageAnnotation::Square(_) => "Square",
        PdfPageAnnotation::Highlight(_) => "Highlight",
        PdfPageAnnotation::Ink(_) => "Ink",
        PdfPageAnnotation::Stamp(_) => "Stamp",
        PdfPageAnnotation::FreeText(_) => "FreeText",
        PdfPageAnnotation::Circle(_) => "Circle",
        PdfPageAnnotation::Underline(_) => "Underline",
        PdfPageAnnotation::Strikeout(_) => "Strikeout",
        PdfPageAnnotation::Squiggly(_) => "Squiggly",
        PdfPageAnnotation::Link(_) => "Link",
        PdfPageAnnotation::Popup(_) => "Popup",
        PdfPageAnnotation::Text(_) => "Text",
        PdfPageAnnotation::Widget(_) => "Widget",
        PdfPageAnnotation::XfaWidget(_) => "XfaWidget",
        PdfPageAnnotation::Redacted(_) => "Redacted",
        PdfPageAnnotation::Unsupported(_) => "Unsupported",
    }
}

fn annotation_to_core(ann: &PdfPageAnnotation<'_>, page: u32, page_h: f64) -> Option<Annotation> {
    match ann {
        PdfPageAnnotation::Square(s) => {
            tracing::debug!("Square: bounds()");
            let bounds = s.bounds().ok()?;
            let bbox = pdf_rect_to_image(&bounds, page_h);
            tracing::debug!(?bbox, "Square: bounds OK, skipping stroke_color");
            // Default to red — pdfium-render's `stroke_color()` segfaults on
            // some annotations authored by older Previewer sessions. Better
            // to drop the original colour than to crash the open. Future
            // work: read /C via lopdf instead.
            Some(Annotation::Rect {
                page,
                bbox,
                stroke: Stroke::new(Color::RED, 2.0),
                fill: None,
            })
        }
        PdfPageAnnotation::Highlight(h) => {
            tracing::debug!("Highlight: bounds()");
            let bounds = h.bounds().ok()?;
            let bbox = pdf_rect_to_image(&bounds, page_h);
            tracing::debug!(?bbox, "Highlight: bounds OK, default color");
            Some(Annotation::Highlight {
                page,
                bbox,
                color: Color::rgba(255, 235, 0, 96),
            })
        }
        _ => None,
    }
}

/// Walk the PDF on disk via lopdf and pull out every `/FreeText` annotation
/// as a `previewer_core::Annotation::FreeText`.
fn read_freetext_via_lopdf(path: &std::path::Path) -> Vec<Annotation> {
    let Ok(doc) = lopdf::Document::load(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (lopdf_idx, page_id) in doc.get_pages() {
        let page_idx = lopdf_idx.saturating_sub(1);
        let Ok(page_dict) = doc.get_object(page_id).and_then(|o| o.as_dict().cloned()) else {
            continue;
        };
        let page_h = lopdf_page_height(&page_dict);
        let Ok(annots_obj) = page_dict.get(b"Annots").cloned() else {
            continue;
        };
        let arr = match annots_obj {
            lopdf::Object::Array(a) => a,
            lopdf::Object::Reference(r) => {
                match doc.get_object(r).and_then(|o| o.as_array().cloned()) {
                    Ok(a) => a,
                    Err(_) => continue,
                }
            }
            _ => continue,
        };
        for entry in arr {
            let dict = match entry {
                lopdf::Object::Dictionary(d) => d,
                lopdf::Object::Reference(r) => {
                    match doc.get_object(r).and_then(|o| o.as_dict().cloned()) {
                        Ok(d) => d,
                        Err(_) => continue,
                    }
                }
                _ => continue,
            };
            let Ok(subtype) = dict.get(b"Subtype").and_then(|o| o.as_name()) else {
                continue;
            };
            if subtype != b"FreeText" {
                continue;
            }
            let Ok(rect) = dict.get(b"Rect").and_then(|o| o.as_array()) else {
                continue;
            };
            if rect.len() != 4 {
                continue;
            }
            let llx = lopdf_num(&rect[0]);
            let lly = lopdf_num(&rect[1]);
            let urx = lopdf_num(&rect[2]);
            let ury = lopdf_num(&rect[3]);
            // PDF (bottom-left origin) → image (top-left origin)
            let bbox = Rect::new(llx, page_h - ury, urx - llx, ury - lly);
            let text = dict
                .get(b"Contents")
                .ok()
                .and_then(|o| match o {
                    lopdf::Object::String(bytes, _) => {
                        Some(String::from_utf8_lossy(bytes).into_owned())
                    }
                    _ => None,
                })
                .unwrap_or_default();

            // Recover the size + color from /DA ("r g b rg /Helv size Tf"),
            // and the family from the appearance stream's font resource. Both
            // fall back to defaults if anything's missing or malformed.
            let (size_from_da, color_from_da) = dict
                .get(b"DA")
                .ok()
                .and_then(|o| match o {
                    lopdf::Object::String(bytes, _) => Some(parse_da(bytes)),
                    _ => None,
                })
                .unwrap_or((None, None));
            let family =
                read_freetext_family(&doc, &dict).unwrap_or_else(|| "Helvetica".to_string());
            let mut font = FontSpec { family, size: 14.0 };
            if let Some(s) = size_from_da {
                font.size = s;
            }
            let color = color_from_da.unwrap_or(Color::BLACK);

            out.push(Annotation::FreeText {
                page: page_idx,
                position: Point::new(bbox.x, bbox.y),
                text,
                font,
                color,
                // Anything we read back from disk is committed real text.
                is_placeholder: false,
            });
        }
    }
    out
}

/// Walk the PDF and pull every `/Square` annotation as `Annotation::Rect`.
/// Like the ellipse / line readers we get the stroke style out of
/// `/Border`'s optional 4th-element dash array.
fn read_rect_via_lopdf(path: &std::path::Path) -> Vec<Annotation> {
    let Ok(doc) = lopdf::Document::load(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (page_idx, page_h, dict) in iter_annot_dicts(&doc) {
        let Ok(subtype) = dict.get(b"Subtype").and_then(|o| o.as_name()) else {
            continue;
        };
        if subtype != b"Square" {
            continue;
        }
        let Some(bbox) = read_rect_to_image_bbox(&dict, page_h) else {
            continue;
        };
        let stroke_color = read_color_array(&dict, b"C").unwrap_or(Color::RED);
        let fill = read_color_array(&dict, b"IC");
        let stroke_width = read_border_width(&dict).unwrap_or(2.0);
        let style = read_border_style(&dict);
        out.push(Annotation::Rect {
            page: page_idx,
            bbox,
            stroke: Stroke::with_style(stroke_color, stroke_width, style),
            fill,
        });
    }
    out
}

/// Walk the PDF and pull every `/Circle` annotation as
/// `Annotation::Ellipse`. Stroke width comes from `/Border [_ _ w]`
/// (defaulting to 2.0 if absent); stroke color from `/C`; optional fill
/// from `/IC`.
fn read_ellipse_via_lopdf(path: &std::path::Path) -> Vec<Annotation> {
    let Ok(doc) = lopdf::Document::load(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (page_idx, page_h, dict) in iter_annot_dicts(&doc) {
        let Ok(subtype) = dict.get(b"Subtype").and_then(|o| o.as_name()) else {
            continue;
        };
        if subtype != b"Circle" {
            continue;
        }
        let Some(bbox) = read_rect_to_image_bbox(&dict, page_h) else {
            continue;
        };
        let stroke_color = read_color_array(&dict, b"C").unwrap_or(Color::RED);
        let fill = read_color_array(&dict, b"IC");
        let stroke_width = read_border_width(&dict).unwrap_or(2.0);
        let style = read_border_style(&dict);
        out.push(Annotation::Ellipse {
            page: page_idx,
            bbox,
            stroke: Stroke::with_style(stroke_color, stroke_width, style),
            fill,
        });
    }
    out
}

/// Walk the PDF and pull every `/Line` annotation as
/// `Annotation::Arrow`, decoding the head config from `/LE`.
fn read_line_via_lopdf(path: &std::path::Path) -> Vec<Annotation> {
    let Ok(doc) = lopdf::Document::load(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (page_idx, page_h, dict) in iter_annot_dicts(&doc) {
        let Ok(subtype) = dict.get(b"Subtype").and_then(|o| o.as_name()) else {
            continue;
        };
        if subtype != b"Line" {
            continue;
        }
        let Ok(l) = dict.get(b"L").and_then(|o| o.as_array()) else {
            continue;
        };
        if l.len() != 4 {
            continue;
        }
        let x1 = lopdf_num(&l[0]);
        let y1 = lopdf_num(&l[1]);
        let x2 = lopdf_num(&l[2]);
        let y2 = lopdf_num(&l[3]);
        let from = Point::new(x1, page_h - y1);
        let to = Point::new(x2, page_h - y2);
        let color = read_color_array(&dict, b"C").unwrap_or(Color::RED);
        let width = read_border_width(&dict).unwrap_or(2.0);
        let style = read_border_style(&dict);
        let ends = read_line_ends(&dict);
        out.push(Annotation::Arrow {
            page: page_idx,
            from,
            to,
            stroke: Stroke::with_style(color, width, style),
            ends,
        });
    }
    out
}

/// Iterate every annotation dict on every page along with its 0-based
/// page index and the page's height in PDF user units. Resolves
/// indirect references so callers see plain dictionaries.
fn iter_annot_dicts(
    doc: &lopdf::Document,
) -> impl Iterator<Item = (u32, f64, lopdf::Dictionary)> + '_ {
    let mut entries: Vec<(u32, f64, lopdf::Dictionary)> = Vec::new();
    for (lopdf_idx, page_id) in doc.get_pages() {
        let page_idx = lopdf_idx.saturating_sub(1);
        let Ok(page_dict) = doc.get_object(page_id).and_then(|o| o.as_dict().cloned()) else {
            continue;
        };
        let page_h = lopdf_page_height(&page_dict);
        let Ok(annots_obj) = page_dict.get(b"Annots").cloned() else {
            continue;
        };
        let arr = match annots_obj {
            lopdf::Object::Array(a) => a,
            lopdf::Object::Reference(r) => {
                match doc.get_object(r).and_then(|o| o.as_array().cloned()) {
                    Ok(a) => a,
                    Err(_) => continue,
                }
            }
            _ => continue,
        };
        for entry in arr {
            let dict = match entry {
                lopdf::Object::Dictionary(d) => d,
                lopdf::Object::Reference(r) => {
                    match doc.get_object(r).and_then(|o| o.as_dict().cloned()) {
                        Ok(d) => d,
                        Err(_) => continue,
                    }
                }
                _ => continue,
            };
            entries.push((page_idx, page_h, dict));
        }
    }
    entries.into_iter()
}

fn read_rect_to_image_bbox(dict: &lopdf::Dictionary, page_h: f64) -> Option<Rect> {
    let rect = dict.get(b"Rect").ok()?.as_array().ok()?;
    if rect.len() != 4 {
        return None;
    }
    let llx = lopdf_num(&rect[0]);
    let lly = lopdf_num(&rect[1]);
    let urx = lopdf_num(&rect[2]);
    let ury = lopdf_num(&rect[3]);
    Some(Rect::new(llx, page_h - ury, urx - llx, ury - lly))
}

fn read_color_array(dict: &lopdf::Dictionary, key: &[u8]) -> Option<Color> {
    let arr = dict.get(key).ok()?.as_array().ok()?;
    if arr.len() < 3 {
        return None;
    }
    let r = lopdf_num(&arr[0]);
    let g = lopdf_num(&arr[1]);
    let b = lopdf_num(&arr[2]);
    Some(Color::rgba(
        (r * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * 255.0).round().clamp(0.0, 255.0) as u8,
        255,
    ))
}

/// Inspect `/Border` for an optional 4th-element dash array and decide
/// whether the stroke is solid, dashed, or dotted. The heuristic mirrors
/// what we *write*: tiny on/off pairs read as Dotted, larger ones as
/// Dashed; everything else (no dash, single zero) is Solid.
fn read_border_style(dict: &lopdf::Dictionary) -> StrokeStyle {
    let Ok(arr) = dict.get(b"Border").and_then(|o| o.as_array()) else {
        return StrokeStyle::Solid;
    };
    if arr.len() < 4 {
        return StrokeStyle::Solid;
    }
    let Ok(dash) = arr[3].as_array() else {
        return StrokeStyle::Solid;
    };
    if dash.is_empty() {
        return StrokeStyle::Solid;
    }
    let on = lopdf_num(&dash[0]);
    if on <= 0.0 {
        return StrokeStyle::Solid;
    }
    let width = read_border_width(dict).unwrap_or(1.0);
    // Same threshold we use on the write side: dotted runs use on ≤ ~1×W,
    // dashed runs use on ≥ ~3×W. Anything in between we lean toward
    // Dashed since that's the more common explicit style.
    if on <= width.max(0.5) * 1.5 {
        StrokeStyle::Dotted
    } else {
        StrokeStyle::Dashed
    }
}

fn read_border_width(dict: &lopdf::Dictionary) -> Option<f64> {
    let arr = dict.get(b"Border").ok()?.as_array().ok()?;
    if arr.len() < 3 {
        return None;
    }
    Some(lopdf_num(&arr[2]))
}

fn read_line_ends(dict: &lopdf::Dictionary) -> ArrowEnds {
    let Ok(le) = dict.get(b"LE").and_then(|o| o.as_array()) else {
        return ArrowEnds::End;
    };
    if le.len() != 2 {
        return ArrowEnds::End;
    }
    let start_is_head = matches!(le[0].as_name(), Ok(n) if n != b"None");
    let end_is_head = matches!(le[1].as_name(), Ok(n) if n != b"None");
    match (start_is_head, end_is_head) {
        (true, true) => ArrowEnds::Both,
        (false, true) => ArrowEnds::End,
        (true, false) => ArrowEnds::End, // odd; treat as end-only
        (false, false) => ArrowEnds::None,
    }
}

fn lopdf_page_height(page: &lopdf::Dictionary) -> f64 {
    let Ok(media_box) = page.get(b"MediaBox").and_then(|o| o.as_array()) else {
        return 792.0; // Letter height fallback
    };
    if media_box.len() < 4 {
        return 792.0;
    }
    lopdf_num(&media_box[3]) - lopdf_num(&media_box[1])
}

fn lopdf_num(o: &lopdf::Object) -> f64 {
    match o {
        lopdf::Object::Integer(i) => *i as f64,
        lopdf::Object::Real(r) => *r as f64,
        _ => 0.0,
    }
}

/// Parse a `/DA` string for the most recent `r g b rg` (text colour) and
/// `/Font size Tf` (font size). Returns `(size, color)`; either may be
/// `None` if the operator wasn't found.
fn parse_da(bytes: &[u8]) -> (Option<f64>, Option<Color>) {
    let s = String::from_utf8_lossy(bytes);
    let toks: Vec<&str> = s.split_whitespace().collect();
    let mut size = None;
    let mut color = None;
    for i in 0..toks.len() {
        if toks[i] == "Tf"
            && i >= 2
            && let Ok(sz) = toks[i - 1].parse::<f64>()
        {
            size = Some(sz);
        }
        if toks[i] == "rg"
            && i >= 3
            && let (Ok(r), Ok(g), Ok(b)) = (
                toks[i - 3].parse::<f64>(),
                toks[i - 2].parse::<f64>(),
                toks[i - 1].parse::<f64>(),
            )
        {
            color = Some(Color::rgba(
                (r * 255.0).round().clamp(0.0, 255.0) as u8,
                (g * 255.0).round().clamp(0.0, 255.0) as u8,
                (b * 255.0).round().clamp(0.0, 255.0) as u8,
                255,
            ));
        }
    }
    (size, color)
}

/// Walk `/AP /N → Resources → Font → <first entry> → BaseFont` and map the
/// resulting Type 1 standard name back to the UI family label
/// (`Helvetica` / `Times` / `Courier`). Returns `None` if any step is
/// missing.
fn read_freetext_family(doc: &lopdf::Document, ann_dict: &lopdf::Dictionary) -> Option<String> {
    let ap_dict = resolve_dict(doc, ann_dict.get(b"AP").ok()?)?;
    let n_obj = ap_dict.get(b"N").ok()?;
    let stream_id = match n_obj {
        lopdf::Object::Reference(r) => *r,
        _ => return None,
    };
    let stream = match doc.get_object(stream_id).ok()? {
        lopdf::Object::Stream(s) => s,
        _ => return None,
    };
    let resources = resolve_dict(doc, stream.dict.get(b"Resources").ok()?)?;
    let font_dict = resolve_dict(doc, resources.get(b"Font").ok()?)?;
    let (_, first_font) = font_dict.iter().next()?;
    let f_dict = match first_font {
        lopdf::Object::Reference(r) => doc.get_object(*r).ok()?.as_dict().ok()?.clone(),
        lopdf::Object::Dictionary(d) => d.clone(),
        _ => return None,
    };
    let base_name = f_dict.get(b"BaseFont").ok()?.as_name().ok()?;
    let ui = match std::str::from_utf8(base_name).unwrap_or("") {
        "Times-Roman" | "Times-Bold" | "Times-Italic" | "Times-BoldItalic" => "Times",
        "Courier" | "Courier-Bold" | "Courier-Oblique" | "Courier-BoldOblique" => "Courier",
        _ => "Helvetica",
    };
    Some(ui.to_string())
}

/// Resolve a possibly-indirect dictionary reference into an owned `Dictionary`.
fn resolve_dict(doc: &lopdf::Document, obj: &lopdf::Object) -> Option<lopdf::Dictionary> {
    match obj {
        lopdf::Object::Dictionary(d) => Some(d.clone()),
        lopdf::Object::Reference(r) => doc.get_object(*r).ok()?.as_dict().ok().cloned(),
        _ => None,
    }
}

fn pdf_rect_to_image(rect: &PdfRect, page_h: f64) -> Rect {
    let left = rect.left().value as f64;
    let right = rect.right().value as f64;
    let top = rect.top().value as f64;
    let bottom = rect.bottom().value as f64;
    Rect::new(left, page_h - top, right - left, top - bottom)
}

// pdf_color_to_core is intentionally not used yet — `stroke_color()` segfaults
// on some annotations authored by older sessions. Re-introduce when we read
// `/C` via lopdf instead.
