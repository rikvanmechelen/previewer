//! Post-pdfium injection of annotations via lopdf.
//!
//! pdfium-render 0.9's high-level API doesn't wrap `FPDFAnnot_AddInkStroke`,
//! and its `/FreeText` writer omits the `/AP` appearance stream that viewers
//! (Okular included) need to render the text. Both go through this lopdf
//! pass instead — we append fresh `/Annot` dictionaries with the right
//! keys (and, for `/FreeText`, a Form-XObject appearance stream that
//! actually paints the glyphs) and reference them from the page's
//! `/Annots` array.

use std::path::Path;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use previewer_core::{ArrowEnds, StrokeStyle};

use crate::document::{
    Error, PendingEllipse, PendingFreeText, PendingInk, PendingLine, PendingRect,
};

pub(crate) fn inject(
    path: &Path,
    inks: &[PendingInk],
    freetexts: &[PendingFreeText],
    rects: &[PendingRect],
    ellipses: &[PendingEllipse],
    lines: &[PendingLine],
) -> Result<(), Error> {
    if inks.is_empty()
        && freetexts.is_empty()
        && rects.is_empty()
        && ellipses.is_empty()
        && lines.is_empty()
    {
        return Ok(());
    }
    let mut doc = Document::load(path)?;
    let pages = doc.get_pages();

    for ink in inks {
        let lopdf_idx = ink.page + 1;
        let Some(page_id) = pages.get(&lopdf_idx).copied() else {
            tracing::warn!(page = ink.page, "ink target page missing, skipping");
            continue;
        };
        let page_h = page_height_pts(&doc, page_id)?;
        let annot_id = build_ink_annotation(&mut doc, ink, page_h);
        attach_annot_to_page(&mut doc, page_id, annot_id)?;
    }

    for ft in freetexts {
        let lopdf_idx = ft.page + 1;
        let Some(page_id) = pages.get(&lopdf_idx).copied() else {
            tracing::warn!(page = ft.page, "freetext target page missing, skipping");
            continue;
        };
        let page_h = page_height_pts(&doc, page_id)?;
        let annot_id = build_freetext_annotation(&mut doc, ft, page_h);
        attach_annot_to_page(&mut doc, page_id, annot_id)?;
    }

    for r in rects {
        let lopdf_idx = r.page + 1;
        let Some(page_id) = pages.get(&lopdf_idx).copied() else {
            tracing::warn!(page = r.page, "rect target page missing, skipping");
            continue;
        };
        let page_h = page_height_pts(&doc, page_id)?;
        let annot_id = build_rect_annotation(&mut doc, r, page_h);
        attach_annot_to_page(&mut doc, page_id, annot_id)?;
    }

    for el in ellipses {
        let lopdf_idx = el.page + 1;
        let Some(page_id) = pages.get(&lopdf_idx).copied() else {
            tracing::warn!(page = el.page, "ellipse target page missing, skipping");
            continue;
        };
        let page_h = page_height_pts(&doc, page_id)?;
        let annot_id = build_ellipse_annotation(&mut doc, el, page_h);
        attach_annot_to_page(&mut doc, page_id, annot_id)?;
    }

    for ln in lines {
        let lopdf_idx = ln.page + 1;
        let Some(page_id) = pages.get(&lopdf_idx).copied() else {
            tracing::warn!(page = ln.page, "line target page missing, skipping");
            continue;
        };
        let page_h = page_height_pts(&doc, page_id)?;
        let annot_id = build_line_annotation(&mut doc, ln, page_h);
        attach_annot_to_page(&mut doc, page_id, annot_id)?;
    }

    doc.save(path)?;
    Ok(())
}

/// Build a `[0 0 W]` border array, or `[0 0 W [on off]]` for dashed /
/// dotted styles. Width is rounded to f32 since most PDF readers expect
/// real numbers there.
fn border_array_for(width: f64, style: StrokeStyle) -> Object {
    let w = Object::Real(width as f32);
    let h_corner = Object::Integer(0);
    let v_corner = Object::Integer(0);
    let dash = match style {
        StrokeStyle::Solid => None,
        StrokeStyle::Dashed => Some(((width * 4.0).max(3.0), (width * 3.0).max(2.5))),
        // PDF "dotted" via `/Border` is a small dash + small gap; viewers
        // render it close to the on-screen dotted look (Cairo round-cap
        // zero-dash trick isn't expressible in `/Border` alone).
        StrokeStyle::Dotted => Some(((width * 0.8).max(0.5), (width * 1.8).max(1.5))),
    };
    if let Some((on, off)) = dash {
        Object::Array(vec![
            h_corner,
            v_corner,
            w,
            Object::Array(vec![Object::Real(on as f32), Object::Real(off as f32)]),
        ])
    } else {
        Object::Array(vec![h_corner, v_corner, w])
    }
}

