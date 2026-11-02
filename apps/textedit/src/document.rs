use std::path::Path;

use bande_toolkit::ui::RichText;
use encoding_rs::Encoding;

use crate::encoding::{Decoded, LineEnding, TextEncoding, decode};
use crate::find::words;
use crate::formats::{docx, html, markdown, odt, rtf};
use crate::worker::WriteError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Plain,
    Rtf,
    Markdown,
    Html,
    Docx,
    Odt,
}

impl Format {
    const ALL: [Self; 6] = [
        Self::Plain,
        Self::Rtf,
        Self::Markdown,
        Self::Html,
        Self::Docx,
        Self::Odt,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Rtf => "rtf",
            Self::Markdown => "markdown",
            Self::Html => "html",
            Self::Docx => "docx",
            Self::Odt => "odt",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.name() == name)
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Plain => "txt",
            Self::Rtf => "rtf",
            Self::Markdown => "md",
            Self::Html => "html",
            Self::Docx => "docx",
            Self::Odt => "odt",
        }
    }

    pub fn by_extension(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match extension.as_str() {
            "rtf" => Self::Rtf,
            "docx" | "docm" | "dotx" => Self::Docx,
            "odt" | "ott" => Self::Odt,
            "md" | "markdown" | "mdown" | "mkd" => Self::Markdown,
            "html" | "htm" | "xhtml" => Self::Html,
            _ => return None,
        })
    }

    pub fn of(path: &Path, bytes: &[u8]) -> Self {
        match Self::by_extension(path) {
            _ if bytes.starts_with(b"{\\rtf") => Self::Rtf,
            Some(Self::Docx | Self::Odt) if !bytes.starts_with(b"PK") => Self::Plain,
            Some(format) => format,
            None => Self::Plain,
        }
    }

    pub fn is_rich(self) -> bool {
        self != Self::Plain
    }

    pub fn is_text(self) -> bool {
        matches!(self, Self::Plain | Self::Markdown)
    }
}

pub struct Loaded {
    pub text: String,
    pub rich: Option<RichText>,
    pub format: Format,
    pub encoding: TextEncoding,
    pub line_ending: LineEnding,
    pub damaged: bool,
    pub lossy: bool,
    pub from_disk: bool,
    pub words: usize,
    pub characters: usize,
}

impl Loaded {
    fn new(decoded: Decoded, rich: Option<RichText>, format: Format) -> Self {
        let text = rich.as_ref().map_or(&decoded.text, |rich| &rich.text);
        Self {
            words: words(text),
            characters: text.chars().count(),
            rich: rich.filter(|_| format.is_rich()),
            text: decoded.text,
            format,
            encoding: decoded.encoding,
            line_ending: decoded.line_ending,
            damaged: decoded.damaged,
            lossy: false,
            from_disk: true,
        }
    }
}

pub fn open(
    path: &Path,
    bytes: Vec<u8>,
    chosen: Option<&'static Encoding>,
) -> Result<Loaded, String> {
    let format = Format::of(path, &bytes);
    read(bytes, format, chosen)
}

fn binary(rich: RichText, format: Format, lossy: bool) -> Loaded {
    let decoded = Decoded {
        text: String::new(),
        encoding: TextEncoding::UTF8,
        line_ending: LineEnding::Lf,
        damaged: false,
    };
    Loaded {
        lossy,
        ..Loaded::new(decoded, Some(rich), format)
    }
}

pub fn read(
    bytes: Vec<u8>,
    format: Format,
    chosen: Option<&'static Encoding>,
) -> Result<Loaded, String> {
    Ok(match format {
        Format::Rtf => binary(rtf::read(&bytes), format, false),
        Format::Docx => {
            let opened = docx::read(&bytes)?;
            binary(opened.rich, format, opened.lossy)
        }
        Format::Odt => {
            let opened = odt::read(&bytes)?;
            binary(opened.rich, format, opened.lossy)
        }
        Format::Markdown => {
            let decoded = decode(bytes, chosen);
            let rich = markdown::read(&decoded.text);
            Loaded::new(decoded, Some(rich), format)
        }
        Format::Html => {
            let decoded = decode(bytes, chosen.or(Some(encoding_rs::UTF_8)));
            let rich = html::read(&decoded.text);
            Loaded::new(decoded, Some(rich), format)
        }
        Format::Plain => Loaded::new(decode(bytes, chosen), None, format),
    })
}

pub enum Content {
    Plain(std::sync::Arc<String>),
    Rich(RichText),
}

pub fn encode(
    content: &Content,
    format: Format,
    encoding: TextEncoding,
    line_ending: LineEnding,
    title: &str,
) -> Result<Vec<u8>, WriteError> {
    let text = match (content, format) {
        (Content::Plain(text), _) => text.to_string(),
        (Content::Rich(rich), Format::Rtf) => return Ok(rtf::write(rich).into_bytes()),
        (Content::Rich(rich), Format::Docx) => return docx::write(rich).map_err(WriteError::Io),
        (Content::Rich(rich), Format::Odt) => return odt::write(rich).map_err(WriteError::Io),
        (Content::Rich(rich), Format::Html) => {
            return TextEncoding::UTF8
                .encode(&html::write(rich, title), line_ending)
                .map_err(|_| WriteError::Unmappable);
        }
        (Content::Rich(rich), Format::Markdown) => markdown::write(rich),
        (Content::Rich(rich), Format::Plain) => rich.text.clone(),
    };
    encoding
        .encode(&text, line_ending)
        .map_err(|_| WriteError::Unmappable)
}
