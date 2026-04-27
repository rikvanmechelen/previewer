//! previewer-render: paint `previewer-core::AnnotationLayer` onto a Cairo
//! context. No GTK, no display server — every entry point here is testable
//! against an `ImageSurface` in unit tests.

use std::f64::consts::{PI, TAU};

use cairo::Context;

pub mod selection;
use previewer_core::{
    Annotation, AnnotationLayer, ArrowEnds, Color, FontSpec, Point, Rect, StampImage, Stroke,
    StrokeStyle,
};
pub use selection::{DragKind, HandleId, HitKind, apply_drag, hit_test, paint_selection};

/// Mapping between image-space (original pixel) coordinates and widget-space
/// (Cairo user-space) coordinates, accounting for zoom and 90°-step rotation.
///
/// Image-space is the **original**, unrotated image. Widget-space is what the
/// user sees and clicks on.
#[derive(Debug, Clone, Copy)]
pub struct ViewTransform {
    image_size: (u32, u32),
    zoom: f64,
    rotation_quarters: u8,
}

impl ViewTransform {
    pub fn for_image(image_size: (u32, u32), zoom: f64, rotation_quarters: u8) -> Self {
        Self {
            image_size,
            zoom,
            rotation_quarters: rotation_quarters % 4,
        }
    }

    /// Outer widget pixel size for the rotated, zoomed image.
    pub fn widget_size(&self) -> (f64, f64) {
        let (ow, oh) = (self.image_size.0 as f64, self.image_size.1 as f64);
        let (rw, rh) = match self.rotation_quarters {
            0 | 2 => (ow, oh),
            1 | 3 => (oh, ow),
            _ => unreachable!(),
        };
        (rw * self.zoom, rh * self.zoom)
    }

    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    pub fn rotation_quarters(&self) -> u8 {
        self.rotation_quarters
    }

    /// Map an image-space point to widget-space.
    pub fn image_to_widget(&self, p: Point) -> Point {
        let (ww, wh) = self.widget_size();
        let z = self.zoom;
        let (sx, sy) = (p.x * z, p.y * z);
        match self.rotation_quarters {
            0 => Point::new(sx, sy),
            1 => Point::new(ww - sy, sx),
            2 => Point::new(ww - sx, wh - sy),
            3 => Point::new(sy, wh - sx),
            _ => unreachable!(),
        }
    }

    /// Map a widget-space point to image-space (clicks → annotation coords).
    pub fn widget_to_image(&self, p: Point) -> Point {
        let (ww, wh) = self.widget_size();
        let z = self.zoom;
        let (ix, iy) = match self.rotation_quarters {
            0 => (p.x, p.y),
            1 => (p.y, ww - p.x),
            2 => (ww - p.x, wh - p.y),
            3 => (wh - p.y, p.x),
            _ => unreachable!(),
        };
        Point::new(ix / z, iy / z)
    }

    /// Apply this transform to `cr` so subsequent drawing is in image-space
    /// (i.e. you can pass image-coord annotations straight to
    /// [`paint_annotations`]).
    pub fn apply(&self, cr: &Context) {
        let (ww, wh) = self.widget_size();
        match self.rotation_quarters {
            0 => {}
            1 => {
                cr.translate(ww, 0.0);
                cr.rotate(PI / 2.0);
            }
            2 => {
                cr.translate(ww, wh);
                cr.rotate(PI);
            }
            3 => {
                cr.translate(0.0, wh);
                cr.rotate(3.0 * PI / 2.0);
            }
            _ => unreachable!(),
        }
        cr.scale(self.zoom, self.zoom);
    }
}

/// Paint every annotation in `layer` onto `cr`. Coordinates are interpreted in
/// the current user-space of `cr`; the caller is responsible for any
/// scale/translate/rotate that maps image-space → widget-space.
pub fn paint_annotations(cr: &Context, layer: &AnnotationLayer) {
    for ann in &layer.items {
        paint(cr, ann);
    }
}

fn paint(cr: &Context, ann: &Annotation) {
    match ann {
        Annotation::Rect {
            bbox, stroke, fill, ..
        } => paint_rect(cr, *bbox, stroke, *fill),
        Annotation::Ellipse {
            bbox, stroke, fill, ..
        } => paint_ellipse(cr, *bbox, stroke, *fill),
        Annotation::Arrow {
            from,
            to,
            stroke,
            ends,
            ..
        } => paint_arrow(cr, *from, *to, stroke, *ends),
        Annotation::FreeText {
            position,
            text,
            font,
            color,
            is_placeholder,
            ..
        } => {
            // Placeholder text renders dim so it reads as a prompt rather
            // than committed content. Halve the alpha against the user's
            // chosen colour so the type-in style still previews.
            let display = if *is_placeholder {
                Color::rgba(color.r, color.g, color.b, color.a / 2)
            } else {
                *color
            };
            paint_freetext(cr, *position, text, font, display);
        }
        Annotation::Highlight { bbox, color, .. } => paint_highlight(cr, *bbox, *color),
        Annotation::Stamp { bbox, image, .. } => paint_stamp(cr, *bbox, image),
        Annotation::Ink {
            strokes,
            color,
            width,
            ..
        } => paint_ink(cr, strokes, *color, *width),
    }
}