fn build_rect_annotation(doc: &mut Document, r: &PendingRect, page_h: f64) -> ObjectId {
    let llx = r.bbox.x as f32;
    let urx = (r.bbox.x + r.bbox.width) as f32;
    let ury = (page_h - r.bbox.y) as f32;
    let lly = (page_h - r.bbox.y - r.bbox.height) as f32;
    let (sr, sg, sb, _) = r.stroke_color.to_unit_rgba();

    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"Annot".to_vec()));
    dict.set("Subtype", Object::Name(b"Square".to_vec()));
    dict.set(
        "Rect",
        Object::Array(vec![
            Object::Real(llx),
            Object::Real(lly),
            Object::Real(urx),
            Object::Real(ury),
        ]),
    );
    dict.set(
        "C",
        Object::Array(vec![
            Object::Real(sr as f32),
            Object::Real(sg as f32),
            Object::Real(sb as f32),
        ]),
    );
    if let Some(fill) = r.fill {
        let (fr, fg, fb, _) = fill.to_unit_rgba();
        dict.set(
            "IC",
            Object::Array(vec![
                Object::Real(fr as f32),
                Object::Real(fg as f32),
                Object::Real(fb as f32),
            ]),
        );
    }
    dict.set("Border", border_array_for(r.stroke_width, r.stroke_style));
    dict.set("F", Object::Integer(4));
    doc.add_object(Object::Dictionary(dict))
}

fn build_ellipse_annotation(doc: &mut Document, el: &PendingEllipse, page_h: f64) -> ObjectId {
    let llx = el.bbox.x as f32;
    let urx = (el.bbox.x + el.bbox.width) as f32;
    let ury = (page_h - el.bbox.y) as f32;
    let lly = (page_h - el.bbox.y - el.bbox.height) as f32;
    let (sr, sg, sb, _) = el.stroke_color.to_unit_rgba();

    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"Annot".to_vec()));
    dict.set("Subtype", Object::Name(b"Circle".to_vec()));
    dict.set(
        "Rect",
        Object::Array(vec![
            Object::Real(llx),
            Object::Real(lly),
            Object::Real(urx),
            Object::Real(ury),
        ]),
    );
    dict.set(
        "C",
        Object::Array(vec![
            Object::Real(sr as f32),
            Object::Real(sg as f32),
            Object::Real(sb as f32),
        ]),
    );
    if let Some(fill) = el.fill {
        let (fr, fg, fb, _) = fill.to_unit_rgba();
        dict.set(
            "IC",
            Object::Array(vec![
                Object::Real(fr as f32),
                Object::Real(fg as f32),
                Object::Real(fb as f32),
            ]),
        );
    }
    dict.set("Border", border_array_for(el.stroke_width, el.stroke_style));
    dict.set("F", Object::Integer(4));
    doc.add_object(Object::Dictionary(dict))
}

fn build_line_annotation(doc: &mut Document, ln: &PendingLine, page_h: f64) -> ObjectId {
    // Image-coord endpoints → PDF user space (Y flip).
    let x1 = ln.from.x as f32;
    let y1 = (page_h - ln.from.y) as f32;
    let x2 = ln.to.x as f32;
    let y2 = (page_h - ln.to.y) as f32;
    // /Rect must enclose the line; normalise to (llx,lly,urx,ury).
    let (rllx, rurx) = if x1 <= x2 { (x1, x2) } else { (x2, x1) };
    let (rlly, rury) = if y1 <= y2 { (y1, y2) } else { (y2, y1) };
    let (r, g, b, _) = ln.color.to_unit_rgba();

    // /LE pairs map to (start ending, end ending). PDF's "OpenArrow" is
    // the conventional arrowhead; "None" is a plain endpoint.
    let (le_start, le_end): (&[u8], &[u8]) = match ln.ends {
        ArrowEnds::None => (b"None", b"None"),
        ArrowEnds::End => (b"None", b"OpenArrow"),
        ArrowEnds::Both => (b"OpenArrow", b"OpenArrow"),
    };

    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"Annot".to_vec()));
    dict.set("Subtype", Object::Name(b"Line".to_vec()));
    dict.set(
        "Rect",
        Object::Array(vec![
            Object::Real(rllx),
            Object::Real(rlly),
            Object::Real(rurx),
            Object::Real(rury),
        ]),
    );
    dict.set(
        "L",
        Object::Array(vec![
            Object::Real(x1),
            Object::Real(y1),
            Object::Real(x2),
            Object::Real(y2),
        ]),
    );
    dict.set(
        "LE",
        Object::Array(vec![
            Object::Name(le_start.to_vec()),
            Object::Name(le_end.to_vec()),
        ]),
    );
    dict.set(
        "C",
        Object::Array(vec![
            Object::Real(r as f32),
            Object::Real(g as f32),
            Object::Real(b as f32),
        ]),
    );
    dict.set("Border", border_array_for(ln.width, ln.style));
    dict.set("F", Object::Integer(4));
    doc.add_object(Object::Dictionary(dict))
}

