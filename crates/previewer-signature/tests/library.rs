//! On-disk library round-trip tests.

use pretty_assertions::assert_eq;
use previewer_signature::{Library, Signature, SignatureId, SignatureKind, Stroke, StrokePoint};
use tempfile::TempDir;

fn vector_sig() -> Signature {
    Signature {
        id: SignatureId(42),
        name: "test-vec".into(),
        kind: SignatureKind::Vector {
            strokes: vec![Stroke {
                points: vec![
                    StrokePoint {
                        x: 0.0,
                        y: 0.0,
                        pressure: 0.5,
                    },
                    StrokePoint {
                        x: 10.0,
                        y: 5.0,
                        pressure: 1.0,
                    },
                ],
            }],
        },
    }
}

fn raster_sig() -> Signature {
    Signature {
        id: SignatureId(99),
        name: "test-raster".into(),
        kind: SignatureKind::Raster {
            width: 4,
            height: 2,
            pixels: vec![
                255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 255, 255, 0, 200, 0, 0, 0, 0, 128,
                128, 128, 128, 200, 200, 200, 200, 250, 250, 250, 250,
            ],
        },
    }
}

#[test]
fn save_then_load_vector_round_trip() {
    let dir = TempDir::new().unwrap();
    let lib = Library::at(dir.path());
    let original = vector_sig();
    lib.save(&original).unwrap();

    let loaded = lib.load_all().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0], original);
}

#[test]
fn save_then_load_raster_round_trip() {
    let dir = TempDir::new().unwrap();
    let lib = Library::at(dir.path());
    let original = raster_sig();
    lib.save(&original).unwrap();

    let loaded = lib.load_all().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0], original);
}

#[test]
fn load_all_returns_multiple_signatures() {
    let dir = TempDir::new().unwrap();
    let lib = Library::at(dir.path());
    lib.save(&vector_sig()).unwrap();
    lib.save(&raster_sig()).unwrap();

    let loaded = lib.load_all().unwrap();
    assert_eq!(loaded.len(), 2);
}

#[test]
fn delete_removes_only_target() {
    let dir = TempDir::new().unwrap();
    let lib = Library::at(dir.path());
    lib.save(&vector_sig()).unwrap();
    lib.save(&raster_sig()).unwrap();

    lib.delete(SignatureId(42)).unwrap();

    let loaded = lib.load_all().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, SignatureId(99));
}

#[test]
fn empty_library_returns_empty_vec() {
    let dir = TempDir::new().unwrap();
    let lib = Library::at(dir.path());
    let loaded = lib.load_all().unwrap();
    assert!(loaded.is_empty());
}

#[test]
fn library_ignores_non_sig_files_in_dir() {
    let dir = TempDir::new().unwrap();
    let lib = Library::at(dir.path());
    lib.save(&vector_sig()).unwrap();
    // A stray file the library shouldn't try to parse.
    std::fs::write(dir.path().join("note.txt"), b"hello").unwrap();

    let loaded = lib.load_all().unwrap();
    assert_eq!(loaded.len(), 1);
}
