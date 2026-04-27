//! Text search tests.

use previewer_pdf::Document;
use tempfile::TempDir;

mod fixtures;

#[test]
#[serial_test::serial]
fn finds_known_marker_text() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("with_text.pdf");
    fixtures::write_pdf_with_text(&path, 0, "Lorem ipsum");

    let doc = Document::open(&path).expect("open");
    let matches = doc.find_text("Lorem").expect("search");

    assert!(!matches.is_empty(), "expected at least one match");
    assert_eq!(matches[0].page, 0);
}

#[test]
#[serial_test::serial]
fn missing_text_returns_empty() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("with_text.pdf");
    fixtures::write_pdf_with_text(&path, 0, "Lorem ipsum");

    let doc = Document::open(&path).expect("open");
    let matches = doc.find_text("nonexistent_string_xyz").expect("search");

    assert!(matches.is_empty(), "expected no matches");
}

#[test]
#[serial_test::serial]
fn one_occurrence_returns_one_match() {
    // pdfium reports a single match as a *list of segments* (one per glyph
    // run); we collapse these to one TextMatch.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("single.pdf");
    fixtures::write_pdf_with_text(&path, 0, "rental");

    let doc = Document::open(&path).expect("open");
    let matches = doc.find_text("rental").expect("search");

    assert_eq!(
        matches.len(),
        1,
        "expected exactly one match, got {matches:?}"
    );
}

#[test]
#[serial_test::serial]
fn search_finds_text_on_correct_page() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("with_text.pdf");
    // 2 blank pages, then page 2 has the marker.
    fixtures::write_pdf_with_text(&path, 2, "FINDME");

    let doc = Document::open(&path).expect("open");
    let matches = doc.find_text("FINDME").expect("search");

    assert!(!matches.is_empty());
    for m in &matches {
        assert_eq!(m.page, 2, "expected match on page index 2, got {}", m.page);
    }
}
