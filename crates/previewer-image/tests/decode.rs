//! Integration tests for `decode_to_rgba`.
//!
//! Fixtures are generated programmatically into a tempdir so the repo stays
//! free of binary blobs for formats the `image` crate can encode itself
//! (PNG, JPEG, WebP). HEIC fixtures are real files (HEIC encoding lives
//! behind the `heic` feature) and are tested in `decode_heic.rs`.

use image::{ColorType, RgbaImage};
use pretty_assertions::assert_eq;
use previewer_image::decode_to_rgba;
use rstest::rstest;
use tempfile::TempDir;

/// Solid red 4x3 RGBA image. Small enough that JPEG quantisation noise
/// across roundtrips is bounded but the asserts still need tolerance.
fn red_image() -> RgbaImage {
    RgbaImage::from_pixel(4, 3, image::Rgba([255, 0, 0, 255]))
}

#[rstest]
#[case::png("png", image::ImageFormat::Png)]
#[case::webp("webp", image::ImageFormat::WebP)]
fn lossless_format_round_trip(#[case] ext: &str, #[case] fmt: image::ImageFormat) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(format!("sample.{ext}"));
    let img = red_image();
    image::save_buffer_with_format(
        &path,
        img.as_raw(),
        img.width(),
        img.height(),
        ColorType::Rgba8,
        fmt,
    )
    .unwrap();

    let decoded = decode_to_rgba(&path).expect("decode failed");

    assert_eq!(decoded.dimensions(), (4, 3));
    // Lossless: pixels exactly match. Top-left pixel is opaque red.
    assert_eq!(&decoded.pixels()[0..4], &[255, 0, 0, 255]);
}

#[test]
fn jpeg_round_trip_dimensions_only() {
    // JPEG is lossy + has no alpha. Assert dimensions and that the
    // overall image is "very red" (red channel >> green/blue).
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("sample.jpg");
    // JPEG encoder via image crate wants RGB8, not RGBA8.
    let rgb = image::RgbImage::from_pixel(4, 3, image::Rgb([255, 0, 0]));
    image::save_buffer_with_format(
        &path,
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        ColorType::Rgb8,
        image::ImageFormat::Jpeg,
    )
    .unwrap();

    let decoded = decode_to_rgba(&path).expect("decode failed");

    assert_eq!(decoded.dimensions(), (4, 3));
    let r = decoded.pixels()[0];
    let g = decoded.pixels()[1];
    let b = decoded.pixels()[2];
    let a = decoded.pixels()[3];
    assert!(r > 200, "expected red-dominant pixel, got r={r}");
    assert!(g < 60, "expected red-dominant pixel, got g={g}");
    assert!(b < 60, "expected red-dominant pixel, got b={b}");
    assert_eq!(a, 255, "JPEG should decode to fully opaque alpha");
}

#[test]
fn nonexistent_path_returns_error() {
    let result = decode_to_rgba("/this/path/does/not/exist.png");
    assert!(result.is_err());
}

mod rotation {
    use image::{ColorType, RgbaImage};
    use pretty_assertions::assert_eq;
    use previewer_image::{DecodedImage, decode_to_rgba};
    use tempfile::TempDir;

    /// Build a 2×3 image with each pixel a unique color we can identify after
    /// rotation. Layout (row-major, top-left = (0,0)):
    ///
    ///   (0,0)R  (1,0)G
    ///   (0,1)B  (1,1)Y
    ///   (0,2)C  (1,2)M
    fn marker_image() -> DecodedImage {
        let mut img = RgbaImage::new(2, 3);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255])); // R
        img.put_pixel(1, 0, image::Rgba([0, 255, 0, 255])); // G
        img.put_pixel(0, 1, image::Rgba([0, 0, 255, 255])); // B
        img.put_pixel(1, 1, image::Rgba([255, 255, 0, 255])); // Y
        img.put_pixel(0, 2, image::Rgba([0, 255, 255, 255])); // C
        img.put_pixel(1, 2, image::Rgba([255, 0, 255, 255])); // M

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("m.png");
        image::save_buffer_with_format(
            &path,
            img.as_raw(),
            2,
            3,
            ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
        decode_to_rgba(&path).unwrap()
    }

    fn pixel_at(img: &DecodedImage, x: u32, y: u32) -> [u8; 4] {
        let (w, _) = img.dimensions();
        let idx = ((y * w + x) * 4) as usize;
        let p = &img.pixels()[idx..idx + 4];
        [p[0], p[1], p[2], p[3]]
    }

    #[test]
    fn rotate_90_cw_swaps_dimensions() {
        let img = marker_image();
        assert_eq!(img.dimensions(), (2, 3));
        let r = img.rotated_90_cw();
        assert_eq!(r.dimensions(), (3, 2));
    }

    #[test]
    fn rotate_90_cw_maps_corners() {
        // 90° CW: top-left → top-right, top-right → bottom-right,
        //         bottom-right → bottom-left, bottom-left → top-left.
        // Original 2×3, new 3×2.
        let img = marker_image();
        let r = img.rotated_90_cw();
        // Original (0,0) R → new (new_w-1, 0) = (2, 0)
        assert_eq!(
            pixel_at(&r, 2, 0),
            [255, 0, 0, 255],
            "R should be top-right"
        );
        // Original (1,0) G → new (2, 1)
        assert_eq!(
            pixel_at(&r, 2, 1),
            [0, 255, 0, 255],
            "G should be bottom-right"
        );
        // Original (0,2) C → new (0, 0)
        assert_eq!(
            pixel_at(&r, 0, 0),
            [0, 255, 255, 255],
            "C should be top-left"
        );
        // Original (1,2) M → new (0, 1)
        assert_eq!(
            pixel_at(&r, 0, 1),
            [255, 0, 255, 255],
            "M should be bottom-left"
        );
    }

    #[test]
    fn four_rotations_is_identity() {
        let img = marker_image();
        let r4 = img
            .rotated_90_cw()
            .rotated_90_cw()
            .rotated_90_cw()
            .rotated_90_cw();
        assert_eq!(img, r4);
    }
}
