use std::sync::Arc;

use image::codecs::png::PngEncoder;
use image::{ImageEncoder, RgbaImage, imageops};

use crate::canvas::Orientation;

pub fn encode_png(source: (u32, u32, Arc<Vec<u8>>), orientation: Orientation) -> Option<Vec<u8>> {
    let (width, height, rgba) = source;
    let mut image = RgbaImage::from_raw(width, height, rgba.to_vec())?;
    image = match orientation.turns {
        1 => imageops::rotate90(&image),
        2 => imageops::rotate180(&image),
        3 => imageops::rotate270(&image),
        _ => image,
    };
    if orientation.flipped {
        imageops::flip_horizontal_in_place(&mut image);
    }
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .inspect_err(|error| tracing::warn!("cannot encode the copy: {error}"))
        .ok()?;
    Some(png)
}
