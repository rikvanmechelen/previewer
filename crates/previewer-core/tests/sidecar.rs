//! Sidecar (JSON-on-disk) persistence tests.

use pretty_assertions::assert_eq;
use previewer_core::{
    Annotation, AnnotationLayer, Color, Rect, Stroke, load_layer, save_layer, sidecar_path,
};
use std::path::PathBuf;
use tempfile::TempDir;

fn one_annotation_layer() -> AnnotationLayer {
    AnnotationLayer {
        items: vec![Annotation::Rect {
            page: 0,
            bbox: Rect::new(10.0, 20.0, 100.0, 50.0),
            stroke: Stroke::new(Color::RED, 2.0),
            fill: None,
        }],
    }
}

#[test]
fn sidecar_path_appends_suffix() {
    let p = sidecar_path(&PathBuf::from("/tmp/foo.png"));
    assert_eq!(p, PathBuf::from("/tmp/foo.png.previewer.json"));
}

#[test]
fn sidecar_path_handles_no_extension() {
    let p = sidecar_path(&PathBuf::from("/tmp/foo"));
    assert_eq!(p, PathBuf::from("/tmp/foo.previewer.json"));
}

#[test]
fn save_then_load_round_trip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("layer.json");
    let layer = one_annotation_layer();

    save_layer(&layer, &path).expect("save");
    let loaded = load_layer(&path).expect("load");

    assert_eq!(layer, loaded);
}

#[test]
fn load_missing_file_is_error() {
    let result = load_layer("/this/path/does/not/exist.json");
    assert!(result.is_err());
}

#[test]
fn load_invalid_json_is_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bad.json");
    std::fs::write(&path, b"this is not json").unwrap();

    let result = load_layer(&path);
    assert!(result.is_err());
}
