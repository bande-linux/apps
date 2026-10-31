use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Atom(String),
    Number(u64),
    String(Vec<u8>),
    Nil,
    List(Vec<Value>),
}

impl Value {
    pub fn atom(&self) -> Option<&str> {
        match self {
            Self::Atom(atom) => Some(atom),
            _ => None,
        }
    }

    pub fn is_atom(&self, name: &str) -> bool {
        self.atom().is_some_and(|atom| atom.eq_ignore_ascii_case(name))
    }

    pub fn number(&self) -> Option<u64> {
        match self {
            Self::Number(number) => Some(*number),
            Self::Atom(atom) => atom.parse().ok(),
            _ => None,
        }
    }

    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::String(bytes) => Some(bytes),
            Self::Atom(atom) => Some(atom.as_bytes()),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<String> {
        match self {
            Self::Number(number) => Some(number.to_string()),
            other => other
                .bytes()
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned()),
        }
    }

    pub fn list(&self) -> Option<&[Value]> {
        match self {
            Self::List(items) => Some(items),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    No,
    Bad,
    Bye,
    PreAuth,
}

impl Status {
    fn parse(word: &str) -> Option<Self> {
        [
            ("OK", Self::Ok),
            ("NO", Self::No),
            ("BAD", Self::Bad),
            ("BYE", Self::Bye),
            ("PREAUTH", Self::PreAuth),
        ]
        .into_iter()
        .find_map(|(name, status)| word.eq_ignore_ascii_case(name).then_some(status))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Status {
        tag: Option<String>,
        status: Status,
        code: Vec<Value>,
        text: String,
    },
    Data(Vec<Value>),
    Continue(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireError(pub String);

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "malformed response: {}", self.0)
    }
}

fn error<T>(what: &str, input: &[u8]) -> Result<T, WireError> {
    let shown = String::from_utf8_lossy(&input[..input.len().min(80)]).into_owned();
    Err(WireError(format!("{what} in {shown:?}")))
}

pub fn literal_length(line: &[u8]) -> Option<usize> {
    let line = line.strip_suffix(b"\r\n").or_else(|| line.strip_suffix(b"\n"))?;
    let inner = line.strip_suffix(b"}")?;
    let open = inner.iter().rposition(|byte| *byte == b'{')?;
    let digits = &inner[open + 1..];
    let digits = digits.strip_suffix(b"+").unwrap_or(digits);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
}

fn trim_line_end(raw: &[u8]) -> &[u8] {
    let raw = raw.strip_suffix(b"\n").unwrap_or(raw);
    raw.strip_suffix(b"\r").unwrap_or(raw)
}

pub fn parse_response(raw: &[u8]) -> Result<Response, WireError> {
    let line = trim_line_end(raw);
    if let Some(rest) = line.strip_prefix(b"+") {
        return Ok(Response::Continue(
            String::from_utf8_lossy(rest.strip_prefix(b" ").unwrap_or(rest)).into_owned(),
        ));
    }
    let Some(space) = line.iter().position(|byte| *byte == b' ') else {
        return error("no tag", line);
    };
    let tag = String::from_utf8_lossy(&line[..space]).into_owned();
    let rest = &line[space + 1..];
    let word_end = rest
        .iter()
        .position(|byte| *byte == b' ')
        .unwrap_or(rest.len());
    let word = String::from_utf8_lossy(&rest[..word_end]);
    if let Some(status) = Status::parse(&word) {
        let after = rest.get(word_end + 1..).unwrap_or_default();
        let (code, text) = split_code(after)?;
        return Ok(Response::Status {
            tag: (tag != "*").then_some(tag),
            status,
            code,
            text,
        });
    }
    if tag != "*" {
        return error("tagged data", line);
    }
    let mut parser = Parser { input: rest, at: 0 };
    let values = parser.values(None)?;
    Ok(Response::Data(values))
}

fn split_code(after: &[u8]) -> Result<(Vec<Value>, String), WireError> {
    let Some(inner) = after.strip_prefix(b"[") else {
        return Ok((Vec::new(), String::from_utf8_lossy(after).into_owned()));
    };
    let Some(close) = inner.iter().position(|byte| *byte == b']') else {
        return Ok((Vec::new(), String::from_utf8_lossy(after).into_owned()));
    };
    let mut parser = Parser {
        input: &inner[..close],
        at: 0,
    };
    let code = parser.values(None).unwrap_or_default();
    let text = inner.get(close + 1..).unwrap_or_default();
    let text = text.strip_prefix(b" ").unwrap_or(text);
    Ok((code, String::from_utf8_lossy(text).into_owned()))
}

pub fn parse_values(input: &[u8]) -> Result<Vec<Value>, WireError> {
    Parser {
        input: trim_line_end(input),
        at: 0,
    }
    .values(None)
}

struct Parser<'a> {
    input: &'a [u8],
    at: usize,
}

