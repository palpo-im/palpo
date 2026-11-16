use std::io::Cursor;

use image::imageops::FilterType;

use crate::{AppResult, MatrixError};

pub(crate) async fn render_stored(
    key: &str,
    width: u32,
    height: u32,
    crop: bool,
) -> AppResult<(Option<Vec<u8>>, Vec<u8>)> {
    crate::utils::cpu::pipeline(crate::storage::read(key), move |image_data| {
        create_thumbnail(image_data, width, height, crop)
    })
    .await
}

/// Decode only inside the bounded CPU pool. Limits apply to the original image,
/// independently of the much smaller requested thumbnail dimensions.
fn create_thumbnail(
    image_data: Vec<u8>,
    width: u32,
    height: u32,
    crop: bool,
) -> AppResult<(Option<Vec<u8>>, Vec<u8>)> {
    if width == 0 || height == 0 {
        return Ok((None, image_data));
    }
    let mut reader = image::ImageReader::new(Cursor::new(&image_data)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = match reader.decode() {
        Ok(image) => image,
        Err(image::ImageError::Limits(_)) => {
            return Err(
                MatrixError::invalid_param("Image exceeds thumbnail decoding limits.").into(),
            );
        }
        Err(_) => return Ok((None, image_data)),
    };
    let original_width = image.width();
    let original_height = image.height();
    if width > original_width || height > original_height {
        return Ok((None, image_data));
    }
    // PNG cannot encode floating-point EXR/HDR pixels. Convert only those
    // formats before sampling: image's integer thumbnail algorithm adds a
    // rounding offset that otherwise biases float colors and transparency.
    // Retain the existing 8/16-bit paths and let image normalize float channels.
    let image = match image.color() {
        image::ColorType::Rgb32F => image::DynamicImage::ImageRgb8(image.into_rgb8()),
        image::ColorType::Rgba32F => image::DynamicImage::ImageRgba8(image.into_rgba8()),
        _ => image,
    };
    let thumbnail = if crop {
        image.resize_to_fill(width, height, FilterType::CatmullRom)
    } else {
        let (exact_width, exact_height) = {
            let ratio = u64::from(original_width) * u64::from(height);
            let nratio = u64::from(width) * u64::from(original_height);

            let use_width = nratio <= ratio;
            let intermediate = if use_width {
                u64::from(original_height) * u64::from(width) / u64::from(original_width)
            } else {
                u64::from(original_width) * u64::from(height) / u64::from(original_height)
            };
            if use_width {
                if intermediate <= u64::from(u32::MAX) {
                    (width, intermediate as u32)
                } else {
                    (
                        (u64::from(width) * u64::from(u32::MAX) / intermediate) as u32,
                        u32::MAX,
                    )
                }
            } else if intermediate <= u64::from(u32::MAX) {
                (intermediate as u32, height)
            } else {
                (
                    u32::MAX,
                    (u64::from(height) * u64::from(u32::MAX) / intermediate) as u32,
                )
            }
        };

        image.thumbnail_exact(exact_width, exact_height)
    };

    let mut thumbnail_bytes = Vec::new();
    thumbnail.write_to(
        &mut Cursor::new(&mut thumbnail_bytes),
        image::ImageFormat::Png,
    )?;
    Ok((Some(thumbnail_bytes), Vec::new()))
}

#[cfg(test)]
mod thumbnail_cpu_tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::DynamicImage::new_rgba8(width, height)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn thumbnail_decoding_limits_apply_before_small_output_resize() {
        assert!(create_thumbnail(png(16_385, 1), 32, 1, false).is_err());
        let (generated, original) = create_thumbnail(png(200, 100), 32, 32, true).unwrap();
        assert!(original.is_empty());
        let image = image::load_from_memory(&generated.unwrap()).unwrap();
        assert_eq!((image.width(), image.height()), (32, 32));
    }

    #[test]
    fn non_images_and_original_size_requests_keep_the_original_bytes() {
        let bytes = b"not an image".to_vec();
        assert_eq!(
            create_thumbnail(bytes.clone(), 32, 32, false).unwrap(),
            (None, bytes)
        );
        let bytes = png(10, 10);
        assert_eq!(
            create_thumbnail(bytes.clone(), 32, 32, true).unwrap(),
            (None, bytes)
        );
    }

    #[test]
    fn exr_decoding_and_png_thumbnail_remain_available() {
        let pixels = image::ImageBuffer::from_pixel(17, 9, image::Rgb([0.5f32, 0.25, 0.125]));
        let mut source = Vec::new();
        image::DynamicImage::ImageRgb32F(pixels)
            .write_to(&mut Cursor::new(&mut source), image::ImageFormat::OpenExr)
            .unwrap();
        let (generated, original) = create_thumbnail(source, 8, 4, false).unwrap();
        assert!(original.is_empty());
        let thumbnail = image::load_from_memory(&generated.unwrap())
            .unwrap()
            .to_rgb8();
        assert_eq!(thumbnail.dimensions(), (7, 4));
        for pixel in thumbnail.pixels() {
            for (actual, expected) in pixel.0.into_iter().zip([128u8, 64, 32]) {
                assert!(actual.abs_diff(expected) <= 1);
            }
        }
    }

    #[test]
    fn default_avif_encoding_capability_remains_available() {
        let pixels = image::ImageBuffer::from_pixel(17, 19, image::Rgb([40u8, 80, 120]));
        let mut encoded = Vec::new();
        image::DynamicImage::ImageRgb8(pixels)
            .write_to(&mut Cursor::new(&mut encoded), image::ImageFormat::Avif)
            .unwrap();
        assert!(encoded.len() > 32);
        assert_eq!(&encoded[4..8], b"ftyp");
        assert_eq!(&encoded[8..12], b"avif");
        assert!(encoded.windows(4).any(|part| part == b"mdat"));
    }

    #[test]
    fn rgba_exr_thumbnails_preserve_alpha_and_normalize_finite_channel_bounds() {
        for (channels, expected) in [
            ([0.5f32, 0.25, 0.125, 0.5], [128u8, 64, 32, 128]),
            ([-0.25, 2.0, f32::MAX, 1.0], [0, 255, 255, 255]),
            ([0.0, 0.0, 0.0, 0.0], [0, 0, 0, 0]),
            ([0.5, 0.25, 0.125, 1.5], [128, 64, 32, 255]),
        ] {
            let pixels = image::ImageBuffer::from_pixel(17, 9, image::Rgba(channels));
            let mut source = Vec::new();
            image::DynamicImage::ImageRgba32F(pixels)
                .write_to(&mut Cursor::new(&mut source), image::ImageFormat::OpenExr)
                .unwrap();
            for crop in [false, true] {
                let (generated, original) = create_thumbnail(source.clone(), 8, 4, crop).unwrap();
                assert!(original.is_empty());
                let thumbnail = image::load_from_memory(&generated.unwrap()).unwrap();
                assert_eq!(thumbnail.color(), image::ColorType::Rgba8);
                assert_eq!(
                    (thumbnail.width(), thumbnail.height()),
                    if crop { (8, 4) } else { (7, 4) }
                );
                for pixel in thumbnail.to_rgba8().pixels() {
                    for (actual, expected) in pixel.0.into_iter().zip(expected) {
                        assert!(actual.abs_diff(expected) <= 1);
                    }
                }
            }
        }
    }

    #[test]
    fn nonfinite_exr_channels_produce_png_without_panicking() {
        for channel in [f32::NAN, f32::NEG_INFINITY, f32::INFINITY] {
            let pixels = image::ImageBuffer::from_pixel(17, 9, image::Rgba([channel; 4]));
            let mut source = Vec::new();
            image::DynamicImage::ImageRgba32F(pixels)
                .write_to(&mut Cursor::new(&mut source), image::ImageFormat::OpenExr)
                .unwrap();
            let (generated, _) = create_thumbnail(source, 8, 4, false).unwrap();
            let thumbnail = image::load_from_memory(&generated.unwrap()).unwrap();
            assert_eq!(thumbnail.color(), image::ColorType::Rgba8);
            assert_eq!((thumbnail.width(), thumbnail.height()), (7, 4));
        }
    }

    #[test]
    fn integer_png_thumbnails_keep_sixteen_bit_color_and_alpha() {
        for (source, expected) in [
            (
                image::DynamicImage::ImageRgb16(image::ImageBuffer::from_pixel(
                    17,
                    9,
                    image::Rgb([40_001u16, 30_003, 20_005]),
                )),
                [40_001u16, 30_003, 20_005, u16::MAX],
            ),
            (
                image::DynamicImage::ImageRgba16(image::ImageBuffer::from_pixel(
                    17,
                    9,
                    image::Rgba([40_001u16, 30_003, 20_005, 12_345]),
                )),
                [40_001u16, 30_003, 20_005, 12_345],
            ),
        ] {
            let color = source.color();
            let mut encoded = Vec::new();
            source
                .write_to(&mut Cursor::new(&mut encoded), image::ImageFormat::Png)
                .unwrap();
            let (generated, _) = create_thumbnail(encoded, 8, 4, false).unwrap();
            let thumbnail = image::load_from_memory(&generated.unwrap()).unwrap();
            assert_eq!(thumbnail.color(), color);
            for pixel in thumbnail.to_rgba16().pixels() {
                for (actual, expected) in pixel.0.into_iter().zip(expected) {
                    assert!(actual.abs_diff(expected) <= 1);
                }
            }
        }
    }
}
