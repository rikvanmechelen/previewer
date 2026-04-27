//! PNG-import tests for raster signatures.

use image::{ColorType, ImageFormat, RgbImage, RgbaImage};
use pretty_assertions::assert_eq;
use previewer_signature::{ImportError, ImportOptions, SignatureKind, import_png_signature};
use tempfile::TempDir;

fn write_png(path: &std::path::Path, img: &RgbaImage) {
    image::save_buffer_with_format(
        path,
        img.as_raw(),
        img.width(),
        img.height(),
        ColorType::Rgba8,
        ImageFormat::Png,
    )
    .unwrap();
}

fn write_rgb_png(path: &std::path::Path, img: &RgbImage) {
    image::save_buffer_with_format(
        path,
        img.as_raw(),
        img.width(),
        img.height(),
        ColorType::Rgb8,
        ImageFormat::Png,
    )
    .unwrap();
}

#[test]
fn opaque_rgb_png_is_rejected() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("opaque.png");
    write_rgb_png(&path, &RgbImage::from_pixel(20, 20, image::Rgb([0, 0, 0])));

    let result = import_png_signature(&path, ImportOptions::default());
    assert!(matches!(result, Err(ImportError::NoTransparency)));
}

#[test]
fn fully_opaque_rgba_png_is_rejected() {
    // RGBA but every pixel has alpha=255 — same problem (no transparency to
    // trim around).
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("opaque_rgba.png");
    write_png(
        &path,
        &RgbaImage::from_pixel(20, 20, image::Rgba([0, 0, 0, 255])),
    );

    let result = import_png_signature(&path, ImportOptions::default());
    assert!(matches!(result, Err(ImportError::NoTransparency)));
}

#[test]
fn transparent_borders_get_trimmed() {
    // 50×30 image, but only the rect (10..40, 5..25) is opaque ink.
    let mut img = RgbaImage::from_pixel(50, 30, image::Rgba([0, 0, 0, 0]));
    for y in 5..25 {
        for x in 10..40 {
            img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
        }
    }
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("inked.png");
    write_png(&path, &img);

    let sig = import_png_signature(&path, ImportOptions::default()).expect("import");

    let SignatureKind::Raster { width, height, .. } = &sig.kind else {
        panic!("expected raster signature");
    };
    assert_eq!((*width, *height), (30, 20), "should crop to ink bbox 30×20");
}

#[test]
fn auto_trim_disabled_keeps_original_size() {
    let mut img = RgbaImage::from_pixel(50, 30, image::Rgba([0, 0, 0, 0]));
    img.put_pixel(25, 15, image::Rgba([0, 0, 0, 255]));
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("dot.png");
    write_png(&path, &img);

    let sig = import_png_signature(
        &path,
        ImportOptions {
            auto_trim: false,
            ..ImportOptions::default()
        },
    )
    .expect("import");

    let SignatureKind::Raster { width, height, .. } = &sig.kind else {
        panic!("expected raster signature");
    };
    assert_eq!((*width, *height), (50, 30));
}

#[test]
fn signature_name_defaults_to_filename_stem() {
    let mut img = RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 0, 0]));
    img.put_pixel(5, 5, image::Rgba([0, 0, 0, 255]));
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("rik-sig.png");
    write_png(&path, &img);

    let sig = import_png_signature(&path, ImportOptions::default()).expect("import");
    assert_eq!(sig.name, "rik-sig");
}
