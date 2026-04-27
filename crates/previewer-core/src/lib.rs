//! previewer-core: pure annotation model and geometry primitives.
//!
//! No GTK and no PDF dependencies. Everything here is testable with
//! `cargo test -p previewer-core` in milliseconds.

mod annotation;
mod geometry;
mod settings;
mod sidecar;
mod undo;

pub use annotation::{
    Annotation, AnnotationLayer, ArrowEnds, FontSpec, StampImage, Stroke, StrokeStyle,
};
pub use geometry::{Color, Point, Rect};
pub use settings::Settings;
pub use sidecar::{SidecarError, load_layer, save_layer, sidecar_path};
pub use undo::{CoalesceKey, UndoStack};
