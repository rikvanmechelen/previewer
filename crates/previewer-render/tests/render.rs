//! Off-screen Cairo render tests. No display server required.
//!
//! Strategy: paint annotations onto an `ImageSurface`, then sample pixels at
//! known coordinates. We deliberately do **not** pixel-diff entire images —
//! anti-aliasing varies across Cairo versions.

use cairo::{Context, Format, ImageSurface};
use previewer_core::{Annotation, AnnotationLayer, Color, Point, Rect, Stroke};
use previewer_render::paint_annotations;

const W: i32 = 300;
const H: i32 = 300;

fn fresh_white_surface() -> ImageSurface {
    let surface = ImageSurface::create(Format::ARgb32, W, H).unwrap();
    let cr = Context::new(&surface).unwrap();
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.paint().unwrap();
    drop(cr);
    surface
}

/// Read a single pixel as `[r, g, b, a]`. Cairo's `ARgb32` is BGRA in memory
/// on little-endian, premultiplied alpha — this normalises back to plain RGBA.
fn pixel_at(surface: &mut ImageSurface, x: usize, y: usize) -> [u8; 4] {
    let stride = surface.stride() as usize;
    let data = surface.data().unwrap();
    let i = y * stride + x * 4;
    let b = data[i];
    let g = data[i + 1];
    let r = data[i + 2];
    let a = data[i + 3];
    [r, g, b, a]
}

fn is_red(p: [u8; 4]) -> bool {
    p[0] > 200 && p[1] < 60 && p[2] < 60
}

fn is_white(p: [u8; 4]) -> bool {
    p[0] > 240 && p[1] > 240 && p[2] > 240
}

fn is_yellow(p: [u8; 4]) -> bool {
    p[0] > 200 && p[1] > 200 && p[2] < 60
}

#[test]
fn rect_stroke_paints_perimeter_and_leaves_interior() {
    let mut surface = fresh_white_surface();
    let cr = Context::new(&surface).unwrap();

    let layer = AnnotationLayer {
        items: vec![Annotation::Rect {
            page: 0,
            bbox: Rect::new(100.0, 100.0, 100.0, 100.0),
            stroke: Stroke::new(Color::RED, 4.0),
            fill: None,
        }],
    };
    paint_annotations(&cr, &layer);
    drop(cr);

    // Top edge of the stroked perimeter (y=100, mid-x): should be red.
    assert!(
        is_red(pixel_at(&mut surface, 150, 100)),
        "top edge not red: {:?}",
        pixel_at(&mut surface, 150, 100)
    );
    // Centre interior (no fill): should remain white.
    assert!(
        is_white(pixel_at(&mut surface, 150, 150)),
        "interior not white: {:?}",
        pixel_at(&mut surface, 150, 150)
    );
}

#[test]
fn rect_fill_colours_interior() {
    let mut surface = fresh_white_surface();
    let cr = Context::new(&surface).unwrap();

    let layer = AnnotationLayer {
        items: vec![Annotation::Rect {
            page: 0,
            bbox: Rect::new(100.0, 100.0, 100.0, 100.0),
            stroke: Stroke::new(Color::BLACK, 1.0),
            fill: Some(Color::rgba(255, 255, 0, 255)),
        }],
    };
    paint_annotations(&cr, &layer);
    drop(cr);

    assert!(
        is_yellow(pixel_at(&mut surface, 150, 150)),
        "interior not yellow: {:?}",
        pixel_at(&mut surface, 150, 150)
    );
}

#[test]
fn empty_layer_leaves_surface_unchanged() {
    let mut surface = fresh_white_surface();
    let cr = Context::new(&surface).unwrap();
    paint_annotations(&cr, &AnnotationLayer::default());
    drop(cr);

    assert!(is_white(pixel_at(&mut surface, 150, 150)));
    assert!(is_white(pixel_at(&mut surface, 0, 0)));
    assert!(is_white(pixel_at(
        &mut surface,
        W as usize - 1,
        H as usize - 1
    )));
}

#[test]
fn arrow_paints_along_path() {
    let mut surface = fresh_white_surface();
    let cr = Context::new(&surface).unwrap();

    let layer = AnnotationLayer {
        items: vec![Annotation::Arrow {
            page: 0,
            from: Point::new(50.0, 150.0),
            to: Point::new(250.0, 150.0),
            stroke: Stroke::new(Color::RED, 4.0),
            ends: previewer_core::ArrowEnds::End,
        }],
    };
    paint_annotations(&cr, &layer);
    drop(cr);

    // Midpoint of the shaft.
    assert!(
        is_red(pixel_at(&mut surface, 150, 150)),
        "arrow shaft not red: {:?}",
        pixel_at(&mut surface, 150, 150)
    );
    // Off the arrow path (well above) — should stay white.
    assert!(
        is_white(pixel_at(&mut surface, 150, 50)),
        "off-path pixel not white: {:?}",
        pixel_at(&mut surface, 150, 50)
    );
}

