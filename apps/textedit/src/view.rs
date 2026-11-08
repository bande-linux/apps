use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use bande_app::View;
use bande_inbox::Outbox;
use bande_toolkit::ui::{
    Alert, Alignment, BUTTON_HEIGHT, Button, CheckState, Checkbox, Context, Event, FreeLayout, Key,
    Label, ListMarker, Modifiers, ParagraphAttributes, RichText, SearchField, TEXT_FIELD_HEIGHT,
    TextAttributes, TextField, TextView, Ui, Widget, WidgetId,
};
use bande_toolkit::{
    Appearance, Color, Rect, Scene, TextAlign, TextStyle, TextSystem, Window, WindowEvent,
};

use crate::L10N;
use crate::document::{Content, Format, Loaded, encode, read};
use crate::encoding::{LineEnding, TextEncoding};
use crate::file::{MAX_FILE_BYTES, read_bounded, write_atomically};
use crate::find::{self, Query};
use crate::formats::{heading, markdown, rtf};
use crate::session::seconds_now;
use crate::session::{Record, Session};
use crate::worker::{Message, WriteError, spawn};

const SETTLE: Duration = Duration::from_millis(1500);
const FIND_ROW: f32 = 40.0;
const STATUS_HEIGHT: f32 = 24.0;
const MARGIN: f32 = 8.0;
const GAP: f32 = 6.0;
const CONTROL_PADDING: f32 = 28.0;
const FIELD_WIDTH: f32 = 220.0;
const PAGE_WIDTH: f32 = 640.0;
pub const MONOSPACE_SIZE: f32 = 13.0;
pub const PROPORTIONAL_SIZE: f32 = 14.0;
const MIN_SIZE: f32 = 8.0;
const MAX_SIZE: f32 = 72.0;
const SIZE_STEP: f32 = 2.0;
const MAX_LEVEL: u8 = 8;
pub const FAMILIES: [(&str, Option<&str>); 4] = [
    ("default", None),
    ("serif", Some("Noto Serif")),
    ("display", Some("Manrope")),
    ("monospace", Some(crate::formats::MONOSPACE)),
];
pub const COLORS: [(&str, Option<u32>); 8] = [
    ("default", None),
    ("red", Some(0xD7263D)),
    ("orange", Some(0xE8710A)),
    ("green", Some(0x2E8B57)),
    ("blue", Some(0x1F6FD1)),
    ("purple", Some(0x7B3FB5)),
    ("brown", Some(0x8B5A2B)),
    ("gray", Some(0x7A7A7A)),
];

