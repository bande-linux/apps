use std::io::{self, BufRead, Read, Write};

use serde::{Deserialize, Serialize};

pub const WORKER_FLAG: &str = "--pdf-worker";
const MAX_LINE: u64 = 64 << 20;
pub const MAX_RENDER_PIXELS: u64 = 40 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Use {
    Page,
    Detail,
    Thumbnail,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Render {
    pub page: usize,
    pub scale: f32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub purpose: Use,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Ask {
    Open { password: String },
    Want { renders: Vec<Render> },
    Text { pages: Vec<usize> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Heading {
    pub title: String,
    pub level: u8,
    pub page: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Opened {
    pub pages: Vec<(f32, f32)>,
    pub outline: Vec<Heading>,
    pub version: String,
    pub details: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Glyph {
    pub text: String,
    pub rect: [f32; 4],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Reply {
    Opened(Opened),
    Locked { retry: bool },
    Failed,
    Rendered { render: Render },
    Text { page: usize, glyphs: Vec<Glyph> },
}

pub fn write<T: Serialize>(out: &mut impl Write, message: &T, payload: &[u8]) -> io::Result<()> {
    serde_json::to_writer(&mut *out, message)?;
    out.write_all(b"\n")?;
    out.write_all(payload)?;
    out.flush()
}

pub fn read<T: for<'a> Deserialize<'a>>(input: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut line = Vec::new();
    let read = input.take(MAX_LINE).read_until(b'\n', &mut line)?;
    if read == 0 {
        return Ok(None);
    }
    serde_json::from_slice(&line)
        .map(Some)
        .map_err(io::Error::other)
}

pub fn payload(input: &mut impl Read, render: &Render) -> io::Result<Vec<u8>> {
    let pixels = u64::from(render.width) * u64::from(render.height);
    if pixels > MAX_RENDER_PIXELS {
        return Err(io::Error::other("the render is too large"));
    }
    let mut bytes = vec![0; pixels as usize * 4];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}