#[test]
fn highlight_tints_pixels_under_it() {
    let mut surface = fresh_white_surface();
    let cr = Context::new(&surface).unwrap();

    let layer = AnnotationLayer {
        items: vec![Annotation::Highlight {
            page: 0,
            bbox: Rect::new(100.0, 100.0, 100.0, 50.0),
            color: Color::rgba(255, 255, 0, 128),
        }],
    };
    paint_annotations(&cr, &layer);
    drop(cr);

    let p = pixel_at(&mut surface, 150, 125);
    // Yellow + white = light yellow: high R, high G, low B.
    assert!(p[0] > 200 && p[1] > 200, "highlight didn't tint: {:?}", p);
    assert!(p[2] < 200, "highlight should reduce blue: {:?}", p);
}

#[test]
fn freetext_followed_by_ellipse_does_not_leak_path() {
    // Regression for "save draws a line from edge of ellipse to end of
    // text". `cr.show_text()` leaves the current point at the tail of the
    // rendered glyphs; `cr.arc()` then connects from there to the arc's
    // start, and the ellipse stroke renders that bridge as a stray line.
    // We paint FreeText then Ellipse and assert that no red pixels
    // appear along the diagonal connecting them.
    use previewer_core::FontSpec;
    let mut surface = fresh_white_surface();
    let cr = Context::new(&surface).unwrap();

    let text_pos = Point::new(50.0, 50.0);
    let ellipse_bbox = Rect::new(200.0, 250.0, 80.0, 60.0);
    let layer = AnnotationLayer {
        items: vec![
            Annotation::FreeText {
                page: 0,
                position: text_pos,
                text: "hello world".into(),
                font: FontSpec {
                    family: "Helvetica".into(),
                    size: 16.0,
                },
                color: Color::rgba(0, 0, 0, 255),
                is_placeholder: false,
            },
            Annotation::Ellipse {
                page: 0,
                bbox: ellipse_bbox,
                stroke: Stroke::new(Color::RED, 3.0),
                fill: None,
            },
        ],
    };
    paint_annotations(&cr, &layer);
    drop(cr);

    // Without the fix, a red diagonal runs from roughly (130, 66) — the
    // right end of "hello world"'s baseline — to (280, 280), the ellipse's
    // right edge. Sample three points well clear of the ellipse bbox.
    for (x, y) in [(170usize, 110usize), (200, 160), (230, 210)] {
        assert!(
            !is_red(pixel_at(&mut surface, x, y)),
            "stray red along text→ellipse diagonal at ({x}, {y}): {:?}",
            pixel_at(&mut surface, x, y)
        );
    }
}

#[test]
fn freetext_then_ellipse_user_repro() {
    // Reproduce the user's bug: draw an ellipse, write text. After save the
    // re-extracted layer comes back as [FreeText, Ellipse] (lopdf walks
    // /Annots in array order, freetext attached first). Painting them in
    // that order should NOT leave a stray red line connecting them.
    use previewer_core::FontSpec;
    let mut surface = fresh_white_surface();
    let cr = Context::new(&surface).unwrap();

    // Big surface so we can place them with a wide gap.
    let layer = AnnotationLayer {
        items: vec![
            Annotation::FreeText {
                page: 0,
                position: Point::new(50.0, 50.0),
                text: "hello world".into(),
                font: FontSpec {
                    family: "Helvetica".into(),
                    size: 16.0,
                },
                color: Color::rgba(0, 0, 0, 255),
                is_placeholder: false,
            },
            Annotation::Ellipse {
                page: 0,
                bbox: Rect::new(180.0, 200.0, 80.0, 60.0),
                stroke: Stroke::new(Color::RED, 3.0),
                fill: None,
            },
        ],
    };
    paint_annotations(&cr, &layer);
    drop(cr);

    // Scan the line from approx (text-end ≈ 150, 60) to ellipse-right-edge
    // (260, 230). Sample several points along that diagonal.
    let from = (150.0_f64, 60.0_f64);
    let to = (260.0_f64, 230.0_f64);
    for t in [0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8] {
        let x = (from.0 + (to.0 - from.0) * t) as usize;
        let y = (from.1 + (to.1 - from.1) * t) as usize;
        let p = pixel_at(&mut surface, x, y);
        assert!(
            !is_red(p),
            "stray red pixel at ({x}, {y}) on line text→ellipse: {p:?}"
        );
    }
}
