use web_host::Url;

const SEARCH: &str = "https://html.duckduckgo.com/html/";

pub(crate) struct Typed {
    pub(crate) url: Url,
    pub(crate) fallback: Option<Url>,
}

pub(crate) fn parse(input: &str) -> Option<Url> {
    typed(input).map(|typed| typed.url)
}

pub(crate) fn typed(input: &str) -> Option<Typed> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if let Ok(url) = Url::parse(input)
        && matches!(url.scheme(), "http" | "https")
    {
        return Some(Typed {
            url,
            fallback: None,
        });
    }
    let host = input.split(['/', '?', '#']).next().unwrap_or(input);
    let looks_like_host = !input.contains(char::is_whitespace)
        && (host.contains('.') || host.starts_with("localhost"))
        && !host.ends_with('.');
    if looks_like_host && let Ok(url) = Url::parse(&format!("https://{input}")) {
        let fallback = Url::parse(&format!("http://{input}")).ok();
        return Some(Typed { url, fallback });
    }
    let url = Url::parse_with_params(SEARCH, [("q", input)]).ok()?;
    Some(Typed {
        url,
        fallback: None,
    })
}