fn set_color(cr: &Context, c: Color) {
    let (r, g, b, a) = c.to_unit_rgba();
    cr.set_source_rgba(r, g, b, a);
}

/// Configure dash pattern + line cap on `cr` for the given stroke style.
/// Scaled to the stroke width so a 1pt and a 6pt dashed line both look
/// "dashy" rather than morse-code or barely-broken. Solid clears any
/// previous dash so callers don't bleed state across shapes.
fn apply_stroke_style(cr: &Context, stroke: &Stroke) {
    match stroke.style {
        StrokeStyle::Solid => {
            cr.set_dash(&[], 0.0);
            cr.set_line_cap(cairo::LineCap::Butt);
        }
        StrokeStyle::Dashed => {
            let on = (stroke.width * 4.0).max(3.0);
            let off = (stroke.width * 3.0).max(2.5);
            cr.set_dash(&[on, off], 0.0);
            cr.set_line_cap(cairo::LineCap::Butt);
        }
        StrokeStyle::Dotted => {
            // Zero-length on-dash + round cap renders perfect round dots
            // separated by `gap`. Without the round cap nothing draws.
            let gap = (stroke.width * 2.0).max(1.5);
            cr.set_dash(&[0.0, gap], 0.0);
            cr.set_line_cap(cairo::LineCap::Round);
        }
    }
}

fn paint_rect(cr: &Context, bbox: Rect, stroke: &Stroke, fill: Option<Color>) {
    cr.rectangle(bbox.x, bbox.y, bbox.width, bbox.height);
    if let Some(fill) = fill {
        set_color(cr, fill);
        let _ = cr.fill_preserve();
    }
    set_color(cr, stroke.color);
    cr.set_line_width(stroke.width);
    apply_stroke_style(cr, stroke);
    let _ = cr.stroke();
}

fn paint_ellipse(cr: &Context, bbox: Rect, stroke: &Stroke, fill: Option<Color>) {
    let cx = bbox.x + bbox.width / 2.0;
    let cy = bbox.y + bbox.height / 2.0;
    let rx = bbox.width / 2.0;
    let ry = bbox.height / 2.0;
    if rx <= 0.0 || ry <= 0.0 {
        return;
    }

    cr.save().unwrap();
    cr.translate(cx, cy);
    cr.scale(rx, ry);
    // `arc()` connects from the current point if one exists. After a prior
    // paint_freetext, `show_text()` leaves the current point at the end of
    // the rendered glyphs, and without this `new_sub_path()` the ellipse
    // stroke renders a stray line from there to the arc's start (right
    // edge of the ellipse). `new_sub_path()` discards the dangling current
    // point without clearing earlier subpaths.
    cr.new_sub_path();
    cr.arc(0.0, 0.0, 1.0, 0.0, TAU);
    cr.restore().unwrap();

    if let Some(fill) = fill {
        set_color(cr, fill);
        let _ = cr.fill_preserve();
    }
    set_color(cr, stroke.color);
    cr.set_line_width(stroke.width);
    apply_stroke_style(cr, stroke);
    let _ = cr.stroke();
}

fn paint_arrow(cr: &Context, from: Point, to: Point, stroke: &Stroke, ends: ArrowEnds) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < f64::EPSILON {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;

    // Arrowhead size scales with stroke width but stays bounded.
    let head_len = (stroke.width * 4.0).clamp(8.0, 24.0);

    let head_at_to = matches!(ends, ArrowEnds::End | ArrowEnds::Both);
    let head_at_from = matches!(ends, ArrowEnds::Both);

    // Shaft endpoints — pull each end inward by `head_len` whenever a head
    // sits there, so the filled triangle butts up cleanly without the
    // square stroke cap poking through.
    let (sx, sy) = if head_at_from {
        (from.x + ux * head_len, from.y + uy * head_len)
    } else {
        (from.x, from.y)
    };
    let (ex, ey) = if head_at_to {
        (to.x - ux * head_len, to.y - uy * head_len)
    } else {
        (to.x, to.y)
    };

    set_color(cr, stroke.color);
    cr.set_line_width(stroke.width);
    apply_stroke_style(cr, stroke);
    // Arrows look better with round caps at the shaft endpoints; the
    // `apply_stroke_style` Dotted branch sets Round, so we explicitly
    // override to Round for the Solid + Dashed branches too.
    if !matches!(stroke.style, StrokeStyle::Dotted) {
        cr.set_line_cap(cairo::LineCap::Round);
    }

    cr.move_to(sx, sy);
    cr.line_to(ex, ey);
    let _ = cr.stroke();

    // Filled head triangle at the requested end(s). The tip sits on the
    // original endpoint; the base is `head_len` upstream along the shaft.
    if head_at_to {
        paint_arrow_head(cr, to, ux, uy, head_len);
    }
    if head_at_from {
        // For the `from` head, the tip *is* `from`, and the head points
        // back toward the originating end (i.e. opposite the shaft
        // direction).
        paint_arrow_head(cr, from, -ux, -uy, head_len);
    }
}

