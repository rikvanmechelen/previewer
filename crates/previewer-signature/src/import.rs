use std::path::Path;

use crate::{Signature, SignatureId, SignatureKind};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("decode error: {0}")]
    Decode(#[from] image::ImageError),

    #[error("PNG has no transparent pixels — pick a signature on a transparent background")]
    NoTransparency,
}

#[derive(Debug, Clone, Copy)]
pub struct ImportOptions {
    /// Crop to the bounding box of non-transparent pixels (default `true`).
    pub auto_trim: bool,
    /// A pixel counts as transparent when `alpha <= alpha_threshold`. Default `0`.
    pub alpha_threshold: u8,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            auto_trim: true,
            alpha_threshold: 0,
        }
    }
}

/// Import a PNG with an alpha channel as a raster signature.
pub fn import_png_signature(
    path: impl AsRef<Path>,
    opts: ImportOptions,
) -> Result<Signature, ImportError> {
    let path = path.as_ref();
    let img = image::ImageReader::open(path)?
        .with_guessed_format()?
        .decode()?;
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();

    // Reject if there's no transparency at all — caller almost certainly
    // didn't mean to import a flat-background image as a "signature".
    let any_transparent = rgba.pixels().any(|p| p.0[3] <= opts.alpha_threshold);
    if !any_transparent {
        return Err(ImportError::NoTransparency);
    }

    let (out_w, out_h, pixels) = if opts.auto_trim {
        crop_to_alpha_bbox(rgba, opts.alpha_threshold)
    } else {
        (w, h, rgba.into_raw())
    };

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Signature")
        .to_string();

    Ok(Signature {
        id: SignatureId::random(),
        name,
        kind: SignatureKind::Raster {
            width: out_w,
            height: out_h,
            pixels,
        },
    })
}

fn crop_to_alpha_bbox(img: image::RgbaImage, alpha_threshold: u8) -> (u32, u32, Vec<u8>) {
    let (w, h) = img.dimensions();
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0_u32;
    let mut max_y = 0_u32;
    let mut found = false;
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[3] > alpha_threshold {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
            found = true;
        }
    }
    if !found {
        // Entire image is transparent — return a 1×1 empty pixel.
        return (1, 1, vec![0, 0, 0, 0]);
    }
    let new_w = max_x - min_x + 1;
    let new_h = max_y - min_y + 1;
    let mut out = Vec::with_capacity((new_w * new_h * 4) as usize);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let p = img.get_pixel(x, y);
            out.extend_from_slice(&p.0);
        }
    }
    (new_w, new_h, out)
}
