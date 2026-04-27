//! JSON round-trip tests for the annotation model.
//!
//! The annotation enum is the central data structure of the workspace; if it
//! serialises stably we can persist sidecar JSONs for images, hand
//! `AnnotationLayer` between threads, and (later) embed a JSON snapshot in
//! the PDF's `/Metadata` for editability.

use pretty_assertions::assert_eq;
use previewer_core::{
    Annotation, AnnotationLayer, ArrowEnds, Color, FontSpec, Point, Rect, Stroke, StrokeStyle,
};

fn sample_layer() -> AnnotationLayer {
    AnnotationLayer {
        items: vec![
            Annotation::Rect {
                page: 0,
                bbox: Rect::new(10.0, 20.0, 100.0, 50.0),
                stroke: Stroke {
                    color: Color::rgba(255, 0, 0, 255),
                    width: 2.0,
                    style: StrokeStyle::Solid,
                },
                fill: None,
            },
            Annotation::Ellipse {
                page: 0,
                bbox: Rect::new(50.0, 60.0, 80.0, 80.0),
                stroke: Stroke {
                    color: Color::rgba(0, 0, 0, 255),
                    width: 1.5,
                    style: StrokeStyle::Solid,
                },
                fill: Some(Color::rgba(255, 255, 0, 128)),
            },
            Annotation::Arrow {
                page: 0,
                from: Point::new(0.0, 0.0),
                to: Point::new(200.0, 150.0),
                stroke: Stroke {
                    color: Color::rgba(0, 0, 255, 255),
                    width: 3.0,
                    style: StrokeStyle::Solid,
                },
                ends: ArrowEnds::End,
            },
            Annotation::FreeText {
                page: 0,
                position: Point::new(15.0, 40.0),
                text: "hello, world".into(),
                font: FontSpec {
                    family: "Cantarell".into(),
                    size: 14.0,
                },
                color: Color::rgba(0, 0, 0, 255),
                is_placeholder: false,
            },
            Annotation::Highlight {
                page: 0,
                bbox: Rect::new(5.0, 100.0, 250.0, 18.0),
                color: Color::rgba(255, 255, 0, 96),
            },
        ],
    }
}

#[test]
fn annotation_layer_round_trips_via_json() {
    let layer = sample_layer();
    let json = serde_json::to_string(&layer).expect("serialize");
    let parsed: AnnotationLayer = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(layer, parsed);
}

#[test]
fn empty_layer_round_trips() {
    let layer = AnnotationLayer::default();
    let json = serde_json::to_string(&layer).expect("serialize");
    let parsed: AnnotationLayer = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(layer, parsed);
}

#[test]
fn arrow_with_double_ends_round_trips() {
    let layer = AnnotationLayer {
        items: vec![Annotation::Arrow {
            page: 0,
            from: Point::new(10.0, 10.0),
            to: Point::new(50.0, 80.0),
            stroke: Stroke {
                color: Color::rgba(255, 0, 0, 255),
                width: 2.0,
                style: StrokeStyle::Solid,
            },
            ends: ArrowEnds::Both,
        }],
    };
    let json = serde_json::to_string(&layer).expect("serialize");
    let parsed: AnnotationLayer = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(layer, parsed);
}

#[test]
fn legacy_arrow_json_without_ends_field_defaults_to_single_arrow() {
    // Older sidecar JSON files don't carry `ends`. The model must accept
    // them so users don't lose old image annotations on upgrade.
    let legacy = r#"{
        "items": [{
            "type": "arrow",
            "page": 0,
            "from": {"x": 0.0, "y": 0.0},
            "to": {"x": 100.0, "y": 0.0},
            "stroke": {"color": {"r": 0, "g": 0, "b": 0, "a": 255}, "width": 2.0}
        }]
    }"#;
    let parsed: AnnotationLayer = serde_json::from_str(legacy).expect("legacy parses");
    let Annotation::Arrow { ends, .. } = &parsed.items[0] else {
        panic!("expected Arrow, got {:?}", parsed.items[0]);
    };
    assert_eq!(*ends, ArrowEnds::End);
}

#[test]
fn legacy_freetext_json_without_is_placeholder_field_defaults_to_false() {
    // Legacy compat: existing sidecar JSONs predate the placeholder UX
    // and must continue to parse cleanly with `is_placeholder = false`.
    let legacy = r#"{
        "items": [{
            "type": "free_text",
            "page": 0,
            "position": {"x": 10.0, "y": 20.0},
            "text": "hello",
            "font": {"family": "Helvetica", "size": 14.0},
            "color": {"r": 0, "g": 0, "b": 0, "a": 255}
        }]
    }"#;
    let parsed: AnnotationLayer = serde_json::from_str(legacy).expect("legacy parses");
    let Annotation::FreeText { is_placeholder, .. } = &parsed.items[0] else {
        panic!("expected FreeText");
    };
    assert!(!is_placeholder);
}

#[test]
fn freetext_with_placeholder_round_trips() {
    let layer = AnnotationLayer {
        items: vec![Annotation::FreeText {
            page: 0,
            position: Point::new(10.0, 20.0),
            text: "Enter some text".into(),
            font: FontSpec::default(),
            color: Color::rgba(0, 0, 0, 255),
            is_placeholder: true,
        }],
    };
    let json = serde_json::to_string(&layer).expect("serialize");
    let parsed: AnnotationLayer = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(layer, parsed);
}

#[test]
fn legacy_stroke_json_without_style_field_defaults_to_solid() {
    let legacy = r#"{
        "items": [{
            "type": "rect",
            "page": 0,
            "bbox": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0},
            "stroke": {"color": {"r": 255, "g": 0, "b": 0, "a": 255}, "width": 2.0},
            "fill": null
        }]
    }"#;
    let parsed: AnnotationLayer = serde_json::from_str(legacy).expect("legacy parses");
    let Annotation::Rect { stroke, .. } = &parsed.items[0] else {
        panic!("expected Rect");
    };
    assert_eq!(stroke.style, StrokeStyle::Solid);
}

#[test]
fn dashed_stroke_round_trips() {
    let layer = AnnotationLayer {
        items: vec![Annotation::Rect {
            page: 0,
            bbox: Rect::new(0.0, 0.0, 50.0, 50.0),
            stroke: Stroke::with_style(Color::rgba(0, 128, 255, 255), 3.0, StrokeStyle::Dashed),
            fill: None,
        }],
    };
    let json = serde_json::to_string(&layer).expect("serialize");
    let parsed: AnnotationLayer = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(layer, parsed);
}

#[test]
fn json_layout_is_stable() {
    // Snapshot via insta. Catches accidental schema changes (renamed fields,
    // changed tags). On first run, review and accept with `cargo insta accept`.
    let layer = sample_layer();
    let json = serde_json::to_string_pretty(&layer).unwrap();
    insta::assert_snapshot!(json);
}
