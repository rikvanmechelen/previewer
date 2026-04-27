//! `ViewTransform` round-trip tests.
//!
//! We test the math (image-coord ↔ widget-coord) without involving Cairo so
//! these run instantly and can use property tests if we add them later.

use approx::assert_relative_eq;
use previewer_core::Point;
use previewer_render::ViewTransform;

const EPS: f64 = 1e-6;

/// `original_size`, `zoom`, `rotation_quarters`, expected `widget_size`.
fn build(orig: (u32, u32), zoom: f64, q: u8) -> ViewTransform {
    ViewTransform::for_image(orig, zoom, q)
}

#[test]
fn widget_size_for_each_rotation() {
    let orig = (200, 100);
    assert_eq!(build(orig, 1.0, 0).widget_size(), (200.0, 100.0));
    assert_eq!(build(orig, 1.0, 1).widget_size(), (100.0, 200.0));
    assert_eq!(build(orig, 1.0, 2).widget_size(), (200.0, 100.0));
    assert_eq!(build(orig, 1.0, 3).widget_size(), (100.0, 200.0));
    assert_eq!(build(orig, 2.0, 1).widget_size(), (200.0, 400.0));
}

#[test]
fn round_trip_each_rotation_at_zoom_1() {
    let orig = (200, 100);
    for q in 0..4 {
        let t = build(orig, 1.0, q);
        for &p in &[
            Point::new(0.0, 0.0),
            Point::new(50.0, 30.0),
            Point::new(199.0, 99.0),
        ] {
            let widget = t.image_to_widget(p);
            let back = t.widget_to_image(widget);
            assert_relative_eq!(back.x, p.x, epsilon = EPS);
            assert_relative_eq!(back.y, p.y, epsilon = EPS);
        }
    }
}

#[test]
fn round_trip_with_zoom() {
    let t = build((200, 100), 2.5, 2);
    let p = Point::new(73.0, 21.0);
    let widget = t.image_to_widget(p);
    let back = t.widget_to_image(widget);
    assert_relative_eq!(back.x, p.x, epsilon = EPS);
    assert_relative_eq!(back.y, p.y, epsilon = EPS);
}

#[test]
fn rotation_zero_origin_maps_to_widget_origin() {
    let t = build((200, 100), 1.5, 0);
    let p = Point::new(0.0, 0.0);
    let w = t.image_to_widget(p);
    assert_relative_eq!(w.x, 0.0, epsilon = EPS);
    assert_relative_eq!(w.y, 0.0, epsilon = EPS);
}

#[test]
fn rotation_90_cw_maps_origin_to_top_right() {
    // 200×100 image rotated 90° CW becomes 100×200; image (0,0) lands at
    // widget (widget_w, 0) = (100, 0) at zoom 1.0.
    let t = build((200, 100), 1.0, 1);
    let w = t.image_to_widget(Point::new(0.0, 0.0));
    assert_relative_eq!(w.x, 100.0, epsilon = EPS);
    assert_relative_eq!(w.y, 0.0, epsilon = EPS);
}

#[test]
fn rotation_180_maps_origin_to_bottom_right() {
    let t = build((200, 100), 1.0, 2);
    let w = t.image_to_widget(Point::new(0.0, 0.0));
    assert_relative_eq!(w.x, 200.0, epsilon = EPS);
    assert_relative_eq!(w.y, 100.0, epsilon = EPS);
}

#[test]
fn rotation_270_cw_maps_origin_to_bottom_left() {
    let t = build((200, 100), 1.0, 3);
    let w = t.image_to_widget(Point::new(0.0, 0.0));
    assert_relative_eq!(w.x, 0.0, epsilon = EPS);
    assert_relative_eq!(w.y, 200.0, epsilon = EPS);
}
