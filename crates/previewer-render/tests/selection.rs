//! Hit-test tests.

use previewer_core::{Annotation, Color, Point, Rect, Stroke};
use previewer_render::{HandleId, HitKind, hit_test};

fn red_rect(x: f64, y: f64, w: f64, h: f64) -> Annotation {
    Annotation::Rect {
        page: 0,
        bbox: Rect::new(x, y, w, h),
        stroke: Stroke::new(Color::RED, 2.0),
        fill: None,
    }
}

#[test]
fn click_inside_rect_body_hits_body() {
    let r = red_rect(100.0, 100.0, 100.0, 50.0);
    assert_eq!(
        hit_test(&r, Point::new(150.0, 125.0), 0.0, false),
        Some(HitKind::Body)
    );
}

#[test]
fn click_outside_rect_misses() {
    let r = red_rect(100.0, 100.0, 100.0, 50.0);
    assert_eq!(hit_test(&r, Point::new(0.0, 0.0), 0.0, false), None);
}

#[test]
fn unselected_rect_does_not_expose_handles() {
    let r = red_rect(100.0, 100.0, 100.0, 50.0);
    // Click ON the top-left corner, with `selected = false`. Should still hit
    // body (corner is inside bbox+tol) — never a handle.
    let hit = hit_test(&r, Point::new(100.0, 100.0), 0.0, false);
    assert_eq!(hit, Some(HitKind::Body));
}

#[test]
fn selected_rect_top_left_handle_is_hittable() {
    let r = red_rect(100.0, 100.0, 100.0, 50.0);
    let hit = hit_test(&r, Point::new(100.0, 100.0), 0.0, true);
    assert_eq!(hit, Some(HitKind::Handle(HandleId::TopLeft)));
}

#[test]
fn selected_rect_bottom_right_handle_is_hittable() {
    let r = red_rect(100.0, 100.0, 100.0, 50.0);
    let hit = hit_test(&r, Point::new(200.0, 150.0), 0.0, true);
    assert_eq!(hit, Some(HitKind::Handle(HandleId::BottomRight)));
}

#[test]
fn arrow_endpoints_are_handles() {
    let arrow = Annotation::Arrow {
        page: 0,
        from: Point::new(50.0, 50.0),
        to: Point::new(150.0, 80.0),
        stroke: Stroke::new(Color::BLACK, 2.0),
        ends: previewer_core::ArrowEnds::End,
    };
    assert_eq!(
        hit_test(&arrow, Point::new(50.0, 50.0), 0.0, true),
        Some(HitKind::Handle(HandleId::ArrowFrom))
    );
    assert_eq!(
        hit_test(&arrow, Point::new(150.0, 80.0), 0.0, true),
        Some(HitKind::Handle(HandleId::ArrowTo))
    );
}

#[test]
fn arrow_body_hit_is_along_segment() {
    let arrow = Annotation::Arrow {
        page: 0,
        from: Point::new(0.0, 0.0),
        to: Point::new(100.0, 0.0),
        stroke: Stroke::new(Color::BLACK, 2.0),
        ends: previewer_core::ArrowEnds::End,
    };
    // Midpoint of segment with small tolerance.
    assert_eq!(
        hit_test(&arrow, Point::new(50.0, 0.5), 1.0, false),
        Some(HitKind::Body)
    );
    // Far above segment.
    assert_eq!(hit_test(&arrow, Point::new(50.0, 50.0), 1.0, false), None);
}
