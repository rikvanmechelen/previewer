//! Round-trip tests for annotation writing.
//!
//! Strategy: open a blank fixture PDF, add an annotation via
//! `Document::add_annotation`, save to a tempfile, then re-parse the saved
//! file with `lopdf` and assert on the `/Annots` array — we check semantic
//! invariants (presence of the right `/Subtype`, `/Rect` near the expected
//! coords) rather than diff bytes.

use lopdf::{Document as LoDoc, Object};
use pretty_assertions::assert_eq;
use previewer_core::{Annotation, ArrowEnds, Color, Point, Rect, StampImage, Stroke};
use previewer_pdf::Document;
use tempfile::TempDir;

mod fixtures;

/// Walk the lopdf doc and return the dictionary of page index `idx`.
fn page_dict(doc: &LoDoc, idx: u32) -> lopdf::Dictionary {
    let pages = doc.get_pages();
    let page_id = *pages.get(&(idx + 1)).expect("page exists"); // lopdf is 1-indexed
    doc.get_object(page_id)
        .expect("get page object")
        .as_dict()
        .expect("page is a dict")
        .clone()
}

fn annots_on_page(doc: &LoDoc, idx: u32) -> Vec<lopdf::Dictionary> {
    let page = page_dict(doc, idx);
    let Ok(annots_obj) = page.get(b"Annots") else {
        return Vec::new();
    };
    let arr = match annots_obj {
        Object::Array(a) => a.clone(),
        Object::Reference(r) => doc.get_object(*r).unwrap().as_array().unwrap().to_vec(),
        _ => panic!("/Annots is neither an array nor a reference"),
    };
    arr.into_iter()
        .map(|o| match o {
            Object::Reference(r) => doc.get_object(r).unwrap().as_dict().unwrap().clone(),
            Object::Dictionary(d) => d,
            other => panic!("unexpected /Annots entry: {other:?}"),
        })
        .collect()
}

