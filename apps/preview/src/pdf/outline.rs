use std::collections::{HashMap, HashSet};

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{
    Array, Dict, Name, Object, ObjectIdentifier, String as PdfString,
};

use crate::pdf::protocol::Heading;

const MAX_HEADINGS: usize = 4096;
const MAX_DEPTH: u8 = 16;
const MAX_NAMES: usize = 65_536;
const MAX_TREE_DEPTH: usize = 32;

pub fn text(bytes: &[u8]) -> String {
    let decoded = match bytes {
        [0xFE, 0xFF, rest @ ..] => {
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_be_bytes(*pair))
                .collect();
            String::from_utf16_lossy(&units)
        }
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => bytes.iter().map(|byte| char::from(*byte)).collect(),
    };
    decoded
        .chars()
        .map(|character| match character.is_control() {
            true => ' ',
            false => character,
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

struct Destinations<'a> {
    pages: HashMap<ObjectIdentifier, usize>,
    names: HashMap<Vec<u8>, Object<'a>>,
}

impl<'a> Destinations<'a> {
    fn new(pdf: &'a Pdf, root: &Dict<'a>) -> Self {
        let pages = pdf
            .pages()
            .iter()
            .enumerate()
            .filter_map(|(index, page)| Some((page.raw().obj_id()?, index)))
            .collect();
        let mut names = HashMap::new();
        if let Some(old) = root.get::<Dict<'a>>("Dests") {
            for (name, value) in old.entries() {
                if let Some(value) = value.as_obj_ref().and_then(|id| pdf.xref().get(id.into())) {
                    names.insert(name.as_str().as_bytes().to_vec(), value);
                }
            }
        }
        if let Some(tree) = root
            .get::<Dict<'a>>("Names")
            .and_then(|names| names.get::<Dict<'a>>("Dests"))
        {
            collect_names(&tree, &mut names, 0);
        }
        Self { pages, names }
    }

    fn page(&self, destination: Object<'a>, hops: u8) -> Option<usize> {
        match destination {
            Object::Array(array) => self.page_of_array(&array),
            Object::Name(name) if hops < 2 => {
                let target = self.names.get(name.as_str().as_bytes())?.clone();
                self.page(target, hops + 1)
            }
            Object::String(string) if hops < 2 => {
                let target = self.names.get(string.as_bytes())?.clone();
                self.page(target, hops + 1)
            }
            Object::Dict(dict) if hops < 2 => self.page(dict.get::<Object<'a>>("D")?, hops + 1),
            _ => None,
        }
    }

    fn page_of_array(&self, array: &Array<'a>) -> Option<usize> {
        let first = array.raw_iter().next()?;
        let id: ObjectIdentifier = first.as_obj_ref()?.into();
        self.pages.get(&id).copied()
    }
}

fn collect_names<'a>(node: &Dict<'a>, names: &mut HashMap<Vec<u8>, Object<'a>>, depth: usize) {
    if depth > MAX_TREE_DEPTH || names.len() >= MAX_NAMES {
        return;
    }
    if let Some(pairs) = node.get::<Array<'a>>("Names") {
        let mut items = pairs.iter::<Object<'a>>();
        while let (Some(key), Some(value)) = (items.next(), items.next()) {
            let key = match key {
                Object::String(string) => string.as_bytes().to_vec(),
                Object::Name(name) => name.as_str().as_bytes().to_vec(),
                _ => continue,
            };
            names.insert(key, value);
            if names.len() >= MAX_NAMES {
                return;
            }
        }
    }
    if let Some(kids) = node.get::<Array<'a>>("Kids") {
        for kid in kids.iter::<Dict<'a>>() {
            collect_names(&kid, names, depth + 1);
        }
    }
}

fn destination<'a>(item: &Dict<'a>) -> Option<Object<'a>> {
    if let Some(destination) = item.get::<Object<'a>>("Dest") {
        return Some(destination);
    }
    let action = item.get::<Dict<'a>>("A")?;
    let kind = action.get::<Name<'a>>("S")?;
    (kind.as_str() == "GoTo")
        .then(|| action.get::<Object<'a>>("D"))
        .flatten()
}

pub fn outline(pdf: &Pdf) -> Vec<Heading> {
    let xref = pdf.xref();
    let Some(root) = xref.get::<Dict<'_>>(xref.root_id()) else {
        return Vec::new();
    };
    let Some(first) = root
        .get::<Dict<'_>>("Outlines")
        .and_then(|outlines| outlines.get::<Dict<'_>>("First"))
    else {
        return Vec::new();
    };
    let destinations = Destinations::new(pdf, &root);
    let mut headings = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![(first, 0u8)];
    while let Some((item, level)) = stack.pop() {
        if headings.len() >= MAX_HEADINGS {
            break;
        }
        if let Some(id) = item.obj_id()
            && !seen.insert(id)
        {
            continue;
        }
        let title = item
            .get::<PdfString<'_>>("Title")
            .map(|title| text(title.as_bytes()))
            .unwrap_or_default();
        let page = destination(&item).and_then(|target| destinations.page(target, 0));
        if let Some(next) = item.get::<Dict<'_>>("Next") {
            stack.push((next, level));
        }
        if level < MAX_DEPTH
            && let Some(child) = item.get::<Dict<'_>>("First")
        {
            stack.push((child, level + 1));
        }
        headings.push(Heading { title, level, page });
    }
    headings
}

pub fn details(pdf: &Pdf) -> Vec<(String, String)> {
    let metadata = pdf.metadata();
    [
        ("title", &metadata.title),
        ("author", &metadata.author),
        ("subject", &metadata.subject),
        ("creator", &metadata.creator),
        ("producer", &metadata.producer),
    ]
    .into_iter()
    .filter_map(|(key, value)| {
        let value = text(value.as_deref()?);
        (!value.is_empty()).then(|| (key.to_owned(), value))
    })
    .collect()
}
