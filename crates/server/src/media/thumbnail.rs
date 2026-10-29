use std::io::Cursor;

use image::imageops::FilterType;

use crate::{AppResult, MatrixError};

pub(crate) async fn render(
    image_data: Vec<u8>,
    width: u32,
    height: u32,
    crop: bool,
) -> AppResult<(Option<Vec<u8>>, Vec<u8>)> {
    crate::utils::cpu::run(move || create_thumbnail(image_data, width, height, crop)).await
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
}
