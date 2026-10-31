use std::cell::RefCell;
use std::ops::Range;

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    Heading,
    Preformatted,
    Item,
    Rule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub quote: u8,
    pub text: String,
    pub bold: Vec<Range<usize>>,
    pub links: Vec<(Range<usize>, String)>,
}

impl Block {
    fn new(kind: BlockKind, quote: u8) -> Self {
        Self {
            kind,
            quote,
            text: String::new(),
            bold: Vec::new(),
            links: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    pub blocks: Vec<Block>,
    pub remote_images: usize,
}

impl Document {
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(|block| block.text.as_str())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

const SAFE_SCHEMES: [&str; 3] = ["http://", "https://", "mailto:"];

fn safe_link(href: &str) -> Option<String> {
    let href = href.trim();
    SAFE_SCHEMES
        .iter()
        .any(|scheme| {
            href.get(..scheme.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(scheme))
        })
        .then(|| href.to_owned())
}

#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    current: Option<Block>,
    quote: u8,
    pre: usize,
    bold: usize,
    bold_start: Option<usize>,
    link: Option<(Option<usize>, String)>,
    lists: Vec<Option<u32>>,
    skipping: Option<String>,
    remote_images: usize,
    space_pending: bool,
}

impl Builder {
    fn block(&mut self) -> &mut Block {
        let quote = self.quote;
        let kind = if self.pre > 0 {
            BlockKind::Preformatted
        } else {
            BlockKind::Paragraph
        };
        self.current.get_or_insert_with(|| Block::new(kind, quote))
    }

    fn close_block(&mut self) {
        self.end_bold();
        if let Some((start, href)) = self.link.take() {
            self.end_link(start, href);
        }
        if let Some(mut block) = self.current.take() {
            let trimmed = block.text.trim_end().len();
            block.text.truncate(trimmed);
            if !block.text.trim().is_empty() || block.kind == BlockKind::Rule {
                self.blocks.push(block);
            }
        }
        self.space_pending = false;
    }

    fn start_block(&mut self, kind: BlockKind) {
        self.close_block();
        self.current = Some(Block::new(kind, self.quote));
    }

    fn mark_starts(&mut self) {
        let length = self.text_length();
        if self.bold > 0 && self.bold_start.is_none() {
            self.bold_start = Some(length);
        }
        if let Some((start @ None, _)) = &mut self.link {
            *start = Some(length);
        }
    }

    fn push_text(&mut self, text: &str) {
        if self.pre > 0 {
            self.block();
            self.mark_starts();
            self.block().text.push_str(text);
            return;
        }
        for word in text.split_inclusive(char::is_whitespace) {
            let ends_with_space = word.ends_with(char::is_whitespace);
            let word = word.trim();
            if word.is_empty() {
                self.space_pending |= ends_with_space;
                continue;
            }
            let space = std::mem::take(&mut self.space_pending);
            let block = self.block();
            if space && !block.text.is_empty() && !block.text.ends_with(['\n', ' ']) {
                block.text.push(' ');
            }
            self.mark_starts();
            self.block().text.push_str(word);
            self.space_pending = ends_with_space;
        }
    }

    fn end_bold(&mut self) {
        if let (Some(start), Some(block)) = (self.bold_start.take(), self.current.as_mut())
            && start < block.text.len()
        {
            block.bold.push(start..block.text.len());
        }
    }

    fn end_link(&mut self, start: Option<usize>, href: String) {
        if let (Some(start), Some(block)) = (start, self.current.as_mut())
            && start < block.text.len()
        {
            block.links.push((start..block.text.len(), href));
        }
    }

    fn text_length(&self) -> usize {
        self.current.as_ref().map_or(0, |block| block.text.len())
    }

    fn line_break(&mut self) {
        let block = self.block();
        if !block.text.is_empty() {
            block.text.push('\n');
        }
        self.space_pending = false;
    }

    fn start_tag(&mut self, tag: &Tag) -> Option<RawKind> {
        let name: &str = &tag.name;
        let attribute = |wanted: &str| {
            tag.attrs
                .iter()
                .find(|attribute| &*attribute.name.local == wanted)
                .map(|attribute| attribute.value.to_string())
        };
        match name {
            "style" | "script" | "title" | "head" | "template" | "noscript" | "iframe"
            | "object" | "svg" | "math" | "textarea" | "select" => {
                if !tag.self_closing && name != "head" {
                    self.skipping = Some(name.to_owned());
                }
                return match name {
                    "script" => Some(RawKind::ScriptData),
                    "title" | "textarea" => Some(RawKind::Rcdata),
                    "style" | "iframe" | "noscript" => Some(RawKind::Rawtext),
                    _ => None,
                };
            }
            "p" | "div" | "section" | "article" | "header" | "footer" | "table" | "tr"
            | "center" | "address" | "dl" | "dt" | "dd" | "figure" | "main" | "aside" => {
                self.close_block();
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => self.start_block(BlockKind::Heading),
            "br" => self.line_break(),
            "hr" => {
                self.start_block(BlockKind::Rule);
                self.close_block();
            }
            "blockquote" => {
                self.close_block();
                self.quote = self.quote.saturating_add(1);
            }
            "pre" => {
                self.close_block();
                self.pre += 1;
            }
            "ul" => {
                self.close_block();
                self.lists.push(None);
            }
            "ol" => {
                self.close_block();
                self.lists.push(Some(1));
            }
            "li" => {
                self.start_block(BlockKind::Item);
                let marker = match self.lists.last_mut() {
                    Some(Some(number)) => {
                        *number += 1;
                        format!("{}. ", *number - 1)
                    }
                    _ => "• ".to_owned(),
                };
                let indent = "    ".repeat(self.lists.len().saturating_sub(1));
                self.block().text.push_str(&format!("{indent}{marker}"));
            }
            "td" | "th" => {
                if self.text_length() > 0 {
                    self.space_pending = true;
                }
                if name == "th" {
                    self.bold += 1;
                }
            }
            "b" | "strong" => self.bold += 1,
            "a" => {
                if let Some(href) = attribute("href").as_deref().and_then(safe_link) {
                    self.link = Some((None, href));
                }
            }
            "img" => {
                let source = attribute("src").unwrap_or_default();
                if source.starts_with("http:") || source.starts_with("https:") {
                    self.remote_images += 1;
                }
                if let Some(alt) = attribute("alt").filter(|alt| !alt.trim().is_empty()) {
                    self.push_text(&format!(" [{}] ", alt.trim()));
                }
            }
            _ => {}
        }
        None
    }

    fn end_tag(&mut self, name: &str) {
        if let Some(skipped) = &self.skipping {
            if skipped == name {
                self.skipping = None;
            }
            return;
        }
        match name {
            "p" | "div" | "section" | "article" | "header" | "footer" | "table" | "tr"
            | "center" | "address" | "dl" | "dt" | "dd" | "figure" | "main" | "aside" | "h1"
            | "h2" | "h3" | "h4" | "h5" | "h6" | "li" => self.close_block(),
            "blockquote" => {
                self.close_block();
                self.quote = self.quote.saturating_sub(1);
            }
            "pre" => {
                self.close_block();
                self.pre = self.pre.saturating_sub(1);
            }
            "ul" | "ol" => {
                self.close_block();
                self.lists.pop();
            }
            "b" | "strong" | "th" => {
                if self.bold == 1 {
                    self.end_bold();
                }
                self.bold = self.bold.saturating_sub(1);
            }
            "a" => {
                if let Some((start, href)) = self.link.take() {
                    self.end_link(start, href);
                }
            }
            _ => {}
        }
    }
}

struct Sink(RefCell<Builder>);

impl TokenSink for Sink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut builder = self.0.borrow_mut();
        match token {
            Token::TagToken(tag) => match tag.kind {
                TagKind::StartTag if builder.skipping.is_none() => {
                    if let Some(raw) = builder.start_tag(&tag) {
                        return TokenSinkResult::RawData(raw);
                    }
                }
                TagKind::StartTag => {}
                TagKind::EndTag => builder.end_tag(&tag.name),
            },
            Token::CharacterTokens(text) if builder.skipping.is_none() => builder.push_text(&text),
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

pub fn render_html(html: &str) -> Document {
    let tokenizer = Tokenizer::new(Sink(RefCell::default()), TokenizerOpts::default());
    let input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(html));
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    let mut builder = tokenizer.sink.0.take();
    builder.close_block();
    Document {
        blocks: builder.blocks,
        remote_images: builder.remote_images,
    }
}

fn find_links(text: &str) -> Vec<(Range<usize>, String)> {
    let mut links = Vec::new();
    let mut from = 0;
    while let Some(found) = ["https://", "http://"]
        .iter()
        .filter_map(|scheme| text[from..].find(scheme))
        .min()
    {
        let start = from + found;
        let end = text[start..]
            .find(|character: char| character.is_whitespace() || "<>\"".contains(character))
            .map_or(text.len(), |length| start + length);
        let end = start
            + text[start..end]
                .trim_end_matches(['.', ',', ')', ';', ':', '!', '?'])
                .len();
        links.push((start..end, text[start..end].to_owned()));
        from = end.max(start + 1);
    }
    links
}

pub fn render_plain(text: &str) -> Document {
    let mut blocks: Vec<Block> = Vec::new();
    for paragraph in text.replace("\r\n", "\n").split("\n\n") {
        let mut current: Option<Block> = None;
        for line in paragraph.lines() {
            let mut quote = 0u8;
            let mut rest = line;
            while let Some(after) = rest.strip_prefix('>') {
                quote = quote.saturating_add(1);
                rest = after.strip_prefix(' ').unwrap_or(after);
            }
            if current.as_ref().is_some_and(|block| block.quote != quote) {
                blocks.extend(current.take());
            }
            let block = current.get_or_insert_with(|| Block::new(BlockKind::Paragraph, quote));
            if !block.text.is_empty() {
                block.text.push('\n');
            }
            block.text.push_str(rest.trim_end());
        }
        blocks.extend(current);
    }
    for block in &mut blocks {
        block.links = find_links(&block.text);
    }
    blocks.retain(|block| !block.text.trim().is_empty());
    Document {
        blocks,
        remote_images: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(document: &Document) -> Vec<&str> {
        document
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect()
    }

    #[test]
    fn scripts_styles_and_head_never_show_and_whitespace_collapses() {
        let document = render_html(
            "<html><head><title>T</title><style>p { color: red }</style></head>\
             <body><script>alert('x < y')</script><p>Hello\n   <b>dear</b>   world</p>\
             <div>Second<br>line</div></body></html>",
        );
        assert_eq!(texts(&document), ["Hello dear world", "Second\nline"]);
        assert_eq!(document.blocks[0].bold, vec![6..10]);
    }

    #[test]
    fn only_safe_links_survive_and_remote_images_are_counted_not_loaded() {
        let document = render_html(
            "<p>See <a href=\"https://example.com/x\">the site</a> or \
             <a href=\"javascript:evil()\">this</a>.\
             <img src=\"https://tracker.example/pixel.gif\" alt=\"\">\
             <img src=\"cid:logo\" alt=\"Logo\"></p>",
        );
        assert_eq!(texts(&document), ["See the site or this. [Logo]"]);
        assert_eq!(
            document.blocks[0].links,
            [(4..12, "https://example.com/x".to_owned())]
        );
        assert_eq!(document.remote_images, 1);
    }

    #[test]
    fn quotes_lists_and_headings_become_their_own_blocks() {
        let document = render_html(
            "<h1>Title</h1><blockquote><p>quoted</p><blockquote>deeper</blockquote></blockquote>\
             <ol><li>one</li><li>two</li></ol><ul><li>dot</li></ul><hr>",
        );
        let kinds = document
            .blocks
            .iter()
            .map(|block| (block.kind, block.quote, block.text.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [
                (BlockKind::Heading, 0, "Title"),
                (BlockKind::Paragraph, 1, "quoted"),
                (BlockKind::Paragraph, 2, "deeper"),
                (BlockKind::Item, 0, "1. one"),
                (BlockKind::Item, 0, "2. two"),
                (BlockKind::Item, 0, "• dot"),
                (BlockKind::Rule, 0, ""),
            ]
        );
    }

    #[test]
    fn plain_text_finds_quotes_and_links() {
        let document = render_plain(
            "Hi,\nsee https://example.com/a.\n\nOn Monday you wrote:\n> old\n>> older\n",
        );
        assert_eq!(
            texts(&document),
            [
                "Hi,\nsee https://example.com/a.",
                "On Monday you wrote:",
                "old",
                "older"
            ]
        );
        assert_eq!(
            document.blocks[0].links,
            [(8..29, "https://example.com/a".to_owned())]
        );
        assert_eq!(document.blocks[2].quote, 1);
        assert_eq!(document.blocks[3].quote, 2);
    }
}
