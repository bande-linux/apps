use std::sync::Arc;

use encoding_rs::Encoding;

use crate::encoding::{LineEnding, TextEncoding, decode};
use crate::find::words;
use crate::worker::WriteError;

pub struct Loaded {
    pub text: String,
    pub encoding: TextEncoding,
    pub line_ending: LineEnding,
    pub damaged: bool,
    pub from_disk: bool,
    pub words: usize,
    pub characters: usize,
}

pub fn read(bytes: Vec<u8>, chosen: Option<&'static Encoding>) -> Result<Loaded, String> {
    let decoded = decode(bytes, chosen);
    Ok(Loaded {
        words: words(&decoded.text),
        characters: decoded.text.chars().count(),
        text: decoded.text,
        encoding: decoded.encoding,
        line_ending: decoded.line_ending,
        damaged: decoded.damaged,
        from_disk: true,
    })
}

pub fn encode(
    text: &Arc<String>,
    encoding: TextEncoding,
    line_ending: LineEnding,
) -> Result<Vec<u8>, WriteError> {
    encoding
        .encode(text, line_ending)
        .map_err(|_| WriteError::Unmappable)
}