const MAX_DEPTH: usize = 64;

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.at).copied()
    }

    fn values(&mut self, close: Option<u8>) -> Result<Vec<Value>, WireError> {
        self.values_at(close, 0)
    }

    fn values_at(&mut self, close: Option<u8>, depth: usize) -> Result<Vec<Value>, WireError> {
        if depth > MAX_DEPTH {
            return error("nesting too deep", self.input);
        }
        let mut values = Vec::new();
        loop {
            while self.peek() == Some(b' ') {
                self.at += 1;
            }
            match (self.peek(), close) {
                (None, None) => return Ok(values),
                (None, Some(_)) => return error("unclosed list", self.input),
                (Some(byte), Some(close)) if byte == close => {
                    self.at += 1;
                    return Ok(values);
                }
                (Some(b')'), _) => return error("unexpected )", self.input),
                _ => values.push(self.value(depth)?),
            }
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, WireError> {
        match self.peek() {
            Some(b'(') => {
                self.at += 1;
                Ok(Value::List(self.values_at(Some(b')'), depth + 1)?))
            }
            Some(b'"') => self.quoted(),
            Some(b'{') => self.literal(),
            Some(b'~') if self.input.get(self.at + 1) == Some(&b'{') => {
                self.at += 1;
                self.literal()
            }
            _ => self.atom(),
        }
    }

    fn quoted(&mut self) -> Result<Value, WireError> {
        self.at += 1;
        let mut bytes = Vec::new();
        loop {
            match self.peek() {
                None => return error("unterminated string", self.input),
                Some(b'"') => {
                    self.at += 1;
                    return Ok(Value::String(bytes));
                }
                Some(b'\\') => {
                    let Some(escaped) = self.input.get(self.at + 1) else {
                        return error("unterminated escape", self.input);
                    };
                    bytes.push(*escaped);
                    self.at += 2;
                }
                Some(byte) => {
                    bytes.push(byte);
                    self.at += 1;
                }
            }
        }
    }

    fn literal(&mut self) -> Result<Value, WireError> {
        let start = self.at + 1;
        let Some(close) = self.input[start..].iter().position(|byte| *byte == b'}') else {
            return error("unterminated literal length", self.input);
        };
        let digits = &self.input[start..start + close];
        let digits = digits.strip_suffix(b"+").unwrap_or(digits);
        let Some(length) = std::str::from_utf8(digits)
            .ok()
            .and_then(|digits| digits.parse::<usize>().ok())
        else {
            return error("bad literal length", self.input);
        };
        let mut body = start + close + 1;
        if self.input.get(body) == Some(&b'\r') {
            body += 1;
        }
        if self.input.get(body) == Some(&b'\n') {
            body += 1;
        }
        let Some(bytes) = self.input.get(body..body + length) else {
            return error("short literal", self.input);
        };
        self.at = body + length;
        Ok(Value::String(bytes.to_vec()))
    }

    fn atom(&mut self) -> Result<Value, WireError> {
        let start = self.at;
        let mut brackets = 0usize;
        while let Some(byte) = self.peek() {
            match byte {
                b'[' => brackets += 1,
                b']' if brackets > 0 => brackets -= 1,
                b' ' | b'(' | b')' if brackets > 0 => {}
                b' ' | b'(' | b')' | b'"' | b'{' => break,
                byte if byte < 0x20 || byte == 0x7f => break,
                _ => {}
            }
            self.at += 1;
        }
        if self.at == start {
            return error("empty atom", self.input);
        }
        let text = String::from_utf8_lossy(&self.input[start..self.at]).into_owned();
        if text.eq_ignore_ascii_case("NIL") {
            return Ok(Value::Nil);
        }
        if !text.is_empty() && text.len() < 20 && text.bytes().all(|byte| byte.is_ascii_digit()) {
            if let Ok(number) = text.parse() {
                return Ok(Value::Number(number));
            }
        }
        Ok(Value::Atom(text))
    }
}

pub fn quote(text: &str) -> Option<String> {
    if text.contains(['\r', '\n', '\0']) {
        return None;
    }
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for character in text.chars() {
        if matches!(character, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    Some(quoted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(text: &str) -> Value {
        Value::Atom(text.to_owned())
    }

    #[test]
    fn a_fetch_with_a_literal_and_a_section_parses_into_pairs() {
        let raw = b"* 12 FETCH (UID 34 FLAGS (\\Seen \\Flagged) BODY[HEADER.FIELDS (FROM TO)] {9}\r\nFrom: a\r\n RFC822.SIZE 100)\r\n";
        let Response::Data(values) = parse_response(raw).unwrap_or(Response::Data(Vec::new()))
        else {
            unreachable!()
        };
        assert_eq!(values[0], Value::Number(12));
        assert_eq!(values[1], atom("FETCH"));
        let pairs = values[2].list().unwrap_or_default();
        assert_eq!(pairs[0], atom("UID"));
        assert_eq!(pairs[1], Value::Number(34));
        assert_eq!(
            pairs[3],
            Value::List(vec![atom("\\Seen"), atom("\\Flagged")])
        );
        assert_eq!(pairs[4], atom("BODY[HEADER.FIELDS (FROM TO)]"));
        assert_eq!(pairs[5], Value::String(b"From: a\r\n".to_vec()));
        assert_eq!(pairs[7], Value::Number(100));
    }

    #[test]
    fn a_status_response_keeps_its_code_and_text() {
        let response = parse_response(b"* OK [UIDVALIDITY 3857529045] UIDs valid\r\n");
        assert_eq!(
            response,
            Ok(Response::Status {
                tag: None,
                status: Status::Ok,
                code: vec![atom("UIDVALIDITY"), Value::Number(3_857_529_045)],
                text: "UIDs valid".to_owned(),
            })
        );
        let tagged = parse_response(b"a7 NO [AUTHENTICATIONFAILED] Invalid \"credentials\r\n");
        assert!(matches!(
            tagged,
            Ok(Response::Status { tag: Some(tag), status: Status::No, ref code, ref text })
                if tag == "a7" && code == &[atom("AUTHENTICATIONFAILED")] && text == "Invalid \"credentials"
        ));
    }

    #[test]
    fn quoted_strings_unescape_and_nil_is_recognised() {
        let values = parse_values(br#"(\HasNoChildren) "/" "Sent \"Old\"" NIL"#);
        assert_eq!(
            values,
            Ok(vec![
                Value::List(vec![atom("\\HasNoChildren")]),
                Value::String(b"/".to_vec()),
                Value::String(b"Sent \"Old\"".to_vec()),
                Value::Nil,
            ])
        );
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        for raw in [
            &b"* 1 FETCH (UID 3"[..],
            b"* 1 FETCH {99}\r\nshort",
            b"* LIST ((((",
            b"* 3 FETCH )",
            b"nonsense",
            b"a1 FETCH",
        ] {
            assert!(parse_response(raw).is_err(), "{raw:?}");
        }
        let deep = format!("* X {}", "(".repeat(500));
        assert!(parse_response(deep.as_bytes()).is_err());
    }

    #[test]
    fn literal_lengths_are_found_only_at_the_end_of_a_line() {
        assert_eq!(literal_length(b"* 1 FETCH (BODY[] {120}\r\n"), Some(120));
        assert_eq!(literal_length(b"A1 APPEND x {5+}\r\n"), Some(5));
        assert_eq!(literal_length(b"* OK {not} a literal\r\n"), None);
        assert_eq!(literal_length(b"* OK {}\r\n"), None);
    }

    #[test]
    fn quoting_escapes_and_refuses_line_breaks() {
        assert_eq!(quote(r#"a "b" \c"#).as_deref(), Some(r#""a \"b\" \\c""#));
        assert_eq!(quote("a\r\nb"), None);
    }
}