#[test]
#[serial_test::serial]
fn freetext_round_trip_through_extract() {
    let dir = TempDir::new().unwrap();
    let in_path = dir.path().join("blank.pdf");
    fixtures::write_blank_pdf(&in_path, 1);

    let mut doc = Document::open(&in_path).expect("open");
    doc.add_annotation(&Annotation::FreeText {
        page: 0,
        position: Point::new(50.0, 100.0),
        text: "hello\nworld".into(),
        font: previewer_core::FontSpec::default(),
        color: Color::BLACK,
        is_placeholder: false,
    })
    .unwrap();
    let out_path = dir.path().join("annotated.pdf");
    doc.save(&out_path).unwrap();
    drop(doc);

    let mut reopened = Document::open(&out_path).expect("reopen");
    let extracted = reopened.extract_annotations();
    let texts: Vec<&str> = extracted
        .iter()
        .filter_map(|a| match a {
            Annotation::FreeText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 1, "expected 1 FreeText, got {extracted:?}");
    assert_eq!(texts[0], "hello\nworld");
}

#[test]
#[serial_test::serial]
fn freetext_preserves_font_family_size_color_through_save() {
    // Regression: writing FreeText with non-default font/size/color and
    // re-opening must yield the same style — otherwise editing styled text,
    // saving, and re-editing reverts everything to defaults.
    let dir = TempDir::new().unwrap();
    let in_path = dir.path().join("blank.pdf");
    fixtures::write_blank_pdf(&in_path, 1);

    let mut doc = Document::open(&in_path).expect("open");
    doc.add_annotation(&Annotation::FreeText {
        page: 0,
        position: Point::new(40.0, 80.0),
        text: "styled".into(),
        font: previewer_core::FontSpec {
            family: "Times".into(),
            size: 24.0,
        },
        color: Color::rgba(255, 0, 0, 255),
        is_placeholder: false,
    })
    .unwrap();
    let out_path = dir.path().join("styled.pdf");
    doc.save(&out_path).unwrap();
    drop(doc);

    let mut reopened = Document::open(&out_path).expect("reopen");
    let extracted = reopened.extract_annotations();
    let ft = extracted
        .iter()
        .find_map(|a| match a {
            Annotation::FreeText { font, color, .. } => Some((font.clone(), *color)),
            _ => None,
        })
        .expect("FreeText not extracted");
    assert_eq!(ft.0.family, "Times");
    assert!((ft.0.size - 24.0).abs() < 0.01, "size = {}", ft.0.size);
    assert_eq!(ft.1, Color::rgba(255, 0, 0, 255));
}

#[test]
#[serial_test::serial]
fn extract_picks_up_existing_square_and_highlight() {
    let dir = TempDir::new().unwrap();
    let in_path = dir.path().join("blank.pdf");
    fixtures::write_blank_pdf(&in_path, 1);

    // Write a Rect + Highlight, save, reopen, then extract.
    let mut doc = Document::open(&in_path).expect("open");
    doc.add_annotation(&Annotation::Rect {
        page: 0,
        bbox: Rect::new(50.0, 60.0, 100.0, 40.0),
        stroke: Stroke::new(Color::RED, 2.0),
        fill: None,
    })
    .unwrap();
    doc.add_annotation(&Annotation::Highlight {
        page: 0,
        bbox: Rect::new(120.0, 160.0, 200.0, 24.0),
        color: Color::rgba(255, 235, 0, 96),
    })
    .unwrap();
    let out_path = dir.path().join("annotated.pdf");
    doc.save(&out_path).unwrap();
    drop(doc);

    let mut reopened = Document::open(&out_path).expect("reopen");
    let extracted = reopened.extract_annotations();

    assert_eq!(extracted.len(), 2, "expected 2 extracted annotations");
    let kinds: Vec<&str> = extracted
        .iter()
        .map(|a| match a {
            Annotation::Rect { .. } => "Rect",
            Annotation::Highlight { .. } => "Highlight",
            _ => "Other",
        })
        .collect();
    assert!(kinds.contains(&"Rect"));
    assert!(kinds.contains(&"Highlight"));

    // After extraction, a fresh save must NOT bring those originals back —
    // they were removed from the pdfium document by extract_annotations.
    let resaved = dir.path().join("resaved.pdf");
    reopened.save(&resaved).unwrap();
    drop(reopened);

    let saved = LoDoc::load(&resaved).unwrap();
    assert_eq!(
        annots_on_page(&saved, 0).len(),
        0,
        "expected re-save to have zero annotations after extract clears them"
    );
}

#[test]
#[serial_test::serial]
fn rect_annotation_survives_round_trip() {
    let dir = TempDir::new().unwrap();
    let in_path = dir.path().join("blank.pdf");
    fixtures::write_blank_pdf(&in_path, 1);

    let mut doc = Document::open(&in_path).expect("open");
    doc.add_annotation(&Annotation::Rect {
        page: 0,
        bbox: Rect::new(100.0, 100.0, 200.0, 150.0),
        stroke: Stroke::new(Color::RED, 2.0),
        fill: None,
    })
    .expect("add_annotation");
    let out_path = dir.path().join("annotated.pdf");
    doc.save(&out_path).expect("save");
    drop(doc);

    let saved = LoDoc::load(&out_path).expect("lopdf load");
    let annots = annots_on_page(&saved, 0);
    assert_eq!(annots.len(), 1, "expected one /Annot on page 0");

    let subtype = annots[0]
        .get(b"Subtype")
        .expect("Subtype")
        .as_name()
        .expect("Subtype name");
    assert_eq!(subtype, b"Square", "expected /Subtype /Square");

    // /Rect = [llx, lly, urx, ury] in PDF user space (origin bottom-left).
    let rect = annots[0]
        .get(b"Rect")
        .expect("Rect")
        .as_array()
        .expect("Rect array");
    assert_eq!(rect.len(), 4);
}

#[test]
#[serial_test::serial]
fn highlight_actually_renders_yellow() {
    // The semantic round-trip test (next) only checks /Subtype = Highlight;
    // that's necessary but not sufficient. This test verifies that the
    // saved PDF, when re-rendered by pdfium, actually paints yellow pixels
    // inside the highlight area.
    let dir = TempDir::new().unwrap();
    let in_path = dir.path().join("blank.pdf");
    fixtures::write_blank_pdf(&in_path, 1);

    let mut doc = Document::open(&in_path).expect("open");
    doc.add_annotation(&Annotation::Highlight {
        page: 0,
        bbox: Rect::new(100.0, 200.0, 200.0, 50.0),
        color: Color::rgba(255, 235, 0, 96),
    })
    .expect("add highlight");
    let out_path = dir.path().join("annotated.pdf");
    doc.save(&out_path).expect("save");
    drop(doc);

    let saved = Document::open(&out_path).expect("reopen");
    let rendered = saved.render_page(0, 1.0).expect("render");

    let (w, _) = rendered.dimensions();
    let (x, y) = (200_usize, 215_usize);
    let i = (y * w as usize + x) * 4;
    let r = rendered.pixels()[i];
    let g = rendered.pixels()[i + 1];
    let b = rendered.pixels()[i + 2];

    assert!(r > 200, "expected red ≥200 (yellow tint), got r={r}");
    assert!(g > 200, "expected green ≥200 (yellow tint), got g={g}");
    assert!(b < 220, "expected blue suppressed, got b={b}");
}

#[test]
#[serial_test::serial]
fn highlight_annotation_survives_round_trip() {
    let dir = TempDir::new().unwrap();
    let in_path = dir.path().join("blank.pdf");
    fixtures::write_blank_pdf(&in_path, 1);

    let mut doc = Document::open(&in_path).expect("open");
    doc.add_annotation(&Annotation::Highlight {
        page: 0,
        bbox: Rect::new(50.0, 200.0, 300.0, 24.0),
        color: Color::rgba(255, 235, 0, 96),
    })
    .expect("add_annotation");
    let out_path = dir.path().join("annotated.pdf");
    doc.save(&out_path).expect("save");
    drop(doc);

    let saved = LoDoc::load(&out_path).expect("lopdf load");
    let annots = annots_on_page(&saved, 0);
    assert_eq!(annots.len(), 1);
    let subtype = annots[0].get(b"Subtype").unwrap().as_name().unwrap();
    assert_eq!(subtype, b"Highlight");
}

#[test]
#[serial_test::serial]
fn multiple_annotations_on_one_page() {
    let dir = TempDir::new().unwrap();
    let in_path = dir.path().join("blank.pdf");
    fixtures::write_blank_pdf(&in_path, 1);

    let mut doc = Document::open(&in_path).expect("open");
    for i in 0..3 {
        doc.add_annotation(&Annotation::Rect {
            page: 0,
            bbox: Rect::new(50.0 + (i as f64) * 60.0, 100.0, 50.0, 50.0),
            stroke: Stroke::new(Color::BLACK, 1.0),
            fill: None,
        })
        .unwrap();
    }
    let out_path = dir.path().join("annotated.pdf");
    doc.save(&out_path).expect("save");
    drop(doc);

    let saved = LoDoc::load(&out_path).expect("lopdf load");
    let annots = annots_on_page(&saved, 0);
    assert_eq!(annots.len(), 3);
}

#[test]
#[serial_test::serial]
fn stamp_annotation_survives_round_trip() {
    let dir = TempDir::new().unwrap();
    let in_path = dir.path().join("blank.pdf");
    fixtures::write_blank_pdf(&in_path, 1);

    // Tiny opaque red 4×4 stamp.
    let pixels: Vec<u8> = (0..4 * 4).flat_map(|_| [255_u8, 0, 0, 255]).collect();

    let mut doc = Document::open(&in_path).expect("open");
    doc.add_annotation(&Annotation::Stamp {
        page: 0,
        bbox: Rect::new(100.0, 100.0, 80.0, 32.0),
        image: StampImage {
            width: 4,
            height: 4,
            pixels,
        },
    })
    .expect("add_annotation");
    let out_path = dir.path().join("annotated.pdf");
    doc.save(&out_path).expect("save");
    drop(doc);

    let saved = LoDoc::load(&out_path).expect("lopdf load");
    let annots = annots_on_page(&saved, 0);
    assert_eq!(annots.len(), 1, "expected one /Annot on page 0");
    let subtype = annots[0].get(b"Subtype").unwrap().as_name().unwrap();
    assert_eq!(subtype, b"Stamp");
}

#[test]
#[serial_test::serial]
fn ink_annotation_survives_round_trip() {
    let dir = TempDir::new().unwrap();
    let in_path = dir.path().join("blank.pdf");
    fixtures::write_blank_pdf(&in_path, 1);

    let mut doc = Document::open(&in_path).expect("open");
    doc.add_annotation(&Annotation::Ink {
        page: 0,
        strokes: vec![
            vec![
                Point::new(50.0, 100.0),
                Point::new(80.0, 105.0),
                Point::new(110.0, 95.0),
            ],
            vec![Point::new(120.0, 110.0), Point::new(150.0, 100.0)],
        ],
        color: Color::BLACK,
        width: 1.5,
    })
    .expect("add_annotation");
    let out_path = dir.path().join("annotated.pdf");
    doc.save(&out_path).expect("save");
    drop(doc);

    let saved = LoDoc::load(&out_path).expect("lopdf load");
    let annots = annots_on_page(&saved, 0);
    assert_eq!(annots.len(), 1);
    let subtype = annots[0].get(b"Subtype").unwrap().as_name().unwrap();
    assert_eq!(subtype, b"Ink");
    let ink_list = annots[0]
        .get(b"InkList")
        .expect("/InkList present")
        .as_array()
        .expect("/InkList is an array");
    assert_eq!(ink_list.len(), 2, "expected two stroke arrays");
}

#[test]
#[serial_test::serial]
fn annotations_target_correct_page() {
    let dir = TempDir::new().unwrap();
    let in_path = dir.path().join("blank.pdf");
    fixtures::write_blank_pdf(&in_path, 3);

    let mut doc = Document::open(&in_path).expect("open");
    doc.add_annotation(&Annotation::Rect {
        page: 2,
        bbox: Rect::new(50.0, 50.0, 100.0, 100.0),
        stroke: Stroke::new(Color::RED, 2.0),
        fill: None,
    })
    .unwrap();
    let out_path = dir.path().join("annotated.pdf");
    doc.save(&out_path).expect("save");
    drop(doc);

    let saved = LoDoc::load(&out_path).expect("lopdf load");
    assert_eq!(annots_on_page(&saved, 0).len(), 0, "page 0 untouched");
    assert_eq!(annots_on_page(&saved, 1).len(), 0, "page 1 untouched");
    assert_eq!(
        annots_on_page(&saved, 2).len(),
        1,
        "page 2 has the annotation"
    );
}

#[test]
#[serial_test::serial]
fn ellipse_round_trip_through_extract() {
    let dir = TempDir::new().unwrap();
    let in_path = dir.path().join("blank.pdf");
    fixtures::write_blank_pdf(&in_path, 1);

    let mut doc = Document::open(&in_path).expect("open");
    doc.add_annotation(&Annotation::Ellipse {
        page: 0,
        bbox: Rect::new(60.0, 80.0, 120.0, 90.0),
        stroke: Stroke::new(Color::rgba(0, 128, 255, 255), 2.5),
        fill: None,
    })
    .unwrap();
    let out_path = dir.path().join("ellipse.pdf");
    doc.save(&out_path).expect("save");
    drop(doc);

    // First: the saved file has a /Circle annotation on page 0.
    let saved = LoDoc::load(&out_path).expect("lopdf load");
    let annots = annots_on_page(&saved, 0);
    assert_eq!(annots.len(), 1);
    assert_eq!(
        annots[0].get(b"Subtype").unwrap().as_name().unwrap(),
        b"Circle"
    );

    // Second: re-open via Document and extract → should come back as Ellipse.
    let mut reopened = Document::open(&out_path).expect("reopen");
    let extracted = reopened.extract_annotations();
    let ellipse = extracted
        .iter()
        .find_map(|a| match a {
            Annotation::Ellipse { bbox, stroke, .. } => Some((*bbox, stroke.clone())),
            _ => None,
        })
        .expect("Ellipse not extracted");
    assert!((ellipse.0.x - 60.0).abs() < 0.5, "x = {}", ellipse.0.x);
    assert!(
        (ellipse.0.width - 120.0).abs() < 0.5,
        "width = {}",
        ellipse.0.width
    );
    assert_eq!(ellipse.1.color, Color::rgba(0, 128, 255, 255));
}

#[test]
#[serial_test::serial]
fn arrow_round_trip_preserves_ends() {
    // All three end variants should survive save → reopen → extract.
    for ends in [ArrowEnds::None, ArrowEnds::End, ArrowEnds::Both] {
        let dir = TempDir::new().unwrap();
        let in_path = dir.path().join("blank.pdf");
        fixtures::write_blank_pdf(&in_path, 1);

        let mut doc = Document::open(&in_path).expect("open");
        doc.add_annotation(&Annotation::Arrow {
            page: 0,
            from: Point::new(50.0, 50.0),
            to: Point::new(200.0, 150.0),
            stroke: Stroke::new(Color::rgba(255, 0, 0, 255), 3.0),
            ends,
        })
        .unwrap();
        let out_path = dir.path().join("arrow.pdf");
        doc.save(&out_path).expect("save");
        drop(doc);

        let saved = LoDoc::load(&out_path).expect("lopdf load");
        let annots = annots_on_page(&saved, 0);
        assert_eq!(annots.len(), 1, "ends = {ends:?}");
        assert_eq!(
            annots[0].get(b"Subtype").unwrap().as_name().unwrap(),
            b"Line"
        );

        let mut reopened = Document::open(&out_path).expect("reopen");
        let extracted = reopened.extract_annotations();
        let recovered_ends = extracted
            .iter()
            .find_map(|a| match a {
                Annotation::Arrow { ends, .. } => Some(*ends),
                _ => None,
            })
            .unwrap_or_else(|| panic!("Arrow not extracted (input ends = {ends:?})"));
        assert_eq!(recovered_ends, ends, "ends should round-trip");
    }
}

#[test]
#[serial_test::serial]
fn rect_dashed_stroke_round_trips_through_extract() {
    use previewer_core::StrokeStyle;
    let dir = TempDir::new().unwrap();
    let in_path = dir.path().join("blank.pdf");
    fixtures::write_blank_pdf(&in_path, 1);

    let mut doc = Document::open(&in_path).expect("open");
    doc.add_annotation(&Annotation::Rect {
        page: 0,
        bbox: Rect::new(40.0, 60.0, 120.0, 80.0),
        stroke: Stroke::with_style(Color::rgba(0, 128, 0, 255), 4.0, StrokeStyle::Dashed),
        fill: None,
    })
    .unwrap();
    let out_path = dir.path().join("dashed.pdf");
    doc.save(&out_path).unwrap();
    drop(doc);

    let mut reopened = Document::open(&out_path).expect("reopen");
    let extracted = reopened.extract_annotations();
    let s = extracted
        .iter()
        .find_map(|a| match a {
            Annotation::Rect { stroke, .. } => Some(stroke.clone()),
            _ => None,
        })
        .expect("Rect not extracted");
    assert_eq!(s.style, StrokeStyle::Dashed);
    assert!((s.width - 4.0).abs() < 0.5, "width = {}", s.width);
    assert_eq!(s.color, Color::rgba(0, 128, 0, 255));
}
