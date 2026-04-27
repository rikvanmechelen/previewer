//! Hit-testing + selection chrome rendering for annotations.

use cairo::Context;
use previewer_core::{Annotation, Point, Rect};

/// Identifies one of the resize handles around a selected annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleId {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    /// Arrow endpoints.
    ArrowFrom,
    ArrowTo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitKind {
    Body,
    Handle(HandleId),
}

/// Half-extent of a handle's hit area in image-coord units. Handles are
/// drawn as squares `2*HANDLE_HALF` on a side, centred on the geometric
/// anchor (e.g. top-left corner).
pub const HANDLE_HALF: f64 = 6.0;

/// Hit-test a single annotation. If `is_selected`, handle hits are checked
/// first so a click in a handle's hot zone never falls through to body
/// movement.
pub fn hit_test(ann: &Annotation, p: Point, tol: f64, is_selected: bool) -> Option<HitKind> {
    if is_selected && let Some(h) = hit_handle(ann, p) {
        return Some(HitKind::Handle(h));
    }
    if hit_body(ann, p, tol) {
        return Some(HitKind::Body);
    }
    None
}

fn hit_body(ann: &Annotation, p: Point, tol: f64) -> bool {
    match ann {
        Annotation::Rect { bbox, .. }
        | Annotation::Ellipse { bbox, .. }
        | Annotation::Highlight { bbox, .. }
        | Annotation::Stamp { bbox, .. } => point_in_rect(p, *bbox, tol),
        Annotation::Arrow {
            from, to, stroke, ..
        } => point_near_segment(p, *from, *to) <= (stroke.width.max(2.0) + tol),
        Annotation::FreeText {
            position,
            text,
            font,
            ..
        } => {
            let (w, h) = crate::freetext_bbox_size(text, font);
            point_in_rect(p, Rect::new(position.x, position.y, w, h), tol)
        }
        Annotation::Ink { strokes, width, .. } => strokes.iter().any(|s| {
            s.windows(2)
                .any(|seg| point_near_segment(p, seg[0], seg[1]) <= (*width + tol).max(3.0))
        }),
    }
}

fn hit_handle(ann: &Annotation, p: Point) -> Option<HandleId> {
    for (id, anchor) in handle_anchors(ann) {
        if (p.x - anchor.x).abs() <= HANDLE_HALF && (p.y - anchor.y).abs() <= HANDLE_HALF {
            return Some(id);
        }
    }
    None
}

/// All handle positions for an annotation, in image-coord units. Empty for
/// types that don't expose handles.
pub fn handle_anchors(ann: &Annotation) -> Vec<(HandleId, Point)> {
    match ann {
        Annotation::Rect { bbox, .. }
        | Annotation::Ellipse { bbox, .. }
        | Annotation::Highlight { bbox, .. }
        | Annotation::Stamp { bbox, .. } => box_handle_anchors(*bbox),
        Annotation::Arrow { from, to, .. } => {
            vec![(HandleId::ArrowFrom, *from), (HandleId::ArrowTo, *to)]
        }
        Annotation::FreeText { .. } | Annotation::Ink { .. } => Vec::new(),
    }
}

fn box_handle_anchors(bbox: Rect) -> Vec<(HandleId, Point)> {
    let l = bbox.x;
    let r = bbox.x + bbox.width;
    let t = bbox.y;
    let b = bbox.y + bbox.height;
    let cx = bbox.x + bbox.width / 2.0;
    let cy = bbox.y + bbox.height / 2.0;
    vec![
        (HandleId::TopLeft, Point::new(l, t)),
        (HandleId::Top, Point::new(cx, t)),
        (HandleId::TopRight, Point::new(r, t)),
        (HandleId::Right, Point::new(r, cy)),
        (HandleId::BottomRight, Point::new(r, b)),
        (HandleId::Bottom, Point::new(cx, b)),
        (HandleId::BottomLeft, Point::new(l, b)),
        (HandleId::Left, Point::new(l, cy)),
    ]
}

fn point_in_rect(p: Point, rect: Rect, tol: f64) -> bool {
    p.x >= rect.x - tol
        && p.x <= rect.x + rect.width + tol
        && p.y >= rect.y - tol
        && p.y <= rect.y + rect.height + tol
}

fn point_near_segment(p: Point, a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len_sq = dx * dx + dy * dy;
    if len_sq < f64::EPSILON {
        let ex = p.x - a.x;
        let ey = p.y - a.y;
        return (ex * ex + ey * ey).sqrt();
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len_sq).clamp(0.0, 1.0);
    let cx = a.x + t * dx;
    let cy = a.y + t * dy;
    let ex = p.x - cx;
    let ey = p.y - cy;
    (ex * ex + ey * ey).sqrt()
}