fn build_ink_annotation(doc: &mut Document, ink: &PendingInk, page_h: f64) -> ObjectId {
    let ink_list: Vec<Object> = ink
        .strokes
        .iter()
        .filter(|s| s.len() >= 2)
        .map(|stroke| {
            let mut nums = Vec::with_capacity(stroke.len() * 2);
            for p in stroke {
                nums.push(Object::Real(p.x as f32));
                nums.push(Object::Real((page_h - p.y) as f32));
            }
            Object::Array(nums)
        })
        .collect();

    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for s in &ink.strokes {
        for p in s {
            if !p.x.is_finite() || !p.y.is_finite() {
                continue;
            }
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
        }
    }
    let rect = if min_x.is_finite() {
        vec![
            Object::Real(min_x as f32),
            Object::Real((page_h - max_y) as f32),
            Object::Real(max_x as f32),
            Object::Real((page_h - min_y) as f32),
        ]
    } else {
        vec![
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(0.0),
        ]
    };

    let (r, g, b, _) = ink.color.to_unit_rgba();

    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"Annot".to_vec()));
    dict.set("Subtype", Object::Name(b"Ink".to_vec()));
    dict.set("Rect", Object::Array(rect));
    dict.set("InkList", Object::Array(ink_list));
    dict.set(
        "C",
        Object::Array(vec![
            Object::Real(r as f32),
            Object::Real(g as f32),
            Object::Real(b as f32),
        ]),
    );
    dict.set(
        "Border",
        Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Real(ink.width as f32),
        ]),
    );
    dict.set("F", Object::Integer(4));

    doc.add_object(Object::Dictionary(dict))
}

