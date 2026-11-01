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
