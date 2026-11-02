use std::io::{self, BufRead, BufReader, Read, Write};

const MAX_HEADER: usize = 64 * 1024;
const MAX_HEADERS: usize = 64;

enum Framing {
    Length(u64),
    Chunked { left: u64, done: bool },
    Close,
}

pub struct Body<R> {
    reader: BufReader<R>,
    framing: Framing,
}

pub struct Response<R> {
    pub status: u16,
    pub body: Body<R>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

pub fn exchange<S: Read + Write>(
    mut stream: S,
    method: &str,
    path: &str,
) -> io::Result<Response<S>> {
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: docker\r\nUser-Agent: Velox\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )?;
    stream.flush()?;
    read_response(stream)
}

pub fn read_response<R: Read>(stream: R) -> io::Result<Response<R>> {
    let mut reader = BufReader::new(stream);
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        if reader.read_until(b'\n', &mut head)? == 0 || head.len() > MAX_HEADER {
            return Err(invalid("the engine sent no complete HTTP header"));
        }
    }
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut response = httparse::Response::new(&mut headers);
    response
        .parse(&head)
        .map_err(|error| invalid(&format!("bad HTTP header: {error}")))?;
    let status = response.code.ok_or_else(|| invalid("no HTTP status"))?;
    let value = |name: &str| {
        response
            .headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .and_then(|header| std::str::from_utf8(header.value).ok())
    };
    let chunked = value("transfer-encoding")
        .is_some_and(|encoding| encoding.to_ascii_lowercase().contains("chunked"));
    let framing = match (chunked, value("content-length")) {
        (true, _) => Framing::Chunked {
            left: 0,
            done: false,
        },
        (false, Some(length)) => Framing::Length(
            length
                .trim()
                .parse()
                .map_err(|_| invalid("bad Content-Length"))?,
        ),
        _ if status == 204 || status == 304 => Framing::Length(0),
        _ => Framing::Close,
    };
    Ok(Response {
        status,
        body: Body { reader, framing },
    })
}

impl<R: Read> Body<R> {
    fn next_chunk(&mut self) -> io::Result<u64> {
        let mut line = String::new();
        self.reader.read_line(&mut line)?;
        let size = line.split(';').next().unwrap_or_default().trim();
        let size = u64::from_str_radix(size, 16).map_err(|_| invalid("bad chunk size"))?;
        if size == 0 {
            loop {
                line.clear();
                if self.reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
                    break;
                }
            }
        }
        Ok(size)
    }

    fn end_chunk(&mut self) -> io::Result<()> {
        let mut end = String::new();
        self.reader.read_line(&mut end)?;
        match end.trim().is_empty() {
            true => Ok(()),
            false => Err(invalid("a chunk did not end with CRLF")),
        }
    }
}

impl<R: Read> Read for Body<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        match self.framing {
            Framing::Close => self.reader.read(buffer),
            Framing::Length(0) => Ok(0),
            Framing::Length(left) => {
                let wanted = buffer
                    .len()
                    .min(usize::try_from(left).unwrap_or(usize::MAX));
                let read = self.reader.read(&mut buffer[..wanted])?;
                if read == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                self.framing = Framing::Length(left - read as u64);
                Ok(read)
            }
            Framing::Chunked { done: true, .. } => Ok(0),
            Framing::Chunked { left, .. } => {
                let left = match left {
                    0 => self.next_chunk()?,
                    left => left,
                };
                if left == 0 {
                    self.framing = Framing::Chunked { left, done: true };
                    return Ok(0);
                }
                let wanted = buffer
                    .len()
                    .min(usize::try_from(left).unwrap_or(usize::MAX));
                let read = self.reader.read(&mut buffer[..wanted])?;
                if read == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                let left = left - read as u64;
                if left == 0 {
                    self.end_chunk()?;
                }
                self.framing = Framing::Chunked { left, done: false };
                Ok(read)
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io::Read;

    use super::read_response;

    #[test]
    fn chunked_bodies_decode_across_split_chunks_extensions_and_trailers() {
        let wire = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
            7;name=x\r\n{\"a\":1}\r\n\
            3\r\n\n{\"\r\nA\r\nb\":2}\n{\"c\"\r\n\
            3\r\n:3}\r\n0\r\nTrailer: yes\r\n\r\nleftover";
        let mut response = read_response(&wire[..]).unwrap();
        let mut body = String::new();
        response.body.read_to_string(&mut body).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(body, "{\"a\":1}\n{\"b\":2}\n{\"c\":3}");
    }
}
