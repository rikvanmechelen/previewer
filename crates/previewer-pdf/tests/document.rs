//! Integration tests for `previewer_pdf::Document`.
//!
//! Fixtures are generated programmatically by pdfium-render itself (no
//! checked-in binary blobs). All tests run headless — pdfium-render only
//! needs the libpdfium.so binary, not a display.

use pretty_assertions::assert_eq;
use previewer_pdf::Document;
use tempfile::TempDir;

mod fixtures;

#[test]
#[serial_test::serial]
fn page_count_matches_creation() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("three.pdf");
    fixtures::write_blank_pdf(&path, 3);

    let doc = Document::open(&path).expect("open PDF");
    assert_eq!(doc.page_count(), 3);
}

#[test]
#[serial_test::serial]
fn page_count_one() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("one.pdf");
    fixtures::write_blank_pdf(&path, 1);

    let doc = Document::open(&path).expect("open PDF");
    assert_eq!(doc.page_count(), 1);
}

#[test]
#[serial_test::serial]
fn open_nonexistent_returns_error() {
    let result = Document::open("/this/path/does/not/exist.pdf");
    assert!(result.is_err());
}
