//! HEIC decode test — only compiled when `--features heic` is enabled.
//!
//! The fixture is generated at runtime via libheif's HEVC encoder so the repo
//! stays free of binary blobs.

#![cfg(feature = "heic")]

use libheif_rs::{
    Channel, ColorSpace, CompressionFormat, EncoderQuality, HeifContext, Image, LibHeif, RgbChroma,
};
use pretty_assertions::assert_eq;
use previewer_image::decode_to_rgba;
use tempfile::TempDir;

const W: u32 = 64;
const H: u32 = 48;

fn write_red_heic(path: &std::path::Path) {
    let lib = LibHeif::new();

    let mut img = Image::new(W, H, ColorSpace::Rgb(RgbChroma::Rgba)).unwrap();
    img.create_plane(Channel::Interleaved, W, H, 8).unwrap();
    {
        let mut planes = img.planes_mut();
        let plane = planes.interleaved.as_mut().unwrap();
        for y in 0..H as usize {
            for x in 0..W as usize {
                let dst = y * plane.stride + x * 4;
                plane.data[dst] = 255; // R
                plane.data[dst + 1] = 0; // G
                plane.data[dst + 2] = 0; // B
                plane.data[dst + 3] = 255; // A
            }
        }
    }

    let mut ctx = HeifContext::new().unwrap();
    let mut encoder = lib.encoder_for_format(CompressionFormat::Hevc).unwrap();
    encoder.set_quality(EncoderQuality::LossLess).unwrap();
    ctx.encode_image(&img, &mut encoder, None).unwrap();
    ctx.write_to_file(path.to_str().unwrap()).unwrap();
}

#[test]
fn heic_round_trip_dimensions() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("sample.heic");
    write_red_heic(&path);

    let decoded = decode_to_rgba(&path).expect("HEIC decode failed");

    assert_eq!(decoded.dimensions(), (W, H));
    // Lossless HEVC should preserve solid red (give or take chroma sub-sampling
    // edge cases — we keep tolerance loose).
    let r = decoded.pixels()[0];
    let g = decoded.pixels()[1];
    let b = decoded.pixels()[2];
    let a = decoded.pixels()[3];
    assert!(r > 200, "expected red-dominant, got r={r}");
    assert!(g < 60, "expected red-dominant, got g={g}");
    assert!(b < 60, "expected red-dominant, got b={b}");
    assert_eq!(a, 255);
}
