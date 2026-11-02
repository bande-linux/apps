use web_net::Url;

const SEARCH: &str = "https://html.duckduckgo.com/html/";

pub(crate) fn parse(input: &str) -> Option<Url> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if let Ok(url) = Url::parse(input)
        && matches!(url.scheme(), "http" | "https")
    {
        return Some(url);
    }
    let host = input.split(['/', '?', '#']).next().unwrap_or(input);
    let looks_like_host = !input.contains(char::is_whitespace)
        && (host.contains('.') || host.starts_with("localhost"))
        && !host.ends_with('.');
    if looks_like_host && let Ok(url) = Url::parse(&format!("https://{input}")) {
        return Some(url);
    }
    Url::parse_with_params(SEARCH, [("q", input)]).ok()
}
