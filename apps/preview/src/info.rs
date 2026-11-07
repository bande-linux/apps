use std::path::Path;

use accesskit::{Node, NodeId, Role};
use bande_toolkit::ui::{Context, Parts, Size, Ui, Widget, WidgetId};
use bande_toolkit::{
    Appearance, Item, Paragraph, Point, Rect, TextAlign, TextStyle, TextSystem,
    WINDOW_HEADER_HEIGHT,
};

use crate::{L10N, tr_with};
use bande_content::pdf::protocol::Opened;
use bande_content::{Decoded, Kind};

const MARGIN: f32 = 18.0;
const LABEL_WIDTH: f32 = 112.0;
const COLUMN_GAP: f32 = 10.0;
const ROW_GAP: f32 = 7.0;
const SECTION_GAP: f32 = 14.0;

pub type Row = (String, String);

fn kind_name(kind: Kind) -> String {
    L10N.tr(match kind {
        Kind::Png => "kind-png",
        Kind::Jpeg => "kind-jpeg",
        Kind::Gif => "kind-gif",
        Kind::Webp => "kind-webp",
        Kind::Svg => "kind-svg",
        Kind::Pdf => "kind-pdf",
    })
}

pub fn file_rows(path: &Path) -> Vec<Row> {
    let mut rows = vec![(
        L10N.tr("info-name"),
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    )];
    if let Some(folder) = path.parent() {
        rows.push((L10N.tr("info-where"), folder.display().to_string()));
    }
    let metadata = match bande_volumes::on_connected_volume(path) {
        true => Err(std::io::ErrorKind::Unsupported.into()),
        false => std::fs::metadata(path),
    };
    if let Ok(metadata) = metadata {
        rows.push((L10N.tr("info-size"), bande_fs::format_size(metadata.len())));
        if let Ok(created) = metadata.created() {
            rows.push((L10N.tr("info-created"), bande_fs::format_list_date(created)));
        }
        if let Ok(modified) = metadata.modified() {
            rows.push((
                L10N.tr("info-modified"),
                bande_fs::format_list_date(modified),
            ));
        }
    }
    rows
}

pub fn picture_rows(decoded: &Decoded) -> Vec<Row> {
    let facts = &decoded.facts;
    let mut rows = Vec::new();
    if let Some(kind) = facts.kind {
        rows.push((L10N.tr("info-kind"), kind_name(kind)));
    }
    let (width, height) = facts.natural.unwrap_or((decoded.width, decoded.height));
    rows.push((
        L10N.tr("info-dimensions"),
        tr_with(
            "dimensions",
            &[("width", width.to_string()), ("height", height.to_string())],
        ),
    ));
    if let Some((count, complete)) = facts.frames {
        let key = if complete { "frames" } else { "frames-first" };
        rows.push((
            L10N.tr("info-frames"),
            tr_with(key, &[("count", count.to_string())]),
        ));
    }
    rows.extend(
        facts
            .camera
            .iter()
            .map(|(label, value)| (L10N.tr(label), value.clone())),
    );
    rows
}

pub fn document_rows(opened: &Opened) -> Vec<Row> {
    let mut rows = vec![
        (L10N.tr("info-kind"), kind_name(Kind::Pdf)),
        (L10N.tr("info-pages"), opened.pages.len().to_string()),
    ];
    if let Some((width, height)) = opened.pages.first() {
        rows.push((
            L10N.tr("info-page-size"),
            tr_with(
                "page-size",
                &[
                    ("width", format!("{width:.0}")),
                    ("height", format!("{height:.0}")),
                ],
            ),
        ));
    }
    if !opened.version.is_empty() {
        rows.push((L10N.tr("info-version"), opened.version.clone()));
    }
    rows.extend(
        opened
            .details
            .iter()
            .map(|(key, value)| (L10N.tr(&format!("info-{key}")), value.clone())),
    );
    rows
}

pub struct InfoPanel {
    id: WidgetId,
    frame: Rect,
    sections: Vec<Vec<Row>>,
    laid_out: Vec<(Point, Paragraph)>,
    width: f32,
}

impl InfoPanel {
    pub fn new(sections: Vec<Vec<Row>>) -> Self {
        Self {
            id: WidgetId::next(),
            frame: Rect::default(),
            sections,
            laid_out: Vec::new(),
            width: 0.0,
        }
    }

    pub fn set_sections(&mut self, sections: Vec<Vec<Row>>) {
        self.sections = sections;
        self.width = 0.0;
    }

    fn lay_out(&mut self, cx: &mut Context) {
        self.laid_out.clear();
        let value_width = (self.frame.width - 2.0 * MARGIN - LABEL_WIDTH - COLUMN_GAP).max(40.0);
        let label_style = TextStyle::new(12.0, cx.palette.ink2);
        let value_style = TextStyle::new(12.0, cx.palette.ink);
        let mut y = self.frame.y + WINDOW_HEADER_HEIGHT as f32 + MARGIN / 2.0;
        for section in &self.sections {
            for (label, value) in section {
                let label =
                    cx.text
                        .paragraph(label, label_style, Some(LABEL_WIDTH), TextAlign::End);
                let value =
                    cx.text
                        .paragraph(value, value_style, Some(value_width), TextAlign::Start);
                let height = label.height().max(value.height());
                self.laid_out
                    .push((Point::new(self.frame.x + MARGIN, y), label));
                self.laid_out.push((
                    Point::new(self.frame.x + MARGIN + LABEL_WIDTH + COLUMN_GAP, y),
                    value,
                ));
                y += height + ROW_GAP;
            }
            y += SECTION_GAP;
        }
    }
}

impl Widget for InfoPanel {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        let moved = frame != self.frame;
        self.frame = frame;
        if moved || self.width != frame.width {
            self.width = frame.width;
            self.lay_out(cx);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, items: &mut Vec<Item>) {
        items.extend(self.laid_out.iter().map(|(origin, paragraph)| Item::Text {
            origin: *origin,
            paragraph: paragraph.clone(),
        }));
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::List)
    }

    fn parts(&self, _cx: &Context) -> Parts {
        let nodes: Vec<(NodeId, Node)> = self
            .sections
            .iter()
            .flatten()
            .enumerate()
            .map(|(index, (label, value))| {
                let mut node = Node::new(Role::ListItem);
                node.set_label(format!("{label}: {value}"));
                (self.id.part(index as u64), node)
            })
            .collect();
        Parts {
            children: nodes.iter().map(|(id, _)| *id).collect(),
            focus: None,
            nodes,
        }
    }
}

pub fn info_ui(appearance: Appearance, sections: Vec<Vec<Row>>) -> (Ui, WidgetId) {
    let panel = InfoPanel::new(sections);
    let id = panel.id();
    let mut ui = Ui::new(
        Box::new(panel),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    (ui, id)
}
