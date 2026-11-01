use std::path::PathBuf;

use bande_xdg::Locale;
use roxmltree::{Document, Node};

use crate::component::{APPLICATION_KINDS, Block, Bundle, Component, Icon, Release, Screenshot};
use crate::lang::best;

const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub remote: Option<String>,
    pub icon_root: PathBuf,
}

fn lang<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    node.attribute((XML_NAMESPACE, "lang"))
}

fn children<'a, 'input>(
    node: Node<'a, 'input>,
    name: &'static str,
) -> impl Iterator<Item = Node<'a, 'input>> {
    node.children()
        .filter(move |child| child.is_element() && child.has_tag_name(name))
}

fn text(node: Node) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|text| text.text())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn localized<'a, 'input>(
    nodes: impl Iterator<Item = Node<'a, 'input>>,
    locale: Option<&Locale>,
) -> Vec<Node<'a, 'input>> {
    let nodes: Vec<_> = nodes.collect();
    let chosen = best(nodes.iter().map(|node| lang(*node)), locale);
    nodes
        .into_iter()
        .filter(|node| lang(*node) == chosen.as_deref())
        .collect()
}

fn localized_text(node: Node, name: &'static str, locale: Option<&Locale>) -> Option<String> {
    localized(children(node, name), locale)
        .first()
        .map(|node| text(*node))
}

fn description(node: Node, locale: Option<&Locale>) -> Vec<Block> {
    let descriptions = localized(children(node, "description"), locale);
    let Some(description) = descriptions.first() else {
        return Vec::new();
    };
    let blocks: Vec<_> = description
        .children()
        .filter(|child| {
            child.has_tag_name("p") || child.has_tag_name("ul") || child.has_tag_name("ol")
        })
        .collect();
    let chosen = best(
        blocks.iter().flat_map(|block| {
            if block.has_tag_name("p") {
                vec![lang(*block)]
            } else {
                children(*block, "li").map(lang).collect()
            }
        }),
        locale,
    );
    blocks
        .into_iter()
        .filter_map(|block| {
            if block.has_tag_name("p") {
                (lang(block) == chosen.as_deref()).then(|| Block::Paragraph(text(block)))
            } else {
                let items: Vec<_> = children(block, "li")
                    .filter(|item| lang(*item) == chosen.as_deref())
                    .map(text)
                    .collect();
                (!items.is_empty()).then_some(Block::List(items))
            }
        })
        .collect()
}

fn size(node: Node) -> u32 {
    node.attribute("width")
        .or_else(|| node.attribute("height"))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn icon(node: Node, source: &Source) -> Option<Icon> {
    let value = text(node);
    if value.is_empty() {
        return None;
    }
    let size = size(node);
    match node.attribute("type")? {
        "stock" => Some(Icon::Stock(value)),
        "cached" => Some(Icon::Cached {
            path: source.icon_root.join(format!("{size}x{size}")).join(value),
            size,
        }),
        "local" => Some(Icon::Local(PathBuf::from(value))),
        "remote" => Some(Icon::Remote { url: value, size }),
        _ => None,
    }
}

fn screenshot(node: Node, locale: Option<&Locale>) -> Option<Screenshot> {
    let images: Vec<_> = children(node, "image").collect();
    let of_type = |kind: &str| {
        images
            .iter()
            .filter(|image| image.attribute("type") == Some(kind))
            .max_by_key(|image| size(**image))
            .map(|image| text(*image))
    };
    Some(Screenshot {
        image: of_type("source")?,
        thumbnail: of_type("thumbnail"),
        caption: localized_text(node, "caption", locale),
    })
}

fn bundles(node: Node, source: &Source) -> Vec<Bundle> {
    let flatpaks = children(node, "bundle")
        .filter(|bundle| bundle.attribute("type") == Some("flatpak"))
        .filter_map(|bundle| {
            Some(Bundle::Flatpak {
                remote: source.remote.clone()?,
                reference: text(bundle),
            })
        });
    let debs = children(node, "pkgname").map(|package| Bundle::Deb(text(package)));
    flatpaks.chain(debs).collect()
}

fn developer(node: Node, locale: Option<&Locale>) -> Option<String> {
    children(node, "developer")
        .find_map(|developer| localized_text(developer, "name", locale))
        .or_else(|| localized_text(node, "developer_name", locale))
}

fn component(node: Node, source: &Source, locale: Option<&Locale>) -> Option<Component> {
    if !APPLICATION_KINDS.contains(&node.attribute("type")?) {
        return None;
    }
    let id = children(node, "id").next().map(text)?;
    let keywords = localized(
        children(node, "keywords").flat_map(|list| children(list, "keyword")),
        locale,
    );
    Some(Component {
        name: localized_text(node, "name", locale).unwrap_or_else(|| id.clone()),
        summary: localized_text(node, "summary", locale).unwrap_or_default(),
        description: description(node, locale),
        developer: developer(node, locale),
        license: children(node, "project_license").next().map(text),
        categories: children(node, "categories")
            .flat_map(|list| children(list, "category"))
            .map(text)
            .collect(),
        keywords: keywords.into_iter().map(text).collect(),
        icons: children(node, "icon")
            .filter_map(|element| icon(element, source))
            .collect(),
        screenshots: children(node, "screenshots")
            .flat_map(|list| children(list, "screenshot"))
            .filter_map(|element| screenshot(element, locale))
            .collect(),
        launchable: children(node, "launchable")
            .find(|launchable| launchable.attribute("type") == Some("desktop-id"))
            .map(text),
        releases: children(node, "releases")
            .flat_map(|list| children(list, "release"))
            .filter_map(|release| {
                Some(Release {
                    version: release.attribute("version")?.to_owned(),
                    timestamp: release
                        .attribute("timestamp")
                        .and_then(|value| value.parse().ok()),
                })
            })
            .collect(),
        bundles: bundles(node, source),
        id,
    })
}

pub fn parse(
    xml: &str,
    source: &Source,
    locale: Option<&Locale>,
) -> Result<Vec<Component>, roxmltree::Error> {
    let document = Document::parse(xml)?;
    let root = document.root_element();
    let source = match (root.attribute("origin"), &source.remote) {
        (Some(origin), None) => Source {
            remote: None,
            icon_root: source.icon_root.join(origin),
        },
        _ => source.clone(),
    };
    let components = if root.has_tag_name("component") {
        vec![root]
    } else {
        children(root, "component").collect()
    };
    Ok(components
        .into_iter()
        .filter_map(|node| component(node, &source, locale))
        .collect())
}
