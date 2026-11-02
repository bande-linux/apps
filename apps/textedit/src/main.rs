use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bande_app::{Cx, Info, Keys, Kind, Menus, Program, WindowId};
use bande_app_menu::{Entry, edit_submenu};
use bande_inbox::Inbox;
use bande_textedit::{
    CHOICES, DocumentView, L10N, LineEnding, Loaded, MAX_FILE_BYTES, Message, Opening, Request,
    Session, Settings, TextEncoding, choose_files, choose_save_path, encoding_name, new_id, now,
    read, read_bounded, spawn,
};

const APP_ID: &str = "org.bande.TextEdit";
const WINDOW_SIZE: (u32, u32) = (760, 640);
const STATE: &str = "textedit";
const ZOOM_STEP: f32 = 1.0;

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
    keys: Keys::Standard,
    settings: None,
};

struct TextEdit {
    inbox: Inbox<Message>,
    session: Option<Session>,
    recent: Vec<PathBuf>,
    untitled: u32,
    next_token: u64,
}

fn item(label: &str, action: &str) -> Entry {
    Entry::item(&L10N.tr(label), action)
}

fn path_of(argument: &str, directory: &Path) -> Option<PathBuf> {
    let path = match argument.starts_with("file://") {
        true => bande_xdg::path_from_file_uri(argument)?,
        false => PathBuf::from(argument),
    };
    Some(match path.is_absolute() {
        true => path,
        false => directory.join(path),
    })
}

fn load(path: &Path, chosen: Option<&'static encoding_rs::Encoding>) -> Result<Loaded, String> {
    read_bounded(path, MAX_FILE_BYTES)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::FileTooLarge => L10N.tr("too-large"),
            _ => error.to_string(),
        })
        .and_then(|bytes| read(bytes, chosen))
}

impl TextEdit {
    fn find(&self, cx: &Cx<Self>, token: u64) -> Option<WindowId> {
        cx.windows()
            .find(|window| cx.view(*window).is_some_and(|view| view.token == token))
    }

    fn view(cx: &mut Cx<Self>, window: WindowId) -> Option<&mut DocumentView> {
        cx.window(window).map(|(_, view)| view)
    }

    fn open_view(
        &mut self,
        cx: &mut Cx<Self>,
        id: String,
        untitled: String,
        opened: u128,
        settings: Settings,
    ) -> Option<(WindowId, u64)> {
        self.next_token += 1;
        let token = self.next_token;
        let outbox = self.inbox.outbox();
        let session = self.session.clone();
        let title = untitled.clone();
        let opening = Opening {
            token,
            id,
            untitled,
            opened,
            settings,
        };
        let opened = cx.open(Kind::Main, &title, WINDOW_SIZE, move |appearance| {
            DocumentView::new(appearance, opening, outbox, session)
        });
        match opened {
            Ok(window) => Some((window, token)),
            Err(error) => {
                tracing::warn!("cannot open a window: {error}");
                None
            }
        }
    }

    fn next_untitled(&mut self) -> String {
        self.untitled += 1;
        match self.untitled {
            1 => L10N.tr("untitled"),
            number => L10N.tr_args("untitled-numbered", [("number", number.into())]),
        }
    }

    fn new_document(&mut self, cx: &mut Cx<Self>) {
        let untitled = self.next_untitled();
        let opened = self.open_view(cx, new_id(), untitled, now(), Settings::PLAIN);
        if let Some((window, _)) = opened {
            self.serve(cx, window);
        }
    }