fn build_freetext_annotation(doc: &mut Document, ft: &PendingFreeText, page_h: f64) -> ObjectId {
    // Image-coord bbox → PDF-coord rect (Y-flip).
    let llx = ft.bbox.x as f32;
    let urx = (ft.bbox.x + ft.bbox.width) as f32;
    let ury = (page_h - ft.bbox.y) as f32;
    let lly = (page_h - ft.bbox.y - ft.bbox.height) as f32;
    let bbox_w = (ft.bbox.width as f32).max(1.0);
    let bbox_h = (ft.bbox.height as f32).max(1.0);

    let (r, g, b, _) = ft.color.to_unit_rgba();
    let size = ft.font.size;
    let line_height = size * 1.4;

    // Map the chosen family to one of the 14 PDF Type 1 standard fonts so we
    // don't have to embed font data. Anything we don't recognise falls back
    // to Helvetica.
    let base_font: &[u8] = match ft.font.family.as_str() {
        "Times" | "Times-Roman" | "Times Roman" => b"Times-Roman",
        "Courier" => b"Courier",
        _ => b"Helvetica",
    };
    let mut font_dict = Dictionary::new();
    font_dict.set("Type", Object::Name(b"Font".to_vec()));
    font_dict.set("Subtype", Object::Name(b"Type1".to_vec()));
    font_dict.set("BaseFont", Object::Name(base_font.to_vec()));
    font_dict.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
    let font_id = doc.add_object(Object::Dictionary(font_dict));

    let mut font_res = Dictionary::new();
    font_res.set("Helv", Object::Reference(font_id));
    let mut resources = Dictionary::new();
    resources.set("Font", Object::Dictionary(font_res));

    // Content stream: position the cursor at the top-left of the bbox
    // (one ascent down from the top), then emit one Tj per line with T*
    // line-feeds between them.
    let first_baseline_y = bbox_h as f64 - size;
    let mut content = String::new();
    content.push_str("q\n");
    content.push_str("BT\n");
    content.push_str(&format!("{r:.3} {g:.3} {b:.3} rg\n"));
    content.push_str(&format!("/Helv {size:.2} Tf\n"));
    content.push_str(&format!("{line_height:.2} TL\n"));
    content.push_str(&format!("1 0 0 1 2 {first_baseline_y:.2} Tm\n"));
    for (i, line) in ft.text.split('\n').enumerate() {
        if i > 0 {
            content.push_str("T*\n");
        }
        content.push('(');
        for c in line.chars() {
            match c {
                '\\' => content.push_str("\\\\"),
                '(' => content.push_str("\\("),
                ')' => content.push_str("\\)"),
                '\r' => {}
                _ => content.push(c),
            }
        }
        content.push_str(") Tj\n");
    }
    content.push_str("ET\n");
    content.push_str("Q\n");

    let mut form_dict = Dictionary::new();
    form_dict.set("Type", Object::Name(b"XObject".to_vec()));
    form_dict.set("Subtype", Object::Name(b"Form".to_vec()));
    form_dict.set("FormType", Object::Integer(1));
    form_dict.set(
        "BBox",
        Object::Array(vec![
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(bbox_w),
            Object::Real(bbox_h),
        ]),
    );
    form_dict.set("Resources", Object::Dictionary(resources));
    let stream = Stream::new(form_dict, content.into_bytes());
    let ap_n_id = doc.add_object(Object::Stream(stream));

    let mut ap_dict = Dictionary::new();
    ap_dict.set("N", Object::Reference(ap_n_id));

    let da = format!("{r:.3} {g:.3} {b:.3} rg /Helv {size:.2} Tf");

    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"Annot".to_vec()));
    dict.set("Subtype", Object::Name(b"FreeText".to_vec()));
    dict.set(
        "Rect",
        Object::Array(vec![
            Object::Real(llx),
            Object::Real(lly),
            Object::Real(urx),
            Object::Real(ury),
        ]),
    );
    dict.set(
        "Contents",
        Object::String(ft.text.as_bytes().to_vec(), lopdf::StringFormat::Literal),
    );
    dict.set(
        "DA",
        Object::String(da.into_bytes(), lopdf::StringFormat::Literal),
    );
    dict.set("AP", Object::Dictionary(ap_dict));
    dict.set("F", Object::Integer(4));

    doc.add_object(Object::Dictionary(dict))
}

fn attach_annot_to_page(
    doc: &mut Document,
    page_id: ObjectId,
    annot_id: ObjectId,
) -> Result<(), Error> {
    let existing = doc
        .get_object(page_id)?
        .as_dict()?
        .get(b"Annots")
        .ok()
        .cloned();

    match existing {
        Some(Object::Array(mut arr)) => {
            arr.push(Object::Reference(annot_id));
            doc.get_object_mut(page_id)?
                .as_dict_mut()?
                .set("Annots", Object::Array(arr));
        }
        Some(Object::Reference(arr_id)) => {
            let arr = doc.get_object_mut(arr_id)?.as_array_mut()?;
            arr.push(Object::Reference(annot_id));
        }
        _ => {
            doc.get_object_mut(page_id)?
                .as_dict_mut()?
                .set("Annots", Object::Array(vec![Object::Reference(annot_id)]));
        }
    }
    Ok(())
}

fn page_height_pts(doc: &Document, page_id: ObjectId) -> Result<f64, Error> {
    let page = doc.get_object(page_id)?.as_dict()?;
    let media_box = page
        .get(b"MediaBox")
        .map_err(|_| lopdf::Error::DictKey("MediaBox".to_string()))?
        .as_array()?;
    if media_box.len() < 4 {
        return Err(lopdf::Error::DictKey("MediaBox not 4 numbers".to_string()).into());
    }
    let lly = number_to_f64(&media_box[1]);
    let ury = number_to_f64(&media_box[3]);
    Ok(ury - lly)
}

fn number_to_f64(o: &Object) -> f64 {
    match o {
        Object::Integer(i) => *i as f64,
        Object::Real(r) => *r as f64,
        _ => 0.0,
    }
}
