use std::collections::HashMap;
use std::path::Path;

use bande_xdg::Locale;
use serde::Deserialize;

use crate::component::{
    APPLICATION_KINDS, Bundle, Component, Icon, Release, Screenshot, THUMBNAIL_WIDTH,
};
use crate::lang::best;
use crate::xml::description_blocks;

const UNTRANSLATED: &str = "C";

type Localized<T> = HashMap<String, T>;

#[derive(Debug, Deserialize)]
struct Image {
    #[serde(alias = "name")]
    url: String,
    width: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct Icons {
    #[serde(default)]
    stock: Option<String>,
    #[serde(default)]
    cached: Vec<Image>,
    #[serde(default)]
    remote: Vec<Image>,
}

#[derive(Debug, Deserialize)]
struct Shot {
    #[serde(default)]
    caption: Localized<String>,
    #[serde(default)]
    thumbnails: Vec<Image>,
    #[serde(rename = "source-image")]
    source_image: Option<Image>,
}

#[derive(Debug, Deserialize)]
struct Developer {
    #[serde(default)]
    name: Localized<String>,
}

#[derive(Debug, Deserialize)]
struct DebRelease {
    version: String,
    #[serde(rename = "unix-timestamp")]
    timestamp: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Document {
    #[serde(rename = "Type")]
    kind: Option<String>,
    #[serde(rename = "ID")]
    id: Option<String>,
    origin: Option<String>,
    #[serde(rename = "MediaBaseUrl")]
    media_base_url: Option<String>,
    package: Option<String>,
    #[serde(default)]
    name: Localized<String>,
    #[serde(default)]
    summary: Localized<String>,
    #[serde(default)]
    description: Localized<String>,
    developer: Option<Developer>,
    #[serde(default)]
    developer_name: Localized<String>,
    project_license: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    keywords: Localized<Vec<String>>,
    #[serde(default)]
    icon: Icons,
    #[serde(default)]
    screenshots: Vec<Shot>,
    #[serde(default)]
    launchable: HashMap<String, Vec<String>>,
    #[serde(default)]
    releases: Vec<DebRelease>,
}

fn pick<T: Clone>(values: &Localized<T>, locale: Option<&Locale>) -> Option<T> {
    let chosen = best(
        values
            .keys()
            .map(|key| (key != UNTRANSLATED).then_some(key.as_str())),
        locale,
    );
    values
        .get(chosen.as_deref().unwrap_or(UNTRANSLATED))
        .cloned()
}

struct Header {
    icon_root: std::path::PathBuf,
    media: String,
}

fn media(header: &Header, url: &str) -> String {
    if url.contains("://") {
        url.to_owned()
    } else {
        format!("{}/{url}", header.media.trim_end_matches('/'))
    }
}

fn component(document: Document, header: &Header, locale: Option<&Locale>) -> Option<Component> {
    if !APPLICATION_KINDS.contains(&document.kind.as_deref()?) {
        return None;
    }
    let id = document.id?;
    let mut icons: Vec<Icon> = document
        .icon
        .cached
        .iter()
        .map(|icon| {
            let size = icon.width.unwrap_or(0);
            Icon::Cached {
                path: header
                    .icon_root
                    .join(format!("{size}x{size}"))
                    .join(&icon.url),
                size,
            }
        })
        .collect();
    icons.extend(document.icon.stock.map(Icon::Stock));
    icons.extend(document.icon.remote.iter().map(|icon| Icon::Remote {
        url: media(header, &icon.url),
        size: icon.width.unwrap_or(0),
    }));
    Some(Component {
        name: pick(&document.name, locale).unwrap_or_else(|| id.clone()),
        summary: pick(&document.summary, locale).unwrap_or_default(),
        description: pick(&document.description, locale)
            .map(|markup| description_blocks(&markup))
            .unwrap_or_default(),
        developer: document
            .developer
            .and_then(|developer| pick(&developer.name, locale))
            .or_else(|| pick(&document.developer_name, locale)),
        license: document.project_license,
        categories: document.categories,
        keywords: pick(&document.keywords, locale).unwrap_or_default(),
        icons,
        screenshots: document
            .screenshots
            .into_iter()
            .filter_map(|shot| {
                Some(Screenshot {
                    image: media(header, &shot.source_image?.url),
                    thumbnail: shot
                        .thumbnails
                        .iter()
                        .filter(|thumbnail| thumbnail.width.unwrap_or(0) <= THUMBNAIL_WIDTH)
                        .max_by_key(|thumbnail| thumbnail.width)
                        .map(|thumbnail| media(header, &thumbnail.url)),
                    caption: pick(&shot.caption, locale),
                })
            })
            .collect(),
        launchable: document
            .launchable
            .get("desktop-id")
            .and_then(|ids| ids.first().cloned()),
        releases: document
            .releases
            .into_iter()
            .map(|release| Release {
                version: release.version,
                timestamp: release.timestamp,
            })
            .collect(),
        bundles: document.package.map(Bundle::Deb).into_iter().collect(),
        needs_x11: false,
        id,
    })
}

pub fn parse(yaml: &str, icon_root: &Path, locale: Option<&Locale>) -> Vec<Component> {
    let mut header = Header {
        icon_root: icon_root.to_path_buf(),
        media: String::new(),
    };
    let mut components = Vec::new();
    for raw in serde_yaml_ng::Deserializer::from_str(yaml) {
        let document = match Document::deserialize(raw) {
            Ok(document) => document,
            Err(error) => {
                tracing::debug!(%error, "skipping DEP-11 document");
                continue;
            }
        };
        if document.kind.is_none() {
            if let Some(origin) = &document.origin {
                header.icon_root = icon_root.join(origin);
            }
            header.media = document.media_base_url.unwrap_or_default();
            continue;
        }
        components.extend(component(document, &header, locale));
    }
    components
}