/// Paint a filled triangle whose tip is at `tip` and whose base sits
/// `head_len` units along the unit-vector `(ux, uy)`.
fn paint_arrow_head(cr: &Context, tip: Point, ux: f64, uy: f64, head_len: f64) {
    let head_w = head_len * 0.6;
    let bx = tip.x - ux * head_len;
    let by = tip.y - uy * head_len;
    let px = -uy; // perpendicular (rotate +90°)
    let py = ux;
    let lx = bx + px * head_w / 2.0;
    let ly = by + py * head_w / 2.0;
    let rx = bx - px * head_w / 2.0;
    let ry = by - py * head_w / 2.0;
    cr.move_to(tip.x, tip.y);
    cr.line_to(lx, ly);
    cr.line_to(rx, ry);
    cr.close_path();
    let _ = cr.fill();
}

fn paint_freetext(cr: &Context, position: Point, text: &str, font: &FontSpec, color: Color) {
    cr.select_font_face(
        &font.family,
        cairo::FontSlant::Normal,
        cairo::FontWeight::Normal,
    );
    cr.set_font_size(font.size);
    set_color(cr, color);
    let line_height = font.size * 1.4;
    for (i, line) in text.split('\n').enumerate() {
        let y = position.y + font.size + (i as f64) * line_height;
        cr.move_to(position.x, y);
        let _ = cr.show_text(line);
    }
    // `show_text` advances the current point to the end of the last glyph.
    // Drop that state so the next annotation paint can't accidentally
    // extend its path from the text's tail. Belt and braces alongside the
    // `new_sub_path()` in `paint_ellipse`.
    cr.new_path();
}

/// Width × height of a multi-line FreeText block in image-coord units.
/// Used by hit-testing, selection chrome, and the PDF write path.
pub fn freetext_bbox_size(text: &str, font: &FontSpec) -> (f64, f64) {
    let line_count = text.split('\n').count().max(1);
    let max_chars = text
        .split('\n')
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(1)
        .max(1);
    let w = (max_chars as f64) * font.size * 0.6;
    let h = (line_count as f64) * font.size * 1.4;
    (w, h)
}

fn paint_highlight(cr: &Context, bbox: Rect, color: Color) {
    cr.rectangle(bbox.x, bbox.y, bbox.width, bbox.height);
    set_color(cr, color);
    let _ = cr.fill();
}

fn paint_stamp(cr: &Context, bbox: Rect, image: &StampImage) {
    if image.width == 0 || image.height == 0 || bbox.width <= 0.0 || bbox.height <= 0.0 {
        return;
    }
    let Ok(mut surface) = cairo::ImageSurface::create(
        cairo::Format::ARgb32,
        image.width as i32,
        image.height as i32,
    ) else {
        return;
    };
    {
        let stride = surface.stride() as usize;
        let Ok(mut data) = surface.data() else {
            return;
        };
        for y in 0..image.height as usize {
            let row = y * stride;
            let src = y * image.width as usize * 4;
            for x in 0..image.width as usize {
                let s = src + x * 4;
                let d = row + x * 4;
                let r = image.pixels[s];
                let g = image.pixels[s + 1];
                let b = image.pixels[s + 2];
                let a = image.pixels[s + 3];
                // Premultiply (no-op for opaque); permute RGBA → BGRA.
                let pr = ((r as u16 * a as u16) / 255) as u8;
                let pg = ((g as u16 * a as u16) / 255) as u8;
                let pb = ((b as u16 * a as u16) / 255) as u8;
                data[d] = pb;
                data[d + 1] = pg;
                data[d + 2] = pr;
                data[d + 3] = a;
            }
        }
    }
    surface.mark_dirty();

    let _ = cr.save();
    cr.translate(bbox.x, bbox.y);
    cr.scale(
        bbox.width / image.width as f64,
        bbox.height / image.height as f64,
    );
    let _ = cr.set_source_surface(&surface, 0.0, 0.0);
    let _ = cr.paint();
    let _ = cr.restore();
}

fn paint_ink(cr: &Context, strokes: &[Vec<Point>], color: Color, width: f64) {
    set_color(cr, color);
    cr.set_line_width(width.max(0.5));
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    for s in strokes {
        if s.len() < 2 {
            continue;
        }
        cr.move_to(s[0].x, s[0].y);
        for p in &s[1..] {
            cr.line_to(p.x, p.y);
        }
        let _ = cr.stroke();
    }
}
