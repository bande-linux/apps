use web_host::Url;

use crate::settings::SearchEngine;

pub(crate) struct Typed {
    pub(crate) url: Url,
    pub(crate) fallback: Option<Url>,
}

pub(crate) enum Input {
    Address(Typed),
    Search(String),
}

pub(crate) fn parse(input: &str, search: Option<SearchEngine>) -> Option<Url> {
    match typed(input)? {
        Input::Address(typed) => Some(typed.url),
        Input::Search(query) => search?.search(&query),
    }
}

pub(crate) fn typed(input: &str) -> Option<Input> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if let Ok(url) = Url::parse(input)
        && matches!(url.scheme(), "http" | "https")
    {
        return Some(Input::Address(Typed {
            url,
            fallback: None,
        }));
    }
    let host = input.split(['/', '?', '#']).next().unwrap_or(input);
    let looks_like_host = !input.contains(char::is_whitespace)
        && (host.contains('.') || host.starts_with("localhost"))
        && !host.ends_with('.');
    if looks_like_host && let Ok(url) = Url::parse(&format!("https://{input}")) {
        let fallback = Url::parse(&format!("http://{input}")).ok();
        return Some(Input::Address(Typed { url, fallback }));
    }
    Some(Input::Search(input.to_owned()))
}