    fn open_path(&mut self, cx: &mut Cx<Self>, path: PathBuf) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        let shown = cx.windows().find(|window| {
            cx.view(*window)
                .is_some_and(|view| view.path.as_deref() == Some(path.as_path()))
        });
        if let Some(window) = shown {
            return cx.raise(window, None);
        }
        let name = path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let opened = self.open_view(cx, new_id(), name, now(), Settings::PLAIN);
        let Some((window, token)) = opened else {
            return;
        };
        if let Some(view) = Self::view(cx, window) {
            view.start_loading(Some(path.clone()));
        }
        self.load_into(token, path, None);
        self.serve(cx, window);
    }

    fn load_into(&self, token: u64, path: PathBuf, chosen: Option<&'static encoding_rs::Encoding>) {
        let outbox = self.inbox.outbox();
        spawn("open", move || {
            outbox.send(Message::Loaded {
                token,
                loaded: load(&path, chosen),
            });
        });
    }

    fn restore(&mut self, cx: &mut Cx<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        for (id, record) in session.records() {
            if !record.edited && record.path.as_deref().is_none_or(|path| !path.exists()) {
                session.forget(&id);
                continue;
            }
            if record.path.is_none() {
                let number = record
                    .title
                    .rsplit(' ')
                    .next()
                    .and_then(|number| number.parse::<u32>().ok())
                    .unwrap_or(1);
                self.untitled = self.untitled.max(number);
            }
            let settings = Settings {
                monospace: record.monospace,
                page: record.page,
                size: record.size,
            };
            let encoding = TextEncoding::named(&record.encoding, record.bom);
            let Some((window, token)) = self.open_view(
                cx,
                id.clone(),
                record.title.clone(),
                record.opened,
                settings,
            ) else {
                continue;
            };
            if let Some(view) = Self::view(cx, window) {
                view.start_loading(record.path.clone());
                if let Some(encoding) = encoding {
                    view.encoding = encoding;
                }
                if let Some(line_ending) = LineEnding::parse(&record.line_ending) {
                    view.line_ending = line_ending;
                }
            }
            match (record.edited, record.path) {
                (true, _) => {
                    let (outbox, session, id) = (self.inbox.outbox(), session.clone(), id.clone());
                    spawn("restore", move || {
                        let loaded = session
                            .text(&id)
                            .map_err(|error| error.to_string())
                            .and_then(|bytes| read(bytes, Some(encoding_rs::UTF_8)))
                            .map(|loaded| Loaded {
                                from_disk: false,
                                ..loaded
                            });
                        outbox.send(Message::Loaded { token, loaded });
                    });
                }
                (false, Some(path)) => {
                    self.load_into(token, path, encoding.map(|encoding| encoding.encoding));
                }
                (false, None) => {}
            }
            self.serve(cx, window);
        }
    }

    fn serve(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let Some((shown, view)) = cx.window(window) else {
            return;
        };
        let requests = view.take_requests();
        shown.set_title(&view.title());
        shown.request_redraw();
        for request in requests {
            match request {
                Request::Close => cx.close(window),
                Request::SaveAs => self.ask_save_path(cx, window),
                Request::Remember(path) => self.remember(&path),
            }
        }
        cx.refresh_menus();
    }

    fn remember(&mut self, path: &Path) {
        if let Some(session) = &self.session {
            session.remember(&mut self.recent, path);
        }
    }

    fn ask_save_path(&self, cx: &mut Cx<Self>, window: WindowId) {
        let Some(view) = cx.view(window) else {
            return;
        };
        let (token, name) = (view.token, view.name());
        let outbox = self.inbox.outbox();
        let title = L10N.tr("save-as-title");
        spawn("save as", move || {
            let path = choose_save_path(&title, &name)
                .inspect_err(|error| tracing::warn!("cannot choose where to save: {error}"))
                .ok()
                .flatten();
            outbox.send(Message::SaveTo { token, path });
        });
    }

    fn ask_open(&self) {
        let outbox = self.inbox.outbox();
        let title = L10N.tr("open-title");
        spawn("choose", move || {
            let paths = choose_files(&title)
                .inspect_err(|error| tracing::warn!("cannot choose files: {error}"))
                .unwrap_or_default();
            outbox.send(Message::Chosen(paths));
        });
    }

    fn file_menu(&self, view: Option<&DocumentView>) -> Entry {
        let windowed = view.is_some();
        let edited = view.is_some_and(DocumentView::is_edited);
        let on_disk = view.is_some_and(|view| view.path.is_some() && !view.loading);
        let recent = self
            .recent
            .iter()
            .enumerate()
            .map(|(index, path)| Entry::item(&path.to_string_lossy(), &format!("recent:{index}")))
            .chain([
                Entry::separator(),
                item("clear-recent", "clear-recent").enabled(!self.recent.is_empty()),
            ])
            .collect();
        let reopen = CHOICES
            .iter()
            .map(|encoding| {
                Entry::item(
                    &encoding_name(TextEncoding {
                        encoding,
                        bom: false,
                    }),
                    &format!("reopen:{}", encoding.name()),
                )
            })
            .collect();
        Entry::submenu(
            &L10N.tr("menu-file"),
            vec![
                item("new", "new").shortcut("Ctrl+N"),
                item("open", "open").shortcut("Ctrl+O"),
                Entry::submenu(&L10N.tr("open-recent"), recent),
                Entry::separator(),
                item("save", "save").shortcut("Ctrl+S").enabled(windowed),
                item("save-as", "save-as")
                    .shortcut("Shift+Ctrl+S")
                    .enabled(windowed),
                item("revert", "revert").enabled(on_disk && edited),
                Entry::submenu(&L10N.tr("reopen-with-encoding"), reopen)
                    .enabled(on_disk && !edited),
            ],
        )
    }

    fn format_menu(view: Option<&DocumentView>) -> Entry {
        let windowed = view.is_some();
        let monospace = view.is_some_and(|view| view.settings.monospace);
        let page = view.is_some_and(|view| view.settings.page);
        let current = view.map(|view| view.encoding.encoding);
        let encodings = CHOICES
            .iter()
            .map(|encoding| {
                Entry::item(
                    &encoding_name(TextEncoding {
                        encoding,
                        bom: false,
                    }),
                    &format!("encoding:{}", encoding.name()),
                )
                .radio(current == Some(*encoding))
            })
            .collect();
        let ending = view.map(|view| view.line_ending);
        let endings = LineEnding::ALL
            .into_iter()
            .map(|line_ending| {
                item(
                    &format!("line-ending-{}", line_ending.name()),
                    &format!("line-ending:{}", line_ending.name()),
                )
                .radio(ending == Some(line_ending))
            })
            .collect();
        let entries = vec![
            Entry::submenu(
                &L10N.tr("menu-font"),
                vec![
                    item("font-monospace", "font-monospace").radio(windowed && monospace),
                    item("font-proportional", "font-proportional").radio(windowed && !monospace),
                    Entry::separator(),
                    item("bigger", "bigger").shortcut("Ctrl+="),
                    item("smaller", "smaller").shortcut("Ctrl+-"),
                    item("actual-size", "actual-size").shortcut("Ctrl+0"),
                ],
            ),
            Entry::separator(),
            item("wrap-window", "wrap-window").radio(windowed && !page),
            item("wrap-page", "wrap-page").radio(windowed && page),
            Entry::separator(),
            Entry::submenu(&L10N.tr("text-encoding"), encodings),
            Entry::submenu(&L10N.tr("line-endings"), endings),
        ];
        Entry::submenu(
            &L10N.tr("menu-format"),
            entries
                .into_iter()
                .map(|entry| entry.enabled(windowed))
                .collect(),
        )
    }

    fn document_command(&mut self, cx: &mut Cx<Self>, window: WindowId, command: &str) -> bool {
        if command == "save-as" {
            self.ask_save_path(cx, window);
            return true;
        }
        let Some(view) = Self::view(cx, window) else {
            return false;
        };
        match command {
            "save" => view.save(false),
            "find" => view.show_find(false),
            "find-replace" => view.show_find(true),
            "find-next" => view.find_next(true),
            "find-previous" => view.find_next(false),
            "use-selection" => view.use_selection_for_find(),
            "go-to-line" => view.go_to_line(),
            "font-monospace" => view.set_monospace(true),
            "font-proportional" => view.set_monospace(false),
            "bigger" => view.zoom(Some(ZOOM_STEP)),
            "smaller" => view.zoom(Some(-ZOOM_STEP)),
            "actual-size" => view.zoom(None),
            "wrap-window" => view.set_page(false),
            "wrap-page" => view.set_page(true),
            "revert" => {
                let (token, encoding) = (view.token, view.encoding.encoding);
                let Some(path) = view.path.clone() else {
                    return false;
                };
                view.start_loading(Some(path.clone()));
                self.load_into(token, path, Some(encoding));
            }
            _ => {
                if let Some(name) = command.strip_prefix("encoding:")
                    && let Some(encoding) = TextEncoding::named(name, false)
                {
                    let bom = view.encoding.bom && encoding.encoding == encoding_rs::UTF_8;
                    view.set_encoding(TextEncoding { bom, ..encoding });
                } else if let Some(name) = command.strip_prefix("line-ending:")
                    && let Some(line_ending) = LineEnding::parse(name)
                {
                    view.set_line_ending(line_ending);
                } else if let Some(name) = command.strip_prefix("reopen:")
                    && let Some(encoding) = TextEncoding::named(name, false)
                    && let Some(path) = view.path.clone()
                {
                    let token = view.token;
                    view.start_loading(Some(path.clone()));
                    self.load_into(token, path, Some(encoding.encoding));
                } else {
                    return false;
                }
            }
        }
        true
    }

    fn message(&mut self, cx: &mut Cx<Self>, message: Message) {
        let token = match &message {
            Message::Chosen(paths) => {
                for path in paths {
                    self.open_path(cx, path.clone());
                }
                return;
            }
            Message::Loaded { token, .. }
            | Message::Written { token, .. }
            | Message::Counted { token, .. }
            | Message::SaveTo { token, .. } => *token,
        };
        let Some(window) = self.find(cx, token) else {
            return;
        };
        let Some(view) = Self::view(cx, window) else {
            return;
        };
        match message {
            Message::Loaded {
                loaded: Ok(loaded), ..
            } => {
                let remembered = loaded.from_disk.then(|| view.path.clone()).flatten();
                view.loaded(loaded);
                if let Some(path) = remembered {
                    self.remember(&path);
                }
            }
            Message::Loaded {
                loaded: Err(error), ..
            } => view.failed(&error),
            Message::Written {
                revision,
                path,
                outcome,
                ..
            } => view.written(revision, path, outcome),
            Message::Counted {
                revision,
                words,
                characters,
                ..
            } => view.counted(revision, words, characters),
            Message::SaveTo {
                path: Some(path), ..
            } => view.save_to(path),
            Message::SaveTo { path: None, .. } => view.cancel_save(),
            Message::Chosen(_) => {}
        }
        self.serve(cx, window);
    }
}

