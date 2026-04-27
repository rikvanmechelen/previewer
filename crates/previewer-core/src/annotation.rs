use serde::{Deserialize, Serialize};

use crate::geometry::{Color, Point, Rect};

/// How the outline is drawn — solid line, dashes, or dots. Maps to
/// PDF's `/Border` dash array on save and to Cairo `set_dash` on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StrokeStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub color: Color,
    pub width: f64,
    /// Line style. Defaulted on deserialise so legacy sidecars (no
    /// `style` field) load as `Solid`.
    #[serde(default)]
    pub style: StrokeStyle,
}

impl Stroke {
    pub const fn new(color: Color, width: f64) -> Self {
        Self {
            color,
            width,
            style: StrokeStyle::Solid,
        }
    }

    pub const fn with_style(color: Color, width: f64, style: StrokeStyle) -> Self {
        Self {
            color,
            width,
            style,
        }
    }
}

/// Which endpoints of an `Annotation::Arrow` carry an arrowhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArrowEnds {
    /// Plain line, no heads — the "Line" tool.
    None,
    /// Head at `to` only — the classic "Arrow" tool.
    #[default]
    End,
    /// Heads at both endpoints — the "DoubleArrow" tool.
    Both,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontSpec {
    pub family: String,
    pub size: f64,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            family: "Helvetica".into(),
            size: 14.0,
        }
    }
}

/// A raster image embedded in an annotation (e.g. an imported signature
/// stamped onto a PDF). RGBA8, row-major, top-left origin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StampImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// A single annotation, in image-space coordinates (top-left origin, pixels).
///
/// `page` is `0` for image annotations; PDFs use it from M4 onward.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Annotation {
    Rect {
        page: u32,
        bbox: Rect,
        stroke: Stroke,
        fill: Option<Color>,
    },
    Ellipse {
        page: u32,
        bbox: Rect,
        stroke: Stroke,
        fill: Option<Color>,
    },
    Arrow {
        page: u32,
        from: Point,
        to: Point,
        stroke: Stroke,
        /// Which endpoints draw an arrowhead. Defaulted on deserialise so
        /// older sidecar JSON files (which only knew the single-headed
        /// flavour) load as `End` rather than failing.
        #[serde(default)]
        ends: ArrowEnds,
    },
    FreeText {
        page: u32,
        position: Point,
        text: String,
        font: FontSpec,
        color: Color,
        /// True for the auto-placed "Enter some text" prompt that the Text
        /// tool drops onto the page; flipped to false the moment the user
        /// types real content. Drives the dim render style and the
        /// "clear on first edit / restore on empty commit" UX.
        ///
        /// Defaulted on deserialise so legacy sidecars (which predate the
        /// placeholder flow) load with `false`.
        #[serde(default)]
        is_placeholder: bool,
    },
    Highlight {
        page: u32,
        bbox: Rect,
        color: Color,
    },
    /// Pasted raster signature (or any image). `bbox` is the placement on
    /// the page in image-coord units; `image` is the source pixels.
    Stamp {
        page: u32,
        bbox: Rect,
        image: StampImage,
    },
    /// Vector ink — one or more polylines, mapped to the PDF `/Ink` subtype
    /// when written. Each inner `Vec<Point>` is one continuous stroke.
    Ink {
        page: u32,
        strokes: Vec<Vec<Point>>,
        color: Color,
        width: f64,
    },
}

impl Annotation {
    pub fn page(&self) -> u32 {
        match self {
            Self::Rect { page, .. }
            | Self::Ellipse { page, .. }
            | Self::Arrow { page, .. }
            | Self::FreeText { page, .. }
            | Self::Highlight { page, .. }
            | Self::Stamp { page, .. }
            | Self::Ink { page, .. } => *page,
        }
    }
}

/// An ordered collection of annotations. Drawing order = vector index order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnnotationLayer {
    pub items: Vec<Annotation>,
}

impl AnnotationLayer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, ann: Annotation) {
        self.items.push(ann);
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}
