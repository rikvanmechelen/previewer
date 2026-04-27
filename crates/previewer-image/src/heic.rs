//! HEIC decoder via libheif-rs. Compiled only when the `heic` feature is on.

use std::path::Path;

use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};

use crate::decode::{DecodedImage, Error};

pub(crate) fn decode_heic(path: &Path) -> Result<DecodedImage, Error> {
    let path_str = path
        .to_str()
        .ok_or_else(|| Error::UnsupportedFormat(format!("non-utf8 path: {}", path.display())))?;

    let ctx = HeifContext::read_from_file(path_str)?;
    let handle = ctx.primary_image_handle()?;
    let lib = LibHeif::new();
    let img = lib.decode(&handle, ColorSpace::Rgb(RgbChroma::Rgba), None)?;
    let planes = img.planes();
    let plane = planes
        .interleaved
        .ok_or_else(|| Error::UnsupportedFormat("HEIC: no interleaved RGBA plane".into()))?;

    let width = plane.width;
    let height = plane.height;
    let stride = plane.stride;
    let row_bytes = width as usize * 4;

    // Tightly-pack pixels: libheif may pad rows for SIMD alignment.
    let mut pixels = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height as usize {
        let start = row * stride;
        pixels.extend_from_slice(&plane.data[start..start + row_bytes]);
    }

    Ok(DecodedImage::from_raw(width, height, pixels))
}
