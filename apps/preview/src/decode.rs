use std::fs::OpenOptions;
use std::io::{Cursor, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::metadata::Orientation;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

use crate::facts::{Facts, camera_facts};

pub const MAX_FILE_BYTES: u64 = 512 << 20;
pub const MAX_PIXELS: u64 = 100_000_000;
const DECODER_BYTES: u64 = 1 << 30;
const ANIMATION_BYTES: u64 = 192 << 20;
const MAX_FRAMES: usize = 4096;
const SMALLEST_LEVEL: u32 = 256;
const SVG_SIDE: f32 = 2048.0;
const SVG_MAX_SIDE: f32 = 8192.0;
const MIN_DELAY: Duration = Duration::from_millis(20);
const DEFAULT_DELAY: Duration = Duration::from_millis(100);

pub struct Level {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

pub struct Frame {
    pub rgba: Arc<Vec<u8>>,
    pub delay: Duration,
}

pub enum Pixels {
    Still(Vec<Level>),
    Animation(Vec<Frame>),
}

pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
    pub facts: Facts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Unreadable,
    NotSupported,
    Damaged,
    TooLarge { width: u32, height: u32 },
    FileTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Png,
    Jpeg,
    Gif,
    Webp,
    Svg,
    Pdf,
}

impl Kind {
    pub fn of(bytes: &[u8], path: &Path) -> Option<Self> {
        match image::guess_format(bytes) {
            Ok(ImageFormat::Png) => return Some(Self::Png),
            Ok(ImageFormat::Jpeg) => return Some(Self::Jpeg),
            Ok(ImageFormat::Gif) => return Some(Self::Gif),
            Ok(ImageFormat::WebP) => return Some(Self::Webp),
            _ => {}
        }
        let head = &bytes[..bytes.len().min(1024)];
        if head.windows(5).any(|window| window == b"%PDF-") {
            return Some(Self::Pdf);
        }
        let svg_name = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
        let svg_text = head.windows(4).any(|window| window == b"<svg");
        (svg_name || svg_text).then_some(Self::Svg)
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Svg => "image/svg+xml",
            Self::Pdf => "application/pdf",
        }
    }
}

pub fn read_file(path: &Path) -> Result<Vec<u8>, Failure> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
        .open(path)
        .map_err(|_| Failure::Unreadable)?;
    let metadata = file.metadata().map_err(|_| Failure::Unreadable)?;
    if !metadata.is_file() {
        return Err(Failure::Unreadable);
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(Failure::FileTooLarge);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Unreadable)?;
    match bytes.len() as u64 > MAX_FILE_BYTES {
        true => Err(Failure::FileTooLarge),
        false => Ok(bytes),
    }
}

fn allowed(width: u32, height: u32) -> Result<(), Failure> {
    match width == 0 || height == 0 {
        true => Err(Failure::Damaged),
        false if u64::from(width) * u64::from(height) > MAX_PIXELS => {
            Err(Failure::TooLarge { width, height })
        }
        false => Ok(()),
    }
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_alloc = Some(DECODER_BYTES);
    limits
}

pub fn decode(bytes: &[u8], kind: Kind) -> Result<Decoded, Failure> {
    match kind {
        Kind::Svg => decode_svg(bytes),
        Kind::Gif => decode_gif(bytes),
        Kind::Webp => decode_webp(bytes),
        Kind::Png | Kind::Jpeg => decode_still(bytes, kind),
        Kind::Pdf => Err(Failure::NotSupported),
    }
}

fn decode_still(bytes: &[u8], kind: Kind) -> Result<Decoded, Failure> {
    let format = match kind {
        Kind::Png => ImageFormat::Png,
        _ => ImageFormat::Jpeg,
    };
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits());
    let mut decoder = reader.into_decoder().map_err(|_| Failure::Damaged)?;
    let (width, height) = decoder.dimensions();
    allowed(width, height)?;
    let exif = decoder.exif_metadata().ok().flatten();
    let orientation = exif
        .as_deref()
        .and_then(Orientation::from_exif_chunk)
        .unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(|error| match error {
        image::ImageError::Limits(_) => Failure::TooLarge { width, height },
        _ => Failure::Damaged,
    })?;
    image.apply_orientation(orientation);
    let facts = camera_facts(exif);
    Ok(still(image.into_rgba8(), facts))
}

fn still(rgba: image::RgbaImage, facts: Facts) -> Decoded {
    let (width, height) = rgba.dimensions();
    Decoded {
        width,
        height,
        pixels: Pixels::Still(pyramid(width, height, rgba.into_raw())),
        facts,
    }
}

fn pyramid(width: u32, height: u32, rgba: Vec<u8>) -> Vec<Level> {
    let mut levels = vec![Level {
        width,
        height,
        rgba: Arc::new(rgba),
    }];
    while let Some(last) = levels.last()
        && last.width.max(last.height) > SMALLEST_LEVEL
    {
        let half = halve(last);
        levels.push(half);
    }
    levels
}