/// Draw the dashed selection outline + handles for a single annotation.
pub fn paint_selection(cr: &Context, ann: &Annotation) {
    cr.save().unwrap();

    // Dashed accent outline of the body bbox (or arrow line).
    cr.set_source_rgba(0.18, 0.55, 0.93, 0.9);
    cr.set_line_width(1.2);
    cr.set_dash(&[4.0, 3.0], 0.0);

    match ann {
        Annotation::Rect { bbox, .. }
        | Annotation::Ellipse { bbox, .. }
        | Annotation::Highlight { bbox, .. }
        | Annotation::Stamp { bbox, .. } => {
            cr.rectangle(bbox.x, bbox.y, bbox.width, bbox.height);
            let _ = cr.stroke();
        }
        Annotation::Arrow { from, to, .. } => {
            cr.move_to(from.x, from.y);
            cr.line_to(to.x, to.y);
            let _ = cr.stroke();
        }
        Annotation::FreeText {
            position,
            text,
            font,
            ..
        } => {
            let (w, h) = crate::freetext_bbox_size(text, font);
            cr.rectangle(position.x, position.y, w, h);
            let _ = cr.stroke();
        }
        Annotation::Ink { strokes, .. } => {
            // Bounding box of all stroke points.
            let mut min_x = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_y = f64::NEG_INFINITY;
            for s in strokes {
                for p in s {
                    min_x = min_x.min(p.x);
                    max_x = max_x.max(p.x);
                    min_y = min_y.min(p.y);
                    max_y = max_y.max(p.y);
                }
            }
            if min_x.is_finite() {
                cr.rectangle(min_x, min_y, max_x - min_x, max_y - min_y);
                let _ = cr.stroke();
            }
        }
    }

    cr.set_dash(&[], 0.0);

    // Handles: filled white square with blue outline.
    for (_, anchor) in handle_anchors(ann) {
        cr.rectangle(
            anchor.x - HANDLE_HALF,
            anchor.y - HANDLE_HALF,
            HANDLE_HALF * 2.0,
            HANDLE_HALF * 2.0,
        );
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(0.18, 0.55, 0.93, 1.0);
        cr.set_line_width(1.2);
        let _ = cr.stroke();
    }

    cr.restore().unwrap();
}

/// Apply a drag of `(dx, dy)` (in image-coord units) to `ann`, given the
/// drag's `kind`. Returns the modified annotation.
pub fn apply_drag(original: &Annotation, kind: DragKind, dx: f64, dy: f64) -> Annotation {
    match kind {
        DragKind::Move => translate_annotation(original, dx, dy),
        DragKind::Resize(handle) => resize_annotation(original, handle, dx, dy),
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DragKind {
    Move,
    Resize(HandleId),
}

fn translate_annotation(ann: &Annotation, dx: f64, dy: f64) -> Annotation {
    let mut out = ann.clone();
    match &mut out {
        Annotation::Rect { bbox, .. }
        | Annotation::Ellipse { bbox, .. }
        | Annotation::Highlight { bbox, .. }
        | Annotation::Stamp { bbox, .. } => {
            bbox.x += dx;
            bbox.y += dy;
        }
        Annotation::Arrow { from, to, .. } => {
            from.x += dx;
            from.y += dy;
            to.x += dx;
            to.y += dy;
        }
        Annotation::FreeText { position, .. } => {
            position.x += dx;
            position.y += dy;
        }
        Annotation::Ink { strokes, .. } => {
            for s in strokes {
                for p in s {
                    p.x += dx;
                    p.y += dy;
                }
            }
        }
    }
    out
}

fn resize_annotation(ann: &Annotation, handle: HandleId, dx: f64, dy: f64) -> Annotation {
    let mut out = ann.clone();
    match (&mut out, handle) {
        // Box handles
        (
            Annotation::Rect { bbox, .. }
            | Annotation::Ellipse { bbox, .. }
            | Annotation::Highlight { bbox, .. }
            | Annotation::Stamp { bbox, .. },
            h @ (HandleId::TopLeft
            | HandleId::Top
            | HandleId::TopRight
            | HandleId::Right
            | HandleId::BottomRight
            | HandleId::Bottom
            | HandleId::BottomLeft
            | HandleId::Left),
        ) => {
            apply_box_resize(bbox, h, dx, dy);
        }
        (Annotation::Arrow { from, .. }, HandleId::ArrowFrom) => {
            from.x += dx;
            from.y += dy;
        }
        (Annotation::Arrow { to, .. }, HandleId::ArrowTo) => {
            to.x += dx;
            to.y += dy;
        }
        // Resize handles on types we don't support resizing for: do nothing.
        _ => {}
    }
    out
}

fn apply_box_resize(bbox: &mut Rect, handle: HandleId, dx: f64, dy: f64) {
    let mut left = bbox.x;
    let mut top = bbox.y;
    let mut right = bbox.x + bbox.width;
    let mut bottom = bbox.y + bbox.height;
    match handle {
        HandleId::TopLeft => {
            left += dx;
            top += dy;
        }
        HandleId::Top => {
            top += dy;
        }
        HandleId::TopRight => {
            right += dx;
            top += dy;
        }
        HandleId::Right => {
            right += dx;
        }
        HandleId::BottomRight => {
            right += dx;
            bottom += dy;
        }
        HandleId::Bottom => {
            bottom += dy;
        }
        HandleId::BottomLeft => {
            left += dx;
            bottom += dy;
        }
        HandleId::Left => {
            left += dx;
        }
        _ => return,
    }
    let nx = left.min(right);
    let ny = top.min(bottom);
    let nw = (right - left).abs().max(1.0);
    let nh = (bottom - top).abs().max(1.0);
    *bbox = Rect::new(nx, ny, nw, nh);
}
