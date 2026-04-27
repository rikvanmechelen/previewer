//! previewer-image: decode image files into owned RGBA8 buffers.
//!
//! Routes formats handled by the `image` crate (PNG, JPEG, WebP, ...) through
//! one path; HEIC is feature-gated behind `heic` and routed through
//! `libheif-rs`.

mod decode;

#[cfg(feature = "heic")]
mod heic;

pub use decode::{DecodedImage, Error, decode_to_rgba};