fn halve(level: &Level) -> Level {
    let (width, height) = (level.width.div_ceil(2), level.height.div_ceil(2));
    let stride = level.width as usize * 4;
    let last_x = level.width as usize - 1;
    let last_y = level.height as usize - 1;
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    let row_of = |y: usize| {
        level
            .rgba
            .get(y * stride..(y + 1) * stride)
            .unwrap_or_default()
    };
    for (y, row) in rgba.chunks_exact_mut(width as usize * 4).enumerate() {
        let (top, bottom) = (row_of((2 * y).min(last_y)), row_of((2 * y + 1).min(last_y)));
        for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (left, right) = (8 * x, 4 * (2 * x + 1).min(last_x));
            for (channel, value) in pixel.iter_mut().enumerate() {
                let at = |line: &[u8], start: usize| {
                    u32::from(line.get(start + channel).copied().unwrap_or(0))
                };
                let sum = at(top, left) + at(top, right) + at(bottom, left) + at(bottom, right);
                *value = ((sum + 2) / 4) as u8;
            }
        }
    }
    Level {
        width,
        height,
        rgba: Arc::new(rgba),
    }
}

fn animation<'a>(
    decoder: impl AnimationDecoder<'a>,
    width: u32,
    height: u32,
    kind: Kind,
) -> Result<Decoded, Failure> {
    let frame_bytes = u64::from(width) * u64::from(height) * 4;
    let fits = (ANIMATION_BYTES / frame_bytes.max(1)).max(1) as usize;
    let mut frames = Vec::new();
    let mut complete = true;
    for frame in decoder.into_frames() {
        if frames.len() >= fits.min(MAX_FRAMES) {
            complete = false;
            break;
        }
        let Ok(frame) = frame else {
            break;
        };
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = match denominator {
            0 => DEFAULT_DELAY,
            _ => Duration::from_millis(u64::from(numerator / denominator)),
        };
        let buffer = frame.into_buffer();
        if buffer.dimensions() != (width, height) {
            break;
        }
        frames.push(Frame {
            rgba: Arc::new(buffer.into_raw()),
            delay: match delay < MIN_DELAY {
                true => DEFAULT_DELAY,
                false => delay,
            },
        });
    }
    if frames.is_empty() {
        return Err(Failure::Damaged);
    }
    let facts = Facts::default().frames(frames.len(), complete);
    if frames.len() == 1 || !complete {
        let first = frames.swap_remove(0);
        let rgba = Arc::try_unwrap(first.rgba).unwrap_or_else(|shared| (*shared).clone());
        return Ok(Decoded {
            width,
            height,
            pixels: Pixels::Still(pyramid(width, height, rgba)),
            facts: Facts {
                kind: Some(kind),
                ..facts
            },
        });
    }
    Ok(Decoded {
        width,
        height,
        pixels: Pixels::Animation(frames),
        facts: Facts {
            kind: Some(kind),
            ..facts
        },
    })
}

fn decode_gif(bytes: &[u8]) -> Result<Decoded, Failure> {
    let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|_| Failure::Damaged)?;
    decoder.set_limits(limits()).map_err(|_| Failure::Damaged)?;
    let (width, height) = decoder.dimensions();
    allowed(width, height)?;
    animation(decoder, width, height, Kind::Gif)
}

fn decode_webp(bytes: &[u8]) -> Result<Decoded, Failure> {
    let mut decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(|_| Failure::Damaged)?;
    let (width, height) = decoder.dimensions();
    allowed(width, height)?;
    if decoder.has_animation() {
        return animation(decoder, width, height, Kind::Webp);
    }
    decoder.set_limits(limits()).map_err(|_| Failure::Damaged)?;
    let exif = decoder.exif_metadata().ok().flatten();
    let orientation = exif
        .as_deref()
        .and_then(Orientation::from_exif_chunk)
        .unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(|_| Failure::Damaged)?;
    image.apply_orientation(orientation);
    Ok(still(image.into_rgba8(), camera_facts(exif)))
}

fn decode_svg(bytes: &[u8]) -> Result<Decoded, Failure> {
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_data(bytes, &options).map_err(|_| Failure::Damaged)?;
    let size = tree.size();
    let longest = size.width().max(size.height());
    if !longest.is_finite() || longest <= 0.0 {
        return Err(Failure::Damaged);
    }
    let scale = (SVG_SIDE / longest).max(1.0).min(SVG_MAX_SIDE / longest);
    let width = (size.width() * scale).round().max(1.0) as u32;
    let height = (size.height() * scale).round().max(1.0) as u32;
    allowed(width, height)?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or(Failure::Damaged)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    let facts = Facts {
        kind: Some(Kind::Svg),
        natural: Some((size.width().round() as u32, size.height().round() as u32)),
        ..Facts::default()
    };
    Ok(Decoded {
        width,
        height,
        pixels: Pixels::Still(pyramid(width, height, rgba)),
        facts,
    })
}

pub fn open(path: &Path) -> Result<(Kind, Vec<u8>), Failure> {
    let bytes = read_file(path)?;
    let kind = Kind::of(&bytes, path).ok_or(Failure::NotSupported)?;
    Ok((kind, bytes))
}

pub fn decode_file(path: &Path) -> Result<Decoded, Failure> {
    let (kind, bytes) = open(path)?;
    let mut decoded = decode(&bytes, kind)?;
    decoded.facts.kind = Some(kind);
    Ok(decoded)
}
