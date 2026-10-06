use std::io::BufReader;

use image::{DynamicImage, ImageDecoder, ImageReader};
use onscripter_core::{
    Error, Limits, Result,
    assets::{Asset, AssetRange},
    scene::{ImageKey, Transparency},
};

pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Decode directly from the selected loose file or bounded archive stream.
pub fn decode(asset: Asset, key: &ImageKey, limits: Limits) -> Result<DecodedImage> {
    if asset.length > limits.asset_bytes as u64 {
        return Err(Error::Limit("encoded image size"));
    }
    let reader = AssetRange::new(asset.reader, 0, asset.length)?;
    let mut reader = ImageReader::new(BufReader::new(reader)).with_guessed_format()?;
    let mut decoder_limits = image::Limits::default();
    decoder_limits.max_image_width = Some(limits.image_dimension);
    decoder_limits.max_image_height = Some(limits.image_dimension);
    decoder_limits.max_alloc = Some(limits.image_bytes as u64);
    reader.limits(decoder_limits);
    let decoder = reader.into_decoder().map_err(image_error)?;
    let (width, height) = decoder.dimensions();
    pixel_bytes(width, height, limits)?;
    if decoder.total_bytes() > limits.image_bytes as u64 {
        return Err(Error::Limit("decoded image size"));
    }
    let has_alpha = decoder.color_type().has_alpha();
    let mut pixels = DynamicImage::from_decoder(decoder)
        .map_err(image_error)?
        .into_rgba8()
        .into_raw();
    let mut width = width;
    let dimension = if key.vertical { height } else { width };
    if key.cells == 0 || key.cells > dimension || dimension % key.cells != 0 {
        return Err(Error::invalid("image dimensions do not fit its cells"));
    }
    match key.transparency {
        Transparency::Copy => pixels
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|pixel| pixel[3] = 255),
        Transparency::Alpha if !has_alpha => {
            // Legacy RGB sheets store a grayscale mask beside each color cell.
            let columns = if key.vertical { 1 } else { key.cells };
            let cell_width = width / columns;
            if cell_width < 2 || cell_width % 2 != 0 {
                return Err(Error::invalid(
                    "split-alpha image cells must have even widths",
                ));
            }
            let color_width = cell_width / 2;
            let output_width = width / 2;
            for y in 0..height as usize {
                for cell in 0..columns as usize {
                    for x in 0..color_width as usize {
                        let source = (y * width as usize + cell * cell_width as usize + x) * 4;
                        let mask = source + color_width as usize * 4;
                        let alpha = 255 - pixels[mask + 2];
                        let color = [
                            pixels[source],
                            pixels[source + 1],
                            pixels[source + 2],
                            alpha,
                        ];
                        let destination =
                            (y * output_width as usize + cell * color_width as usize + x) * 4;
                        pixels[destination..destination + 4].copy_from_slice(&color);
                    }
                }
            }
            pixels.truncate(output_width as usize * height as usize * 4);
            width = output_width;
        }
        Transparency::Alpha => {}
        mode => {
            let color = match mode {
                Transparency::TopLeft => [pixels[0], pixels[1], pixels[2]],
                Transparency::TopRight => {
                    let offset = (width as usize - 1) * 4;
                    [pixels[offset], pixels[offset + 1], pixels[offset + 2]]
                }
                Transparency::Color(color) => color,
                _ => return Err(Error::invalid("invalid image transparency")),
            };
            for pixel in pixels.as_chunks_mut::<4>().0 {
                pixel[3] = if pixel[..3] == color { 0 } else { 255 };
            }
        }
    }
    Ok(DecodedImage {
        width,
        height,
        pixels,
    })
}

pub(crate) fn pixel_bytes(width: u32, height: u32, limits: Limits) -> Result<usize> {
    if width == 0
        || height == 0
        || width > limits.image_dimension
        || height > limits.image_dimension
    {
        return Err(Error::Limit("image dimensions"));
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|size| size.checked_mul(4))
        .filter(|&size| size <= limits.image_bytes)
        .ok_or(Error::Limit("decoded image size"))
}

fn image_error(error: image::ImageError) -> Error {
    Error::invalid(format!("image: {error}"))
}