#[derive(Debug, Clone, PartialEq)]
enum Signal {
    Query(String),
    Replacement(String),
    Case(bool),
    Regex(bool),
    Next,
    Previous,
    Replace,
    ReplaceAll,
    HideFind,
    Choice(String),
    Line(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asking {
    SaveChanges,
    SaveAsUnicode,
    GoToLine,
    Failed,
    MakePlain,
    Lossy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Close,
    SaveAs,
    Remember(PathBuf),
}

struct FindBar {
    replacing: bool,
    query: WidgetId,
    case: WidgetId,
    regex: WidgetId,
    previous: WidgetId,
    next: WidgetId,
    done: WidgetId,
    replacement: WidgetId,
    replace: WidgetId,
    replace_all: WidgetId,
    message: WidgetId,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub monospace: bool,
    pub page: bool,
    pub size: f32,
}

impl Settings {
    pub const PLAIN: Self = Self {
        monospace: true,
        page: false,
        size: MONOSPACE_SIZE,
    };
    pub const RICH: Self = Self {
        monospace: false,
        page: false,
        size: PROPORTIONAL_SIZE,
    };
}

pub struct Opening {
    pub token: u64,
    pub id: String,
    pub untitled: String,
    pub opened: u128,
    pub settings: Settings,
    pub format: Format,
}

pub struct DocumentView {
    ui: Ui,
    host: WidgetId,
    editor: WidgetId,
    status: WidgetId,
    status_text: String,
    find: Option<FindBar>,
    query: Query,
    replacement: String,
    find_message: String,
    signals: Rc<RefCell<Vec<Signal>>>,
    asking: Option<Asking>,
    line: String,
    pub token: u64,
    pub id: String,
    pub path: Option<PathBuf>,
    pub untitled: String,
    pub opened: u128,
    pub encoding: TextEncoding,
    pub line_ending: LineEnding,
    pub settings: Settings,
    pub format: Format,
    pub rich: bool,
    lossy: bool,
    pub loading: bool,
    saved: Option<u64>,
    format_changed: bool,
    seen: u64,
    settle_at: Option<Instant>,
    stored: Option<(u64, bool)>,
    counts: Option<(usize, usize)>,
    counted: u64,
    unloaded: bool,
    shown_message: String,
    note: Option<String>,
    snapshot: Option<(u64, Arc<String>)>,
    saving: bool,
    close_after_save: bool,
    requests: Vec<Request>,
    outbox: Outbox<Message>,
    session: Option<Session>,
    versions: Vec<u64>,
}

fn editor_of(ui: &mut Ui, id: WidgetId) -> Option<&mut TextView> {
    ui.widget_as::<TextView>(id)
}

impl DocumentView {
    pub fn new(
        appearance: Appearance,
        opening: Opening,
        outbox: Outbox<Message>,
        session: Option<Session>,
    ) -> Self {
        let Opening {
            token,
            id,
            untitled,
            opened,
            settings,
            format,
        } = opening;
        let mut host = FreeLayout::new();
        let host_id = Widget::id(&host);
        let editor = host.insert(TextView::document("").borderless(), Rect::default());
        let status = host.insert(Label::new("").secondary(), Rect::default());
        let mut ui = Ui::new(
            Box::new(host),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        ui.set_focus(editor);
        let mut view = Self {
            ui,
            host: host_id,
            editor,
            status,
            status_text: String::new(),
            find: None,
            query: Query::default(),
            replacement: String::new(),
            find_message: String::new(),
            signals: Rc::new(RefCell::new(Vec::new())),
            asking: None,
            line: String::new(),
            token,
            id,
            path: None,
            untitled,
            opened,
            encoding: TextEncoding::UTF8,
            line_ending: LineEnding::Lf,
            settings,
            format,
            rich: format.is_rich(),
            lossy: false,
            loading: false,
            saved: None,
            format_changed: false,
            seen: 0,
            settle_at: None,
            stored: None,
            counts: Some((0, 0)),
            counted: 0,
            unloaded: false,
            shown_message: String::new(),
            note: None,
            snapshot: None,
            saving: false,
            close_after_save: false,
            requests: Vec::new(),
            outbox,
            session,
            versions: Vec::new(),
        };
        view.apply_settings();
        view.saved = Some(view.revision());
        view.seen = view.revision();
        view.counted = view.revision();
        view
    }

    fn editor(&mut self) -> Option<&mut TextView> {
        editor_of(&mut self.ui, self.editor)
    }

    fn revision(&self) -> u64 {
        self.ui
            .widget_ref::<TextView>(self.editor)
            .map_or(0, TextView::revision)
    }

    pub fn text(&mut self) -> Arc<String> {
        let revision = self.revision();
        if let Some((seen, text)) = &self.snapshot
            && *seen == revision
        {
            return Arc::clone(text);
        }
        let text = Arc::new(
            self.ui
                .widget_ref::<TextView>(self.editor)
                .map(TextView::text)
                .unwrap_or_default(),
        );
        self.snapshot = Some((revision, Arc::clone(&text)));
        text
    }

    fn content(&mut self) -> Content {
        match self.rich {
            true => Content::Rich(
                self.ui
                    .widget_ref::<TextView>(self.editor)
                    .map(TextView::rich_text)
                    .unwrap_or_default(),
            ),
            false => Content::Plain(self.text()),
        }
    }

    pub fn printable(&mut self) -> (String, bande_app::Document) {
        let document = match self.content() {
            Content::Rich(rich) => bande_app::rich_document(rich, false),
            Content::Plain(text) => bande_app::text_document(text.to_string(), true),
        };
        (self.name(), document)
    }

    pub fn name(&self) -> String {
        self.path.as_deref().and_then(Path::file_name).map_or_else(
            || self.untitled.clone(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    pub fn is_edited(&self) -> bool {
        self.saved != Some(self.revision()) || self.format_changed
    }

    pub fn is_empty_untitled(&self) -> bool {
        self.path.is_none()
            && self
                .ui
                .widget_ref::<TextView>(self.editor)
                .is_none_or(|editor| editor.text().is_empty())
    }

    pub fn title(&self) -> String {
        match self.is_edited() && !self.loading {
            true => L10N.tr_args("title-edited", [("name", self.name().into())]),
            false => self.name(),
        }
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn apply_settings(&mut self) {
        let Settings {
            monospace,
            page,
            size,
        } = self.settings;
        if let Some(editor) = self.editor() {
            editor.set_font(monospace, size);
            editor.set_page_width(page.then_some(PAGE_WIDTH));
        }
    }

    pub fn set_monospace(&mut self, monospace: bool) {
        self.settings.monospace = monospace;
        self.settings.size = match monospace {
            true => MONOSPACE_SIZE,
            false => PROPORTIONAL_SIZE,
        };
        self.apply_settings();
        self.persist();
    }

    pub fn set_page(&mut self, page: bool) {
        self.settings.page = page;
        self.apply_settings();
        self.persist();
    }

    pub fn zoom(&mut self, step: Option<f32>) {
        self.settings.size = match step {
            Some(step) => (self.settings.size + step).clamp(MIN_SIZE, MAX_SIZE),
            None => match self.settings.monospace {
                true => MONOSPACE_SIZE,
                false => PROPORTIONAL_SIZE,
            },
        };
        self.apply_settings();
        self.persist();
    }

    pub fn set_encoding(&mut self, encoding: TextEncoding) {
        if self.encoding != encoding {
            self.encoding = encoding;
            self.format_changed = true;
            self.persist();
        }
    }

    pub fn set_line_ending(&mut self, line_ending: LineEnding) {
        if self.line_ending != line_ending {
            self.line_ending = line_ending;
            self.format_changed = true;
            self.persist();
        }
    }

    pub fn record(&self) -> Record {
        Record {
            path: self.path.clone(),
            title: self.untitled.clone(),
            encoding: self.encoding.encoding.name().to_owned(),
            bom: self.encoding.bom,
            line_ending: self.line_ending.name().to_owned(),
            edited: self.is_edited(),
            monospace: self.settings.monospace,
            page: self.settings.page,
            size: self.settings.size,
            opened: self.opened,
            format: self.format.name().to_owned(),
            rich: self.rich,
        }
    }

    pub fn start_loading(&mut self, path: Option<PathBuf>) {
        self.path = path;
        self.loading = true;
    }

    pub fn loaded(&mut self, loaded: Loaded) {
        let Loaded {
            text,
            rich,
            format,
            encoding,
            line_ending,
            damaged,
            lossy,
            from_disk,
            words,
            characters,
        } = loaded;
        self.lossy = lossy && from_disk;
        self.loading = false;
        if from_disk {
            self.encoding = encoding;
            self.line_ending = line_ending;
            if self.rich != rich.is_some() {
                self.settings = match rich.is_some() {
                    true => Settings::RICH,
                    false => Settings::PLAIN,
                };
                self.apply_settings();
            }
            self.format = format;
        }
        self.note = damaged.then(|| {
            L10N.tr_args(
                "status-damaged",
                [("encoding", crate::encoding_name(encoding).into())],
            )
        });
        self.rich = rich.is_some();
        let text = match (rich, self.editor()) {
            (Some(rich), Some(editor)) => {
                editor.set_rich_text(&rich);
                rich.text
            }
            (None, Some(editor)) => {
                editor.set_text(&text);
                text
            }
            (_, None) => text,
        };
        let revision = self.revision();
        self.seen = revision;
        self.snapshot = Some((revision, Arc::new(text)));
        self.counts = Some((words, characters));
        self.counted = revision;
        self.unloaded = false;
        self.format_changed = false;
        self.saved = from_disk.then_some(revision);
        self.stored = Some((revision, !from_disk));
        self.settle_at = None;
        self.persist();
        self.list_versions();
    }

    pub fn failed(&mut self, error: &str) {
        self.loading = false;
        self.unloaded = self.saved.is_some() && self.revision() == self.counted;
        self.asking = Some(Asking::Failed);
        let signals = Rc::clone(&self.signals);
        self.ui.present_sheet(
            Alert::new(
                &L10N.tr_args("open-failed", [("name", self.name().into())]),
                error,
                move |action| signals.borrow_mut().push(Signal::Choice(action.to_owned())),
            )
            .primary(&L10N.tr("ok"), "ok"),
        );
    }

    fn persist(&mut self) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let revision = self.revision();
        let edited = self.is_edited();
        if self.loading {
            return;
        }
        let record = self.record();
        let id = self.id.clone();
        let text = (edited && self.stored != Some((revision, true))).then(|| self.content());
        self.stored = Some((revision, edited));
        let counted = (self.counted != revision).then(|| self.text());
        self.counted = revision;
        let (outbox, token) = (self.outbox.clone(), self.token);
        spawn("autosave", move || {
            let text = text.map(|content| autosaved(&content));
            if let Err(error) = session.store(&id, &record, text.as_deref()) {
                tracing::warn!("cannot autosave {}: {error}", record.title);
            }
            if let Some(text) = counted {
                outbox.send(Message::Counted {
                    token,
                    revision,
                    words: find::words(&text),
                    characters: text.chars().count(),
                });
            }
        });
    }

    pub fn flush(&mut self) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if self.loading {
            return;
        }
        let revision = self.revision();
        let edited = self.is_edited();
        if self.stored == Some((revision, edited)) && self.settle_at.is_none() {
            return;
        }
        let text = edited.then(|| autosaved(&self.content()));
        let record = self.record();
        if let Err(error) = session.store(&self.id, &record, text.as_deref()) {
            tracing::warn!("cannot autosave {}: {error}", record.title);
        }
        self.stored = Some((revision, edited));
    }

    pub fn forget(&self) {
        if let Some(session) = &self.session {
            session.forget(&self.id);
        }
    }

    pub fn counted(&mut self, revision: u64, words: usize, characters: usize) {
        if revision == self.revision() {
            self.counts = Some((words, characters));
        }
        self.counted = revision;
    }

    pub fn save(&mut self, close: bool) {
        self.close_after_save = close;
        if self.lossy && self.path.is_some() {
            return self.ask(
                Asking::Lossy,
                &L10N.tr_args("lossy-title", [("name", self.name().into())]),
                &L10N.tr("lossy-message"),
                &[("save-copy", true), ("cancel", false)],
            );
        }
        match self.path.clone() {
            Some(path) => self.save_to(path),
            None => self.requests.push(Request::SaveAs),
        }
    }

    pub fn cancel_save(&mut self) {
        self.close_after_save = false;
    }

    pub fn save_to(&mut self, path: PathBuf) {
        if self.saving {
            return;
        }
        self.saving = true;
        match (self.rich, Format::by_extension(&path)) {
            (true, Some(chosen))
                if chosen != Format::Markdown || self.format == Format::Markdown =>
            {
                self.format = chosen;
            }
            (false, Some(Format::Markdown)) => self.format = Format::Markdown,
            (false, _) if self.format != Format::Markdown => self.format = Format::Plain,
            _ => {}
        }
        let content = self.content();
        let revision = self.revision();
        let (encoding, line_ending, format) = (self.encoding, self.line_ending, self.format);
        let title = self.name();
        let (outbox, token) = (self.outbox.clone(), self.token);
        let session = self.session.clone();
        spawn("save", move || {
            let outcome =
                encode(&content, format, encoding, line_ending, &title).and_then(|bytes| {
                    if let Some(session) = &session
                        && let Ok(before) = read_bounded(&path, MAX_FILE_BYTES)
                        && before != bytes
                        && let Err(error) = session.keep_version(&path, &before, seconds_now())
                    {
                        tracing::warn!("cannot keep the old version: {error}");
                    }
                    write_atomically(&path, &bytes)
                        .map_err(|error| WriteError::Io(error.to_string()))
                });
            outbox.send(Message::Written {
                token,
                revision,
                path,
                outcome,
            });
        });
    }

    pub fn versions(&self) -> &[u64] {
        &self.versions
    }

    pub fn set_versions(&mut self, versions: Vec<u64>) {
        self.versions = versions;
    }

    fn list_versions(&self) {
        let (Some(session), Some(path)) = (self.session.clone(), self.path.clone()) else {
            return;
        };
        let (outbox, token) = (self.outbox.clone(), self.token);
        spawn("versions", move || {
            let versions = session.versions(&path);
            outbox.send(Message::Versions { token, versions });
        });
    }

    pub fn revert_to(&mut self, version: u64) {
        let (Some(session), Some(path)) = (self.session.clone(), self.path.clone()) else {
            return;
        };
        let edited = self.is_edited().then(|| {
            (
                self.content(),
                self.format,
                self.encoding,
                self.line_ending,
                self.name(),
            )
        });
        self.start_loading(Some(path.clone()));
        let kept = self.format;
        let (outbox, token) = (self.outbox.clone(), self.token);
        spawn("revert-to", move || {
            if let Some((content, format, encoding, line_ending, title)) = edited
                && let Ok(bytes) = encode(&content, format, encoding, line_ending, &title)
                && let Err(error) = session.keep_version(&path, &bytes, seconds_now())
            {
                tracing::warn!("cannot keep the edited version: {error}");
            }
            let loaded = session
                .version(&path, version)
                .map_err(|error| error.to_string())
                .and_then(|bytes| read(bytes, kept, None))
                .map(|loaded| Loaded {
                    from_disk: false,
                    ..loaded
                });
            outbox.send(Message::Loaded { token, loaded });
        });
    }

    pub fn written(&mut self, revision: u64, path: PathBuf, outcome: Result<(), WriteError>) {
        self.saving = false;
        match outcome {
            Ok(()) => {
                self.lossy &= self.path.as_ref() == Some(&path);
                self.path = Some(path.clone());
                self.saved = Some(revision);
                self.format_changed = false;
                self.persist();
                self.list_versions();
                self.requests.push(Request::Remember(path));
                if std::mem::take(&mut self.close_after_save) {
                    self.requests.push(Request::Close);
                }
            }
            Err(WriteError::Unmappable) => {
                self.ask(
                    Asking::SaveAsUnicode,
                    &L10N.tr_args(
                        "unmappable-title",
                        [("encoding", crate::encoding_name(self.encoding).into())],
                    ),
                    &L10N.tr("unmappable-message"),
                    &[("save-unicode", true), ("cancel", false)],
                );
            }
            Err(WriteError::Io(error)) => {
                self.close_after_save = false;
                self.ask(
                    Asking::Failed,
                    &L10N.tr_args("save-failed", [("name", self.name().into())]),
                    &error,
                    &[("ok", true)],
                );
            }
        }
    }

    pub fn ask_to_save(&mut self) {
        self.ask(
            Asking::SaveChanges,
            &L10N.tr_args("save-changes-title", [("name", self.name().into())]),
            &L10N.tr("save-changes-message"),
            &[("save", true), ("dont-save", false), ("cancel", false)],
        );
    }

    fn ask(&mut self, asking: Asking, title: &str, message: &str, buttons: &[(&str, bool)]) {
        if self.ui.has_sheet() {
            return;
        }
        self.asking = Some(asking);
        let signals = Rc::clone(&self.signals);
        let mut alert = Alert::new(title, message, move |action| {
            signals.borrow_mut().push(Signal::Choice(action.to_owned()));
        });
        for (action, primary) in buttons {
            let label = L10N.tr(action);
            alert = match (*primary, *action) {
                (true, _) => alert.primary(&label, action),
                (false, "cancel") => alert.cancel(&label, action),
                (false, _) => alert.other(&label, action),
            };
        }
        self.ui.present_sheet(alert);
    }

    pub fn go_to_line(&mut self) {
        if self.ui.has_sheet() {
            return;
        }
        self.asking = Some(Asking::GoToLine);
        self.line.clear();
        let (typed, chosen) = (Rc::clone(&self.signals), Rc::clone(&self.signals));
        let field = TextField::new(&L10N.tr("line-placeholder"), "", move |text| {
            typed.borrow_mut().push(Signal::Line(text.to_owned()));
        })
        .without_text_checking();
        self.ui.present_sheet(
            Alert::new(&L10N.tr("go-to-line"), "", move |action| {
                chosen.borrow_mut().push(Signal::Choice(action.to_owned()));
            })
            .accessory(field)
            .cancel(&L10N.tr("cancel"), "cancel")
            .primary(&L10N.tr("go"), "go"),
        );
    }

    fn chosen(&mut self, action: &str) {
        match (self.asking.take(), action) {
            (Some(Asking::SaveChanges), "save") => self.save(true),
            (Some(Asking::SaveChanges), "dont-save") => self.requests.push(Request::Close),
            (Some(Asking::SaveAsUnicode), "save-unicode") => {
                self.encoding = TextEncoding::UTF8;
                self.format_changed = true;
                match self.path.clone() {
                    Some(path) => self.save_to(path),
                    None => self.close_after_save = false,
                }
            }
            (Some(Asking::GoToLine), "go") => {
                let text = self.text();
                let range = self
                    .line
                    .trim()
                    .parse()
                    .ok()
                    .and_then(|line| find::line_range(&text, line));
                if let (Some(range), Some(editor)) = (range, self.editor()) {
                    editor.select(range);
                }
                self.ui.set_focus(self.editor);
            }
            (Some(Asking::Failed), _) if self.unloaded => self.requests.push(Request::Close),
            (Some(Asking::MakePlain), "make-plain-confirm") => self.make_plain(true),
            (Some(Asking::Lossy), "save-copy") => {
                self.lossy = false;
                self.retarget(Format::Rtf);
                self.changed_format();
                self.requests.push(Request::SaveAs);
            }
            _ => self.close_after_save = false,
        }
    }

    pub fn show_find(&mut self, replacing: bool) {
        if let Some(find) = &mut self.find {
            find.replacing |= replacing;
            let query = find.query;
            self.ui.set_focus(query);
            return;
        }
        let signals = &self.signals;
        let push = |signal: fn(String) -> Signal| {
            let signals = Rc::clone(signals);
            move |text: &str| signals.borrow_mut().push(signal(text.to_owned()))
        };
        let toggle = |signal: fn(bool) -> Signal| {
            let signals = Rc::clone(signals);
            move |on: bool| signals.borrow_mut().push(signal(on))
        };
        let click = |signal: Signal| {
            let signals = Rc::clone(signals);
            move || signals.borrow_mut().push(signal.clone())
        };
        let state = |on: bool| match on {
            true => CheckState::On,
            false => CheckState::Off,
        };
        let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) else {
            return;
        };
        let at = Rect::default();
        let bar = FindBar {
            replacing,
            query: host.insert(
                SearchField::new(
                    &L10N.tr("find-placeholder"),
                    &self.query.pattern,
                    push(Signal::Query),
                ),
                at,
            ),
            case: host.insert(
                Checkbox::new(
                    &L10N.tr("match-case"),
                    state(self.query.case),
                    toggle(Signal::Case),
                ),
                at,
            ),
            regex: host.insert(
                Checkbox::new(
                    &L10N.tr("regular-expression"),
                    state(self.query.regex),
                    toggle(Signal::Regex),
                ),
                at,
            ),
            previous: host.insert(
                Button::new("‹", click(Signal::Previous)).tooltip(&L10N.tr("find-previous")),
                at,
            ),
            next: host.insert(
                Button::new("›", click(Signal::Next)).tooltip(&L10N.tr("find-next")),
                at,
            ),
            done: host.insert(Button::new(&L10N.tr("done"), click(Signal::HideFind)), at),
            replacement: host.insert(
                TextField::new(
                    &L10N.tr("replace-placeholder"),
                    &self.replacement,
                    push(Signal::Replacement),
                ),
                at,
            ),
            replace: host.insert(Button::new(&L10N.tr("replace"), click(Signal::Replace)), at),
            replace_all: host.insert(
                Button::new(&L10N.tr("replace-all"), click(Signal::ReplaceAll)),
                at,
            ),
            message: host.insert(Label::new("").secondary(), at),
        };
        let query = bar.query;
        self.find = Some(bar);
        self.ui.set_focus(query);
    }

    fn hide_find(&mut self) {
        let Some(bar) = self.find.take() else {
            return;
        };
        if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) {
            for id in [
                bar.query,
                bar.case,
                bar.regex,
                bar.previous,
                bar.next,
                bar.done,
                bar.replacement,
                bar.replace,
                bar.replace_all,
                bar.message,
            ] {
                host.remove(id);
            }
        }
        self.find_message.clear();
        self.shown_message.clear();
        self.ui.set_focus(self.editor);
    }

    pub fn use_selection_for_find(&mut self) {
        let Some(editor) = self.ui.widget_ref::<TextView>(self.editor) else {
            return;
        };
        let range = editor.selection();
        let text = self.text();
        if let Some(selected) = text.get(range).filter(|selected| !selected.is_empty()) {
            self.query.pattern = match self.query.regex {
                true => regex::escape(selected),
                false => selected.to_owned(),
            };
        }
        let signals = Rc::clone(&self.signals);
        let mut field = SearchField::new(
            &L10N.tr("find-placeholder"),
            &self.query.pattern,
            move |text| {
                signals.borrow_mut().push(Signal::Query(text.to_owned()));
            },
        );
        field.select(0..self.query.pattern.len());
        if let Some(bar) = &mut self.find
            && let Some(host) = self.ui.widget_as::<FreeLayout>(self.host)
            && let Some(query) = host.replace(bar.query, field)
        {
            bar.query = query;
        }
    }

    pub fn find_next(&mut self, forward: bool) {
        self.search(forward, false);
    }

    fn search(&mut self, forward: bool, incremental: bool) {
        if self.query.pattern.is_empty() {
            self.find_message.clear();
            return;
        }
        let matcher = match self.query.compile() {
            Ok(matcher) => matcher,
            Err(_) => {
                self.find_message = L10N.tr("find-invalid");
                return;
            }
        };
        let text = self.text();
        let Some(selection) = self
            .ui
            .widget_ref::<TextView>(self.editor)
            .map(TextView::selection)
        else {
            return;
        };
        let from = match (forward, incremental) {
            (true, true) => selection.start,
            (true, false) => selection.end,
            (false, _) => selection.start,
        };
        match find::next(&matcher, &text, from, forward) {
            Some(range) => {
                self.find_message.clear();
                if let Some(editor) = self.editor() {
                    editor.select(range);
                }
            }
            None => self.find_message = L10N.tr("find-none"),
        }
    }

    fn replace(&mut self) {
        let Ok(matcher) = self.query.compile() else {
            self.find_message = L10N.tr("find-invalid");
            return;
        };
        let text = self.text();
        let Some(selection) = self
            .ui
            .widget_ref::<TextView>(self.editor)
            .map(TextView::selection)
        else {
            return;
        };
        let selected = text.get(selection.clone()).unwrap_or_default();
        let exact = matcher
            .find(selected)
            .is_some_and(|found| found.range() == (0..selected.len()) && !selected.is_empty());
        if exact {
            let replaced = self
                .query
                .substitution(&matcher, selected, &self.replacement);
            if let Some(editor) = self.editor() {
                editor.insert(&replaced);
            }
        }
        self.search(true, false);
    }

    fn replace_all(&mut self) {
        let Ok(matcher) = self.query.compile() else {
            self.find_message = L10N.tr("find-invalid");
            return;
        };
        let text = self.text();
        let (replaced, count) = self.query.replace_all(&matcher, &text, &self.replacement);
        if count > 0
            && let Some(editor) = self.editor()
        {
            editor.select(0..text.len());
            editor.insert(&replaced);
            editor.select(0..0);
        }
        self.find_message = L10N.tr_args("replaced", [("count", count.into())]);
    }

    pub fn attributes(&self) -> (TextAttributes, ParagraphAttributes) {
        self.ui
            .widget_ref::<TextView>(self.editor)
            .map(|editor| (editor.attributes(), editor.paragraph_attributes()))
            .unwrap_or_default()
    }

    fn paragraph_range(&mut self) -> Option<std::ops::Range<usize>> {
        let selection = self.ui.widget_ref::<TextView>(self.editor)?.selection();
        let text = self.text();
        let start = text[..selection.start].rfind('\n').map_or(0, |at| at + 1);
        let end = text[selection.end..]
            .find('\n')
            .map_or(text.len(), |at| selection.end + at);
        Some(start..end)
    }

    pub fn style(&mut self, command: &str) -> bool {
        if !self.rich {
            return false;
        }
        let (attributes, _) = self.attributes();
        let (name, value) = command.split_once(':').unwrap_or((command, ""));
        let Some(editor) = self.ui.widget_as::<TextView>(self.editor) else {
            return false;
        };
        match name {
            "bold" => editor.change_attributes(|span| span.bold = !attributes.bold),
            "italic" => editor.change_attributes(|span| span.italic = !attributes.italic),
            "underline" => editor.change_attributes(|span| span.underline = !attributes.underline),
            "strikethrough" => {
                editor.change_attributes(|span| span.strikethrough = !attributes.strikethrough);
            }
            "family" => {
                let family = FAMILIES
                    .iter()
                    .find(|(key, _)| *key == value)
                    .and_then(|(_, family)| *family)
                    .map(Arc::from);
                editor.change_attributes(|span| span.family = family.clone());
            }
            "color" => {
                let color = COLORS
                    .iter()
                    .find(|(key, _)| *key == value)
                    .and_then(|(_, color)| *color)
                    .map(Color::hex);
                editor.change_attributes(|span| span.color = color);
            }
            "bigger" | "smaller" => {
                let step = match name {
                    "bigger" => SIZE_STEP,
                    _ => -SIZE_STEP,
                };
                editor.change_attributes(|span| {
                    let size = span.size.unwrap_or(PROPORTIONAL_SIZE) + step;
                    span.size = Some(size.clamp(MIN_SIZE, MAX_SIZE))
                        .filter(|size| (size - PROPORTIONAL_SIZE).abs() > 0.1);
                });
            }
            "align" => {
                let alignment = match value {
                    "center" => Alignment::Center,
                    "end" => Alignment::End,
                    "justify" => Alignment::Justify,
                    _ => Alignment::Start,
                };
                editor.change_paragraphs(|attributes| attributes.alignment = alignment);
            }
            "list" => {
                let list = match value {
                    "bullet" => Some(ListMarker::Bullet),
                    "number" => Some(ListMarker::Number),
                    _ => None,
                };
                editor.change_paragraphs(|attributes| {
                    attributes.list = list;
                    if list.is_none() {
                        attributes.level = 0;
                    }
                });
            }
            "indent" => editor.change_paragraphs(|attributes| {
                attributes.level = (attributes.level + 1).min(MAX_LEVEL);
            }),
            "outdent" => editor.change_paragraphs(|attributes| {
                attributes.level = attributes.level.saturating_sub(1);
            }),
            "heading" => {
                let level: usize = value.parse().unwrap_or(0);
                let selection = editor.selection();
                let Some(range) = self.paragraph_range() else {
                    return false;
                };
                let Some(editor) = self.ui.widget_as::<TextView>(self.editor) else {
                    return false;
                };
                editor.select(range);
                let styled = match level {
                    0 => TextAttributes::default(),
                    level => heading(level),
                };
                editor.change_attributes(|span| {
                    span.bold = styled.bold;
                    span.size = styled.size;
                });
                editor.select(selection);
            }
            _ => return false,
        }
        self.after_edit();
        true
    }

    pub fn make_plain(&mut self, confirmed: bool) {
        if !self.rich {
            return;
        }
        if !confirmed && self.format != Format::Markdown {
            return self.ask(
                Asking::MakePlain,
                &L10N.tr("make-plain-title"),
                &L10N.tr("make-plain-message"),
                &[("make-plain-confirm", true), ("cancel", false)],
            );
        }
        let rich = self
            .ui
            .widget_ref::<TextView>(self.editor)
            .map(TextView::rich_text)
            .unwrap_or_default();
        let text = match self.format {
            Format::Markdown => markdown::write(&rich),
            _ => rich.text,
        };
        if let Some(editor) = self.editor() {
            editor.set_text(&text);
        }
        self.rich = false;
        if self.format != Format::Markdown {
            self.retarget(Format::Plain);
        }
        self.settings = Settings::PLAIN;
        self.apply_settings();
        self.changed_format();
    }

    pub fn make_rich(&mut self) {
        if self.rich {
            return;
        }
        let text = self.text();
        let rich = match self.format {
            Format::Markdown => markdown::read(&text),
            _ => RichText {
                text: text.to_string(),
                runs: Vec::new(),
                paragraphs: Vec::new(),
            },
        };
        if let Some(editor) = self.editor() {
            editor.set_rich_text(&rich);
        }
        self.rich = true;
        if self.format != Format::Markdown {
            self.retarget(Format::Rtf);
        }
        self.settings = Settings::RICH;
        self.apply_settings();
        self.changed_format();
    }

    fn retarget(&mut self, format: Format) {
        self.format = format;
        if let Some(path) = self.path.take() {
            let stem = path
                .file_stem()
                .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
            self.untitled = format!("{stem}.{}", format.extension());
        }
    }

    fn changed_format(&mut self) {
        self.format_changed = true;
        self.saved = None;
        self.after_edit();
        self.persist();
    }

    fn take_signals(&mut self) -> bool {
        let signals: Vec<Signal> = self.signals.borrow_mut().drain(..).collect();
        let any = !signals.is_empty();
        for signal in signals {
            match signal {
                Signal::Query(pattern) => {
                    self.query.pattern = pattern;
                    self.search(true, true);
                }
                Signal::Replacement(text) => self.replacement = text,
                Signal::Case(case) => self.query.case = case,
                Signal::Regex(regex) => self.query.regex = regex,
                Signal::Next => self.find_next(true),
                Signal::Previous => self.find_next(false),
                Signal::Replace => self.replace(),
                Signal::ReplaceAll => self.replace_all(),
                Signal::HideFind => self.hide_find(),
                Signal::Choice(action) => self.chosen(&action),
                Signal::Line(line) => self.line = line,
            }
        }
        any
    }

    fn after_edit(&mut self) {
        let revision = self.revision();
        if revision != self.seen {
            self.seen = revision;
            self.settle_at = Some(Instant::now() + SETTLE);
        }
    }

    fn status_line(&self) -> String {
        let counts = match (self.loading, self.counts) {
            (true, _) => L10N.tr("status-opening"),
            (false, Some((words, characters))) => L10N.tr_args(
                "status-counts",
                [("words", words.into()), ("characters", characters.into())],
            ),
            (false, None) => String::new(),
        };
        let mut parts = vec![counts];
        if self.format != Format::Plain {
            parts.push(L10N.tr(&format!("format-{}", self.format.name())));
        }
        if self.format.is_text() {
            parts.push(crate::encoding_name(self.encoding));
            parts.push(L10N.tr(&format!("line-ending-{}", self.line_ending.name())));
        }
        parts.extend(self.note.clone());
        parts.retain(|part| !part.is_empty());
        parts.join("   ·   ")
    }

    fn fit(&mut self, label: &str) -> f32 {
        let style = TextStyle::new(13.0, self.ui.context().palette.ink);
        let width = self
            .ui
            .context()
            .text
            .paragraph(label, style, None, TextAlign::Start)
            .width();
        (width + CONTROL_PADDING).ceil()
    }

    fn layout(&mut self, width: f32, height: f32) {
        let status_text = self.status_line();
        if status_text != self.status_text {
            self.status_text = status_text.clone();
            if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host)
                && let Some(status) =
                    host.replace(self.status, Label::new(&status_text).secondary())
            {
                self.status = status;
            }
        }
        let message = self.find_message.clone();
        if message != self.shown_message
            && let Some(bar) = &mut self.find
            && let Some(host) = self.ui.widget_as::<FreeLayout>(self.host)
            && let Some(label) = host.replace(bar.message, Label::new(&message).secondary())
        {
            bar.message = label;
            self.shown_message = message;
        }
        let mut frames = Vec::new();
        let mut top = 0.0;
        if let Some(bar) = &self.find {
            let ids = (
                bar.query,
                bar.case,
                bar.regex,
                bar.previous,
                bar.next,
                bar.done,
                bar.replacement,
                bar.replace,
                bar.replace_all,
                bar.message,
                bar.replacing,
            );
            let (
                query,
                case,
                regex,
                previous,
                next,
                done,
                replacement,
                replace,
                all,
                label,
                replacing,
            ) = ids;
            let row =
                |index: f32, height: f32| index * FIND_ROW + ((FIND_ROW - height) / 2.0).round();
            let mut right = width - MARGIN;
            let mut place = |id: WidgetId,
                             item_width: f32,
                             index: f32,
                             height: f32,
                             frames: &mut Vec<(WidgetId, Rect)>| {
                right -= item_width;
                frames.push((id, Rect::new(right, row(index, height), item_width, height)));
                right -= GAP;
            };
            let done_width = self.fit(&L10N.tr("done"));
            let regex_width = self.fit(&L10N.tr("regular-expression"));
            let case_width = self.fit(&L10N.tr("match-case"));
            place(done, done_width, 0.0, BUTTON_HEIGHT, &mut frames);
            place(next, BUTTON_HEIGHT + 8.0, 0.0, BUTTON_HEIGHT, &mut frames);
            place(
                previous,
                BUTTON_HEIGHT + 8.0,
                0.0,
                BUTTON_HEIGHT,
                &mut frames,
            );
            place(regex, regex_width, 0.0, BUTTON_HEIGHT, &mut frames);
            place(case, case_width, 0.0, BUTTON_HEIGHT, &mut frames);
            let field_width = (right - MARGIN).max(FIELD_WIDTH);
            frames.push((
                query,
                Rect::new(
                    MARGIN,
                    row(0.0, TEXT_FIELD_HEIGHT),
                    field_width,
                    TEXT_FIELD_HEIGHT,
                ),
            ));
            top = FIND_ROW;
            match replacing {
                true => {
                    let replace_width = self.fit(&L10N.tr("replace"));
                    let all_width = self.fit(&L10N.tr("replace-all"));
                    let field = Rect::new(
                        MARGIN,
                        row(1.0, TEXT_FIELD_HEIGHT),
                        field_width,
                        TEXT_FIELD_HEIGHT,
                    );
                    frames.push((replacement, field));
                    let x = MARGIN + field_width + GAP;
                    frames.push((
                        replace,
                        Rect::new(x, row(1.0, BUTTON_HEIGHT), replace_width, BUTTON_HEIGHT),
                    ));
                    let x = x + replace_width + GAP;
                    frames.push((
                        all,
                        Rect::new(x, row(1.0, BUTTON_HEIGHT), all_width, BUTTON_HEIGHT),
                    ));
                    let x = x + all_width + GAP;
                    frames.push((
                        label,
                        Rect::new(x, row(1.0, 16.0), (width - x - MARGIN).max(0.0), 16.0),
                    ));
                    top = 2.0 * FIND_ROW;
                }
                false => {
                    frames.push((replacement, Rect::default()));
                    frames.push((replace, Rect::default()));
                    frames.push((all, Rect::default()));
                    frames.push((
                        label,
                        Rect::new(MARGIN, top, (width - 2.0 * MARGIN).max(0.0), 16.0),
                    ));
                    top += 16.0;
                }
            }
        }
        frames.push((
            self.editor,
            Rect::new(0.0, top, width, (height - top - STATUS_HEIGHT).max(0.0)),
        ));
        frames.push((
            self.status,
            Rect::new(
                MARGIN * 2.0,
                height - STATUS_HEIGHT + 4.0,
                (width - 4.0 * MARGIN).max(0.0),
                STATUS_HEIGHT - 4.0,
            ),
        ));
        if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) {
            for (id, frame) in frames {
                host.set_frame(id, frame);
            }
        }
    }

    fn key(&mut self, event: &Event) -> Option<bool> {
        let Event::Key { key, modifiers } = event else {
            return None;
        };
        let focus = self.ui.focus();
        let plain = *modifiers == Modifiers::default();
        let shift_only = *modifiers
            == Modifiers {
                shift: true,
                ..Modifiers::default()
            };
        if self.ui.has_sheet() {
            return None;
        }
        match key {
            Key::Tab if plain && focus == Some(self.editor) => {
                self.editor()?.insert("\t");
                Some(true)
            }
            Key::Enter
                if (plain || shift_only)
                    && self
                        .find
                        .as_ref()
                        .is_some_and(|bar| Some(bar.query) == focus) =>
            {
                self.find_next(plain);
                Some(true)
            }
            Key::Escape if plain && self.find.is_some() => {
                self.hide_find();
                Some(true)
            }
            _ => None,
        }
    }
}

impl View for DocumentView {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.layout(width as f32, height as f32);
        self.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        if let WindowEvent::Input(input) = &event
            && let Some(handled) = self.key(input)
        {
            self.after_edit();
            window.request_redraw();
            return handled;
        }
        let handled = window
            .drive(&mut self.ui, event)
            .inspect_err(|error| tracing::warn!("{error}"))
            .is_ok_and(|unhandled| unhandled.is_none());
        let signalled = self.take_signals();
        self.after_edit();
        if signalled {
            window.request_redraw();
        }
        handled || signalled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn deadline(&self) -> Option<Instant> {
        self.settle_at
    }

    fn tick(&mut self, _window: &mut Window, now: Instant) -> bool {
        if self.settle_at.is_some_and(|at| at <= now) {
            self.settle_at = None;
            self.persist();
        }
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<bande_toolkit::Rect> {
        self.ui.text_input_area()
    }
}

fn autosaved(content: &Content) -> String {
    match content {
        Content::Plain(text) => text.to_string(),
        Content::Rich(rich) => rtf::write(rich),
    }
}
