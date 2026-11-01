use data_encoding::{Encoding, Specification};
use std::sync::LazyLock;

static MODIFIED_BASE64: LazyLock<Option<Encoding>> = LazyLock::new(|| {
    let mut specification = Specification::new();
    specification
        .symbols
        .push_str("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,");
    specification.check_trailing_bits = false;
    specification.encoding().ok()
});

pub fn decode(name: &str) -> String {
    let Some(base64) = MODIFIED_BASE64.as_ref() else {
        return name.to_owned();
    };
    let mut decoded = String::new();
    let mut rest = name;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('-') else {
            decoded.push_str(&rest[start..]);
            return decoded;
        };
        let encoded = &after[..end];
        if encoded.is_empty() {
            decoded.push('&');
        } else {
            let units = base64
                .decode(encoded.as_bytes())
                .ok()
                .map(|bytes| {
                    bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                        .collect::<Vec<_>>()
                })
                .and_then(|units| String::from_utf16(&units).ok());
            match units {
                Some(text) => decoded.push_str(&text),
                None => decoded.push_str(&rest[start..start + end + 2]),
            }
        }
        rest = &after[end + 1..];
    }
    decoded.push_str(rest);
    decoded
}

pub fn encode(name: &str) -> String {
    let Some(base64) = MODIFIED_BASE64.as_ref() else {
        return name.to_owned();
    };
    let mut encoded = String::new();
    let mut pending = Vec::<u16>::new();
    let flush = |pending: &mut Vec<u16>, encoded: &mut String| {
        if pending.is_empty() {
            return;
        }
        let bytes = pending
            .iter()
            .flat_map(|unit| unit.to_be_bytes())
            .collect::<Vec<_>>();
        encoded.push('&');
        encoded.push_str(base64.encode(&bytes).trim_end_matches('='));
        encoded.push('-');
        pending.clear();
    };
    for character in name.chars() {
        if (' '..='~').contains(&character) {
            flush(&mut pending, &mut encoded);
            match character {
                '&' => encoded.push_str("&-"),
                other => encoded.push(other),
            }
        } else {
            let mut units = [0u16; 2];
            pending.extend_from_slice(character.encode_utf16(&mut units));
        }
    }
    flush(&mut pending, &mut encoded);
    encoded
}
