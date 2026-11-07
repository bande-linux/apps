use bande_toolkit::{Color, Palette};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use web_host::Url;

use crate::L10N;
use crate::places::Place;

fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            character => escaped.push(character),
        }
    }
    escaped
}

fn css(color: Color) -> String {
    format!(
        "rgba({}, {}, {}, {:.3})",
        color.r,
        color.g,
        color.b,
        f32::from(color.a) / 255.0
    )
}

fn style(palette: &Palette) -> String {
    format!(
        "html {{ background: {sheet}; }}
         body {{ margin: 0; padding: 40px 48px; min-height: 100vh; box-sizing: border-box; \
         background: {sheet}; color: {ink}; \
         font-family: Inter, sans-serif; font-size: 15px; line-height: 1.5; }}
         h1 {{ font-size: 26px; font-weight: 600; margin: 0 0 24px 0; }}
         h2 {{ font-size: 13px; font-weight: 600; color: {ink2}; text-transform: uppercase; \
         letter-spacing: 0.04em; margin: 28px 0 8px 0; }}
         a {{ color: {accent}; text-decoration: none; }}
         a:hover {{ text-decoration: underline; }}
         .row {{ padding: 6px 0; border-bottom: 1px solid {hairline}; }}
         .row small {{ display: block; color: {ink3}; font-size: 12px; }}
         .tile {{ display: inline-block; width: 168px; margin: 0 16px 16px 0; padding: 14px 16px; \
         background: {well}; border-radius: 12px; vertical-align: top; }}
         .tile small {{ display: block; color: {ink2}; font-size: 12px; }}
         .empty {{ color: {ink3}; }}",
        sheet = css(palette.sheet),
        ink = css(palette.ink),
        ink2 = css(palette.ink2),
        ink3 = css(palette.ink3),
        accent = css(palette.accent_text),
        hairline = css(palette.hairline),
        well = css(palette.well),
    )
}

fn document(title: &str, palette: &Palette, body: &str) -> Option<Url> {
    let html = format!(
        "<!doctype html><html><head><meta charset=utf-8><title>{title}</title>\
         <style>{style}</style></head><body>{body}</body></html>",
        title = escape(title),
        style = style(palette),
    );
    Url::parse(&format!(
        "data:text/html;charset=utf-8;base64,{}",
        STANDARD.encode(html)
    ))
    .ok()
}

fn label(place: &Place) -> String {
    match place.title.is_empty() {
        true => place.url.host_str().unwrap_or_default().to_owned(),
        false => place.title.clone(),
    }
}

fn row(place: &Place) -> String {
    format!(
        "<div class=row><a href=\"{url}\">{title}</a><small>{shown}</small></div>",
        url = escape(place.url.as_str()),
        title = escape(&label(place)),
        shown = escape(place.url.as_str()),
    )
}

pub(crate) fn favourites(places: &[Place], palette: &Palette) -> Option<Url> {
    let tiles: String = places
        .iter()
        .map(|place| {
            format!(
                "<a class=tile href=\"{url}\">{title}<small>{host}</small></a>",
                url = escape(place.url.as_str()),
                title = escape(&label(place)),
                host = escape(place.url.host_str().unwrap_or_default()),
            )
        })
        .collect();
    let body = match tiles.is_empty() {
        true => format!(
            "<p class=empty>{}</p>",
            escape(&L10N.tr("favourites-empty"))
        ),
        false => tiles,
    };
    let title = L10N.tr("favourites");
    document(
        &title,
        palette,
        &format!("<h1>{}</h1>{body}", escape(&title)),
    )
}

pub(crate) fn listing(
    title: &str,
    places: &[Place],
    palette: &Palette,
    empty: &str,
) -> Option<Url> {
    let mut body = format!("<h1>{}</h1>", escape(title));
    if places.is_empty() {
        body.push_str(&format!("<p class=empty>{}</p>", escape(empty)));
    }
    for place in places {
        body.push_str(&row(place));
    }
    document(title, palette, &body)
}

pub(crate) fn reader_style(palette: &Palette) -> String {
    format!(
        "html {{ background: {sheet}; }}
         body {{ margin: 0; min-height: 100vh; background: {sheet}; color: {ink}; \
         font-family: Inter, sans-serif; font-size: 18px; line-height: 1.6; }}
         article {{ max-width: 660px; margin: 0 auto; padding: 48px 32px 96px 32px; }}
         h1 {{ font-size: 34px; line-height: 1.2; font-weight: 600; margin: 0 0 8px 0; }}
         h2 {{ font-size: 25px; line-height: 1.25; font-weight: 600; margin: 36px 0 12px 0; }}
         h3, h4, h5, h6 {{ font-size: 20px; font-weight: 600; margin: 28px 0 8px 0; }}
         .site {{ color: {ink3}; font-size: 14px; margin: 0 0 32px 0; }}
         p, ul, ol, dl, blockquote, pre, figure, table {{ margin: 0 0 20px 0; }}
         a {{ color: {accent}; text-decoration: none; }}
         a:hover {{ text-decoration: underline; }}
         img {{ max-width: 100%; height: auto; border-radius: 8px; }}
         figcaption {{ color: {ink2}; font-size: 14px; margin-top: 8px; }}
         blockquote {{ padding-left: 16px; border-left: 3px solid {hairline}; color: {ink2}; }}
         pre {{ background: {well}; padding: 12px 16px; border-radius: 8px; font-size: 14px; \
         white-space: pre-wrap; }}
         code {{ font-size: 15px; }}
         table {{ border-collapse: collapse; font-size: 15px; }}
         td, th {{ border: 1px solid {hairline}; padding: 6px 10px; }}",
        sheet = css(palette.sheet),
        ink = css(palette.ink),
        ink2 = css(palette.ink2),
        ink3 = css(palette.ink3),
        accent = css(palette.accent_text),
        hairline = css(palette.hairline),
        well = css(palette.well),
    )
}
