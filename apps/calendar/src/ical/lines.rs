use std::fmt::Write;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub params: Vec<(String, String)>,
    pub value: String,
}

impl Property {
    pub fn new(name: &str, value: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            params: Vec::new(),
            value: value.into(),
        }
    }

    pub fn with_param(mut self, name: &str, value: &str) -> Self {
        self.params.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Component {
    pub name: String,
    pub properties: Vec<Property>,
    pub components: Vec<Component>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Line(usize),
    Unbalanced(String),
    Empty,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Line(line) => write!(f, "malformed content line {line}"),
            Self::Unbalanced(name) => write!(f, "{name} is not closed"),
            Self::Empty => write!(f, "no calendar data"),
        }
    }
}

impl std::error::Error for ParseError {}

impl Component {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            ..Self::default()
        }
    }

    pub fn property(&self, name: &str) -> Option<&Property> {
        self.properties
            .iter()
            .find(|property| property.name.eq_ignore_ascii_case(name))
    }

    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Property> {
        self.properties
            .iter()
            .filter(move |property| property.name.eq_ignore_ascii_case(name))
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        self.property(name).map(|property| property.value.as_str())
    }

    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Component> {
        self.components
            .iter()
            .filter(move |component| component.name.eq_ignore_ascii_case(name))
    }

    pub fn push(&mut self, property: Property) {
        self.properties.push(property);
    }

    pub fn to_ics(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        fold_into(out, &format!("BEGIN:{}", self.name));
        for property in &self.properties {
            let mut line = property.name.clone();
            for (name, value) in &property.params {
                let quoted = value.contains([':', ';', ',']);
                match quoted {
                    true => {
                        let _ = write!(line, ";{name}=\"{value}\"");
                    }
                    false => {
                        let _ = write!(line, ";{name}={value}");
                    }
                }
            }
            line.push(':');
            line.push_str(&property.value);
            fold_into(out, &line);
        }
        for component in &self.components {
            component.write(out);
        }
        fold_into(out, &format!("END:{}", self.name));
    }
}

const FOLD_OCTETS: usize = 75;

fn fold_into(out: &mut String, line: &str) {
    let mut width = 0;
    for character in line.chars() {
        let size = character.len_utf8();
        if width + size > FOLD_OCTETS {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(character);
        width += size;
    }
    out.push_str("\r\n");
}

fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        match (raw.strip_prefix([' ', '\t']), lines.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ if raw.is_empty() => {}
            _ => lines.push(raw.to_owned()),
        }
    }
    lines
}

fn parse_line(line: &str) -> Option<Property> {
    let mut name_end = None;
    let mut value_start = None;
    let mut quoted = false;
    for (index, character) in line.char_indices() {
        match character {
            '"' => quoted = !quoted,
            ';' if !quoted && name_end.is_none() => name_end = Some(index),
            ':' if !quoted => {
                value_start = Some(index);
                break;
            }
            _ => {}
        }
    }
    let value_start = value_start?;
    let name_end = name_end.unwrap_or(value_start);
    let name = line[..name_end].trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let mut params = Vec::new();
    if name_end < value_start {
        for param in split_unquoted(&line[name_end + 1..value_start], ';') {
            let (key, value) = param.split_once('=')?;
            params.push((
                key.trim().to_ascii_uppercase(),
                value.trim_matches('"').to_owned(),
            ));
        }
    }
    Some(Property {
        name,
        params,
        value: line[value_start + 1..].to_owned(),
    })
}

fn split_unquoted(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if character == '"' {
            quoted = !quoted;
        } else if character == separator && !quoted {
            parts.push(&text[start..index]);
            start = index + 1;
        }
    }
    parts.push(&text[start..]);
    parts
}

pub fn parse(text: &str) -> Result<Vec<Component>, ParseError> {
    let mut stack: Vec<Component> = Vec::new();
    let mut roots = Vec::new();
    for (number, line) in unfold(text).iter().enumerate() {
        let property = parse_line(line).ok_or(ParseError::Line(number + 1))?;
        match property.name.as_str() {
            "BEGIN" => stack.push(Component::new(&property.value.trim().to_ascii_uppercase())),
            "END" => {
                let component = stack.pop().ok_or(ParseError::Line(number + 1))?;
                if !component.name.eq_ignore_ascii_case(property.value.trim()) {
                    return Err(ParseError::Unbalanced(component.name));
                }
                match stack.last_mut() {
                    Some(parent) => parent.components.push(component),
                    None => roots.push(component),
                }
            }
            _ => match stack.last_mut() {
                Some(component) => component.properties.push(property),
                None => return Err(ParseError::Line(number + 1)),
            },
        }
    }
    if let Some(open) = stack.pop() {
        return Err(ParseError::Unbalanced(open.name));
    }
    match roots.is_empty() {
        true => Err(ParseError::Empty),
        false => Ok(roots),
    }
}

pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        match (character, character == '\\') {
            (_, true) => match characters.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            },
            (other, false) => out.push(other),
        }
    }
    out
}

pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            other => out.push(other),
        }
    }
    out
}

pub fn split_list(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        match character {
            '\\' if !escaped => escaped = true,
            ',' if !escaped => {
                parts.push(&value[start..index]);
                start = index + 1;
            }
            _ => escaped = false,
        }
    }
    parts.push(&value[start..]);
    parts
}
