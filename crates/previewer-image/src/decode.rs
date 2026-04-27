use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("decode error: {0}")]
    Decode(#[from] image::ImageError),

    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),

    #[cfg(feature = "heic")]
    #[error("HEIC decode error: {0}")]
    Heic(#[from] libheif_rs::HeifError),
}

/// An owned RGBA8 image buffer.
///
/// Invariant: `pixels.len() == width as usize * height as usize * 4`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl DecodedImage {
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Raw RGBA8 pixels, row-major, top-left origin, no row padding.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub(crate) fn from_raw(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        debug_assert_eq!(pixels.len(), width as usize * height as usize * 4);
        Self {
            width,
            height,
            pixels,
        }
    }

    /// Return a new image rotated 90° clockwise. Dimensions swap.
    pub fn rotated_90_cw(&self) -> Self {
        let w = self.width as usize;
        let h = self.height as usize;
        let new_w = h;
        let new_h = w;
        let mut out = vec![0u8; new_w * new_h * 4];
        for y in 0..h {
            for x in 0..w {
                let src = (y * w + x) * 4;
                let nx = h - 1 - y;
                let ny = x;
                let dst = (ny * new_w + nx) * 4;
                out[dst..dst + 4].copy_from_slice(&self.pixels[src..src + 4]);
            }
        }
        Self::from_raw(new_w as u32, new_h as u32, out)
    }
}

/// Decode an image file at `path` into an owned RGBA8 buffer.
///
/// Routes `.heic` / `.heif` to libheif (when the `heic` feature is on),
/// everything else to the `image` crate.
pub fn decode_to_rgba(path: impl AsRef<Path>) -> Result<DecodedImage, Error> {
    let path = path.as_ref();

    if is_heic_extension(path) {
        #[cfg(feature = "heic")]
        return crate::heic::decode_heic(path);
        #[cfg(not(feature = "heic"))]
        return Err(Error::UnsupportedFormat(
            "HEIC support not compiled in (rebuild with --features heic)".into(),
        ));
    }

    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let dynamic = reader.decode()?;
    let rgba = dynamic.into_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(DecodedImage::from_raw(width, height, rgba.into_raw()))
}

fn is_heic_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|s| {
            let s = s.to_ascii_lowercase();
            s == "heic" || s == "heif"
        })
        .unwrap_or(false)
}
