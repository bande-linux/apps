use encoding_rs::{
    BIG5, EUC_JP, EUC_KR, Encoding, GBK, IBM866, ISO_8859_2, ISO_8859_5, ISO_8859_7, ISO_8859_15,
    KOI8_R, MACINTOSH, SHIFT_JIS, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1250, WINDOWS_1251,
    WINDOWS_1252, WINDOWS_1253, WINDOWS_1254,
};

pub const CHOICES: [&Encoding; 21] = [
    UTF_8,
    UTF_16LE,
    UTF_16BE,
    WINDOWS_1252,
    ISO_8859_15,
    MACINTOSH,
    WINDOWS_1250,
    ISO_8859_2,
    WINDOWS_1251,
    ISO_8859_5,
    KOI8_R,
    IBM866,
    WINDOWS_1253,
    ISO_8859_7,
    WINDOWS_1254,
    SHIFT_JIS,
    EUC_JP,
    GBK,
    BIG5,
    EUC_KR,
    encoding_rs::WINDOWS_874,
];

const UTF16_SAMPLE: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub const ALL: [Self; 3] = [Self::Lf, Self::CrLf, Self::Cr];

    pub fn name(self) -> &'static str {
        match self {
            Self::Lf => "lf",
            Self::CrLf => "crlf",
            Self::Cr => "cr",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|ending| ending.name() == name)
    }

    pub fn detect(text: &str) -> Self {
        match text.find(['\n', '\r']).map(|at| &text.as_bytes()[at..]) {
            Some([b'\r', b'\n', ..]) => Self::CrLf,
            Some([b'\r', ..]) => Self::Cr,
            _ => Self::Lf,
        }
    }

    fn apply(self, text: &str) -> std::borrow::Cow<'_, str> {
        match self {
            Self::Lf => text.into(),
            Self::CrLf => text.replace('\n', "\r\n").into(),
            Self::Cr => text.replace('\n', "\r").into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextEncoding {
    pub encoding: &'static Encoding,
    pub bom: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    pub encoding: TextEncoding,
    pub line_ending: LineEnding,
    pub damaged: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Unmappable;

impl TextEncoding {
    pub const UTF8: Self = Self {
        encoding: UTF_8,
        bom: false,
    };

    pub fn named(name: &str, bom: bool) -> Option<Self> {
        Encoding::for_label(name.as_bytes()).map(|encoding| Self { encoding, bom })
    }

    pub fn is_unicode(self) -> bool {
        [UTF_8, UTF_16LE, UTF_16BE].contains(&self.encoding)
    }

    pub fn encode(self, text: &str, line_ending: LineEnding) -> Result<Vec<u8>, Unmappable> {
        let text = line_ending.apply(text);
        let utf16 = |little: bool| {
            let mut bytes = Vec::with_capacity(text.len() * 2 + 2);
            for unit in std::iter::once(0xFEFF).chain(text.encode_utf16()) {
                bytes.extend(match little {
                    true => unit.to_le_bytes(),
                    false => unit.to_be_bytes(),
                });
            }
            bytes
        };
        match self.encoding {
            encoding if encoding == UTF_16LE => Ok(utf16(true)),
            encoding if encoding == UTF_16BE => Ok(utf16(false)),
            encoding if encoding == UTF_8 => {
                let mut bytes = Vec::with_capacity(text.len() + 3);
                if self.bom {
                    bytes.extend(b"\xEF\xBB\xBF");
                }
                bytes.extend(text.as_bytes());
                Ok(bytes)
            }
            encoding => {
                let mut encoder = encoding.new_encoder();
                let mut bytes = Vec::with_capacity(
                    encoder
                        .max_buffer_length_from_utf8_without_replacement(text.len())
                        .unwrap_or(text.len()),
                );
                let (result, _) =
                    encoder.encode_from_utf8_to_vec_without_replacement(&text, &mut bytes, true);
                match result {
                    encoding_rs::EncoderResult::InputEmpty => Ok(bytes),
                    _ => Err(Unmappable),
                }
            }
        }
    }
}

fn looks_utf16(bytes: &[u8]) -> Option<&'static Encoding> {
    let sample = &bytes[..bytes.len().min(UTF16_SAMPLE) & !1];
    let pairs = sample.len() / 2;
    if pairs < 2 {
        return None;
    }
    let zero_at = |parity: usize| {
        sample
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|pair| pair[parity] == 0 && pair[1 - parity] != 0)
            .count()
    };
    match (zero_at(1) * 10 >= pairs * 4, zero_at(0) * 10 >= pairs * 4) {
        (true, false) => Some(UTF_16LE),
        (false, true) => Some(UTF_16BE),
        _ => None,
    }
}

