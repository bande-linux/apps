use image::imageops::FilterType;

pub const MAX_WIDTH: u32 = 1600;
pub const MAX_HEIGHT: u32 = 4800;
const MAX_PIXELS: u64 = 40_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn decode(bytes: &[u8]) -> Option<Picture> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let (width, height) = reader.into_dimensions().ok()?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return None;
    }
    let mut decoded = image::load_from_memory(bytes).ok()?;
    if decoded.width() > MAX_WIDTH || decoded.height() > MAX_HEIGHT {
        decoded = decoded.resize(MAX_WIDTH, MAX_HEIGHT, FilterType::Triangle);
    }
    let rgba = decoded.to_rgba8();
    Some(Picture {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        let encoded = image::DynamicImage::ImageRgba8(image).write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        );
        assert!(encoded.is_ok());
        bytes
    }

    #[test]
    fn pictures_decode_and_wide_ones_shrink_to_fit() {
        let small = decode(&png(4, 2));
        assert_eq!(
            small
                .as_ref()
                .map(|picture| (picture.width, picture.height)),
            Some((4, 2))
        );
        assert_eq!(
            small.map(|picture| picture.rgba[..4].to_vec()),
            Some(vec![10, 20, 30, 255])
        );
        let wide = decode(&png(3200, 100));
        assert_eq!(
            wide.map(|picture| (picture.width, picture.height)),
            Some((MAX_WIDTH, 50))
        );
        let tall = decode(&png(100, 9600));
        assert_eq!(
            tall.map(|picture| (picture.width, picture.height)),
            Some((50, MAX_HEIGHT))
        );
    }

    #[test]
    fn junk_and_bombs_are_refused() {
        assert_eq!(decode(b"not an image"), None);
        let mut header = png(1, 1);
        header[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        header[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        assert_eq!(decode(&header), None);
    }
}
