use exif::{Exif, In, Tag};

use crate::decode::Kind;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub kind: Option<Kind>,
    pub natural: Option<(u32, u32)>,
    pub frames: Option<(usize, bool)>,
    pub camera: Vec<(&'static str, String)>,
}

impl Facts {
    pub fn frames(self, count: usize, complete: bool) -> Self {
        Self {
            frames: (count > 1).then_some((count, complete)),
            ..self
        }
    }
}

const SHOWN: [(&str, &[Tag]); 7] = [
    ("info-camera", &[Tag::Make, Tag::Model]),
    ("info-lens", &[Tag::LensModel]),
    ("info-taken", &[Tag::DateTimeOriginal]),
    ("info-exposure", &[Tag::ExposureTime]),
    ("info-aperture", &[Tag::FNumber]),
    ("info-iso", &[Tag::PhotographicSensitivity]),
    ("info-focal-length", &[Tag::FocalLength]),
];

fn text(exif: &Exif, tag: Tag) -> Option<String> {
    let field = exif.get_field(tag, In::PRIMARY)?;
    let shown = field.display_value().with_unit(exif).to_string();
    let shown = shown.trim().trim_matches('"').trim().to_owned();
    (!shown.is_empty()).then_some(shown)
}

fn location(exif: &Exif) -> Option<String> {
    let coordinate = |value: Tag, reference: Tag| {
        let field = exif.get_field(value, In::PRIMARY)?;
        let exif::Value::Rational(parts) = &field.value else {
            return None;
        };
        let degrees = parts
            .iter()
            .take(3)
            .zip([1.0, 60.0, 3600.0])
            .map(|(part, divisor)| part.to_f64() / divisor)
            .sum::<f64>();
        let side = text(exif, reference).unwrap_or_default();
        degrees
            .is_finite()
            .then(|| format!("{degrees:.5}° {side}").trim().to_owned())
    };
    Some(format!(
        "{}, {}",
        coordinate(Tag::GPSLatitude, Tag::GPSLatitudeRef)?,
        coordinate(Tag::GPSLongitude, Tag::GPSLongitudeRef)?
    ))
}

pub fn camera_facts(chunk: Option<Vec<u8>>) -> Facts {
    let Some(exif) = chunk.and_then(|chunk| exif::Reader::new().read_raw(chunk).ok()) else {
        return Facts::default();
    };
    let mut camera: Vec<(&'static str, String)> = SHOWN
        .iter()
        .filter_map(|(label, tags)| {
            let mut parts: Vec<String> = tags.iter().filter_map(|tag| text(&exif, *tag)).collect();
            if let [make, model] = parts.as_slice()
                && model.starts_with(make.as_str())
            {
                parts.remove(0);
            }
            (!parts.is_empty()).then(|| (*label, parts.join(" ")))
        })
        .collect();
    if let Some(place) = location(&exif) {
        camera.push(("info-location", place));
    }
    Facts {
        camera,
        ..Facts::default()
    }
}
