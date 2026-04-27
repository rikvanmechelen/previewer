//! Page rendering tests.

use pretty_assertions::assert_eq;
use previewer_pdf::Document;
use tempfile::TempDir;

mod fixtures;

#[test]
#[serial_test::serial]
fn render_page_at_scale_one() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("one.pdf");
    fixtures::write_blank_pdf(&path, 1);

    let doc = Document::open(&path).expect("open");
    let rendered = doc.render_page(0, 1.0).expect("render");

    // A4 at 1 PDF point per pixel is ~595×842; pdfium rounds, so allow ±1.
    let (w, h) = rendered.dimensions();
    assert!((594..=596).contains(&w), "width was {w}");
    assert!((841..=843).contains(&h), "height was {h}");
    assert_eq!(
        rendered.pixels().len() as u32,
        rendered.width() * rendered.height() * 4
    );
}

#[test]
#[serial_test::serial]
fn render_page_scales_dimensions() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("one.pdf");
    fixtures::write_blank_pdf(&path, 1);

    let doc = Document::open(&path).expect("open");
    let rendered = doc.render_page(0, 2.0).expect("render");

    // 2× of ~595×842 → ~1190×1684, allow ±2 for rounding.
    let (w, h) = rendered.dimensions();
    assert!((1188..=1192).contains(&w), "width was {w}");
    assert!((1682..=1686).contains(&h), "height was {h}");
}

#[test]
#[serial_test::serial]
fn render_page_index_out_of_range_is_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("one.pdf");
    fixtures::write_blank_pdf(&path, 1);

    let doc = Document::open(&path).expect("open");
    assert!(doc.render_page(5, 1.0).is_err());
}