pub fn decode(bytes: Vec<u8>, chosen: Option<&'static Encoding>) -> Decoded {
    let (encoding, bom) = match (chosen, Encoding::for_bom(&bytes)) {
        (Some(chosen), Some((found, length))) if chosen == found => (chosen, length),
        (Some(chosen), _) => (chosen, 0),
        (None, Some((found, length))) => (found, length),
        (None, None) => match looks_utf16(&bytes) {
            Some(found) => (found, 0),
            None => match std::str::from_utf8(&bytes) {
                Ok(_) => (UTF_8, 0),
                Err(error) if error.error_len().is_none() => (UTF_8, 0),
                Err(_) => (WINDOWS_1252, 0),
            },
        },
    };
    let encoding_used = TextEncoding {
        encoding,
        bom: bom > 0,
    };
    let (text, damaged) = match (encoding == UTF_8, bom) {
        (true, 0) => match String::from_utf8(bytes) {
            Ok(text) => (text, false),
            Err(error) => (String::from_utf8_lossy(error.as_bytes()).into_owned(), true),
        },
        _ => {
            let (text, damaged) = encoding.decode_without_bom_handling(&bytes[bom..]);
            (text.into_owned(), damaged)
        }
    };
    let line_ending = LineEnding::detect(&text);
    let text = match text.contains('\r') {
        true => text.replace("\r\n", "\n").replace('\r', "\n"),
        false => text,
    };
    Decoded {
        line_ending,
        text,
        encoding: encoding_used,
        damaged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings_and_line_endings_survive_a_round_trip() -> Result<(), Unmappable> {
        let latin = b"caf\xe9\r\nna\xefve\r\n".to_vec();
        let decoded = decode(latin.clone(), None);
        assert_eq!(decoded.text, "café\nnaïve\n");
        assert_eq!(decoded.encoding.encoding, WINDOWS_1252);
        assert_eq!(decoded.line_ending, LineEnding::CrLf);
        assert_eq!(
            decoded.encoding.encode(&decoded.text, decoded.line_ending),
            Ok(latin)
        );
        assert_eq!(
            decoded.encoding.encode("snow ☃", LineEnding::Lf),
            Err(Unmappable)
        );

        let utf16 = TextEncoding {
            encoding: UTF_16LE,
            bom: true,
        }
        .encode("a\nb", LineEnding::Cr)?;
        let decoded = decode(utf16[2..].to_vec(), None);
        assert_eq!(
            (decoded.text.as_str(), decoded.line_ending),
            ("a\nb", LineEnding::Cr)
        );
        assert_eq!(decoded.encoding.encoding, UTF_16LE);

        let bom = decode(b"\xEF\xBB\xBFx\xff".to_vec(), None);
        assert_eq!(
            (bom.text.as_str(), bom.encoding.bom, bom.damaged),
            ("x\u{FFFD}", true, true)
        );
        let cut = decode("ü".as_bytes()[..1].to_vec(), None);
        assert_eq!((cut.encoding.encoding, cut.damaged), (UTF_8, true));
        Ok(())
    }
}
