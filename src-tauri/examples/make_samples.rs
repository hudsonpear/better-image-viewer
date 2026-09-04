// Generates test images for the formats that have a Rust encoder available.
// Usage: cargo run --example make_samples -- <output dir>
//
// The rest (JXL, PSD, KTX2, RAW) are decode-only here and have to come from
// real files.

use image::{ImageFormat, Rgb, Rgb32FImage, RgbImage, Rgba, RgbaImage};
use std::fs::File;
use std::path::PathBuf;

fn test_pattern(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        let checker = ((x / 16) + (y / 16)) % 2 == 0;
        let shade = (x * 255 / width.max(1)) as u8;

        if checker {
            Rgba([shade, 60, 255 - shade, 255])
        } else {
            Rgba([255 - shade, 200, shade, 255])
        }
    })
}

fn main() {
    let dir: PathBuf = std::env::args()
        .nth(1)
        .expect("pass an output directory")
        .into();
    std::fs::create_dir_all(&dir).expect("create output dir");

    let pattern = test_pattern(256, 192);

    // --- DDS, with mipmaps and as a cubemap-less plain texture ---
    let dds = image_dds::dds_from_image(
        &pattern,
        image_dds::ImageFormat::BC1RgbaUnorm,
        image_dds::Quality::Slow,
        image_dds::Mipmaps::GeneratedAutomatic,
    )
    .expect("encode dds");

    let mut file = File::create(dir.join("sample_bc1_mips.dds")).expect("create dds");
    dds.write(&mut file).expect("write dds");
    println!("wrote sample_bc1_mips.dds (BC1, mipmaps)");

    let dds7 = image_dds::dds_from_image(
        &pattern,
        image_dds::ImageFormat::BC7RgbaUnorm,
        image_dds::Quality::Fast,
        image_dds::Mipmaps::Disabled,
    )
    .expect("encode bc7 dds");

    let mut file = File::create(dir.join("sample_bc7.dds")).expect("create dds");
    dds7.write(&mut file).expect("write dds");
    println!("wrote sample_bc7.dds (BC7, single level)");

    // --- Radiance HDR: values above 1.0 so tone mapping has something to do ---
    let hdr = Rgb32FImage::from_fn(256, 192, |x, y| {
        let bright = 4.0 * (x as f32 / 255.0);
        Rgb([bright, y as f32 / 191.0, 1.0 - bright.min(1.0)])
    });
    hdr.save_with_format(dir.join("sample.hdr"), ImageFormat::Hdr)
        .expect("write hdr");
    println!("wrote sample.hdr (Radiance, values > 1.0)");

    // --- OpenEXR ---
    hdr.save_with_format(dir.join("sample.exr"), ImageFormat::OpenExr)
        .expect("write exr");
    println!("wrote sample.exr (32-bit float)");

    // --- APNG: four frames, so animation is visible ---
    let path = dir.join("sample_animated.apng");
    let file = File::create(&path).expect("create apng");
    let mut encoder = png::Encoder::new(file, 128, 128);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_animated(4, 0).expect("set animated");
    encoder
        .set_frame_delay(1, 4)
        .expect("set delay"); // 250ms per frame
    let mut writer = encoder.write_header().expect("apng header");

    for frame in 0..4u32 {
        let img = RgbImage::from_fn(128, 128, |x, y| {
            // a square that moves across the frames
            let left = frame * 24;
            if x >= left && x < left + 32 && y >= 48 && y < 80 {
                Rgb([255, 80, 0])
            } else {
                Rgb([20, 20, 40])
            }
        });

        let rgba: Vec<u8> = img
            .pixels()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect();
        writer.write_image_data(&rgba).expect("write apng frame");
    }
    writer.finish().expect("finish apng");
    println!("wrote sample_animated.apng (4 frames)");

    println!("\ndone: {}", dir.display());
}