impl Program for TextEdit {
    type View = DocumentView;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let session = cx.state_file(STATE).map(Session::new);
        let recent = session.as_ref().map(Session::recent).unwrap_or_default();
        let mut program = Self {
            inbox: Inbox::new()?,
            session,
            recent,
            untitled: 0,
            next_token: 0,
        };
        program.restore(cx);
        let directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        for argument in arguments {
            if let Some(path) = path_of(&argument, &directory) {
                program.open_path(cx, path);
            }
        }
        if cx.windows().next().is_none() {
            program.new_document(cx);
        }
        Ok(program)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let view = cx.focused().and_then(|window| cx.view(window));
        let windowed = view.is_some();
        let find = vec![
            item("find", "find").shortcut("Ctrl+F").enabled(windowed),
            item("find-replace", "find-replace")
                .shortcut("Ctrl+H")
                .enabled(windowed),
            item("find-next", "find-next")
                .shortcut("Ctrl+G")
                .enabled(windowed),
            item("find-previous", "find-previous")
                .shortcut("Shift+Ctrl+G")
                .enabled(windowed),
            item("use-selection", "use-selection")
                .shortcut("Ctrl+E")
                .enabled(windowed),
            Entry::separator(),
            item("go-to-line", "go-to-line")
                .shortcut("Ctrl+L")
                .enabled(windowed),
        ];
        let documents = cx
            .windows()
            .filter_map(|window| cx.view(window))
            .map(|view| Entry::item(&view.title(), &format!("window:{}", view.token)))
            .collect();
        Menus {
            menus: vec![
                self.file_menu(view),
                edit_submenu(find),
                Self::format_menu(view),
            ],
            window: documents,
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, window: Option<WindowId>, command: &str) {
        match command {
            "new" => return self.new_document(cx),
            "open" => return self.ask_open(),
            "clear-recent" => {
                self.recent.clear();
                if let Some(session) = &self.session {
                    session.write_recent(&self.recent);
                }
                return;
            }
            _ => {}
        }
        if let Some(index) = command.strip_prefix("recent:") {
            if let Some(path) = index
                .parse::<usize>()
                .ok()
                .and_then(|index| self.recent.get(index).cloned())
            {
                self.open_path(cx, path);
            }
            return;
        }
        if let Some(token) = command.strip_prefix("window:") {
            if let Some(shown) = token.parse().ok().and_then(|token| self.find(cx, token)) {
                cx.raise(shown, None);
            }
            return;
        }
        if let Some(window) = window.or_else(|| cx.focused())
            && self.document_command(cx, window, command)
        {
            self.serve(cx, window);
        }
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        for argument in arguments {
            if let Some(path) = path_of(&argument, directory) {
                self.open_path(cx, path);
            }
        }
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        self.new_document(cx);
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        self.serve(cx, window);
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, _window: WindowId, view: DocumentView) {
        view.forget();
    }

    fn should_close(&mut self, cx: &mut Cx<Self>, window: WindowId) -> bool {
        let Some(view) = Self::view(cx, window) else {
            return true;
        };
        if view.loading || !view.is_edited() || view.is_empty_untitled() {
            return true;
        }
        view.ask_to_save();
        self.serve(cx, window);
        false
    }

    fn should_quit(&mut self, cx: &mut Cx<Self>) -> bool {
        let windows: Vec<WindowId> = cx.windows().collect();
        for window in windows {
            if let Some(view) = Self::view(cx, window) {
                view.flush();
            }
        }
        true
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.inbox.fd()]
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, _index: usize) {
        for message in self.inbox.drain() {
            self.message(cx, message);
        }
    }
}

fn main() -> ExitCode {
    bande_app::run::<TextEdit>(&INFO)
}
