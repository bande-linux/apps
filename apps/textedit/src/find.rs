use std::ops::Range;

use regex::{Regex, RegexBuilder};

const COMPILED_LIMIT: usize = 16 << 20;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub pattern: String,
    pub regex: bool,
    pub case: bool,
}

impl Query {
    pub fn compile(&self) -> Result<Regex, regex::Error> {
        let pattern = match self.regex {
            true => self.pattern.clone(),
            false => regex::escape(&self.pattern),
        };
        RegexBuilder::new(&pattern)
            .case_insensitive(!self.case)
            .multi_line(true)
            .size_limit(COMPILED_LIMIT)
            .build()
    }

    pub fn substitution(&self, matcher: &Regex, found: &str, replacement: &str) -> String {
        match self.regex {
            true => matcher.replace(found, replacement).into_owned(),
            false => replacement.to_owned(),
        }
    }

    pub fn replace_all(&self, matcher: &Regex, text: &str, replacement: &str) -> (String, usize) {
        let count = matcher
            .find_iter(text)
            .filter(|found| !found.is_empty())
            .count();
        let replaced = match self.regex {
            true => matcher.replace_all(text, replacement),
            false => matcher.replace_all(text, regex::NoExpand(replacement)),
        };
        (replaced.into_owned(), count)
    }
}

fn first_after(matcher: &Regex, text: &str, mut at: usize) -> Option<Range<usize>> {
    while at <= text.len() {
        let found = matcher.find_at(text, at)?;
        if !found.is_empty() {
            return Some(found.range());
        }
        at = found.end() + text[found.end()..].chars().next()?.len_utf8();
    }
    None
}

pub fn next(matcher: &Regex, text: &str, from: usize, forward: bool) -> Option<Range<usize>> {
    let from = from.min(text.len());
    match forward {
        true => first_after(matcher, text, from)
            .or_else(|| first_after(matcher, text, 0).filter(|found| found.start < from)),
        false => {
            let before = matcher
                .find_iter(text)
                .map(|found| found.range())
                .filter(|found| !found.is_empty());
            let mut last = None;
            let mut wrapped = None;
            for found in before {
                match found.start < from {
                    true => last = Some(found),
                    false => wrapped = Some(found),
                }
            }
            last.or(wrapped)
        }
    }
}

pub fn line_range(text: &str, line: usize) -> Option<Range<usize>> {
    let start = match line {
        0 => return None,
        1 => 0,
        _ => text.match_indices('\n').nth(line - 2)?.0 + 1,
    };
    let end = text[start..].find('\n').map_or(text.len(), |at| start + at);
    Some(start..end)
}

pub fn words(text: &str) -> usize {
    text.split_whitespace().count()
}
