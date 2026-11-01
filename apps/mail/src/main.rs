mod menus;
mod picker;

use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_app::{Cx, Info, Keys, Kind, Menus, Program, View, WindowId};
use bande_mail::L10N;
use bande_mail::compose::{Outgoing, from_mailto};
use bande_mail::message::Attachment;
use bande_mail::ui::{COMPOSE_SIZE, ComposeRequest, ComposeView, MailView, Request, SIDEBAR_WIDTH};
use bande_notify::Urgency;
use bande_notify::client::{Request as Notification, notify};
use bande_toolkit::{
    Appearance, FrameRegions, MenuPlacement, Rect, Scene, TextSystem, Window, WindowEvent,
};
use bande_xdg::{Apps, config_dirs, data_dirs, data_home};

use crate::picker::{MAX_ATTACHMENT, Picked, Picker};

const APP_ID: &str = "org.bande.Mail";
const WINDOW_SIZE: (u32, u32) = (1180, 720);
const NEW_MAIL_SOUND: &str = "message-new-email";

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
    keys: Keys::Standard,
    settings: None,
};

enum Pane {
    Main(Box<MailView>),
    Compose(Box<ComposeView>),
}

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let full_screen = window.header_height() == 0;
        let (width, height) = window.size();
        match self {
            Self::Main(view) => {
                window.set_frame_regions(FrameRegions {
                    sidebar: SIDEBAR_WIDTH,
                    toolbar: 0.0,
                });
                view.set_full_screen(full_screen);
                let scene = view.scene(width, height);
                window.set_header(view.header());
                scene
            }
            Self::Compose(view) => {
                window.set_frame_regions(FrameRegions {
                    sidebar: 0.0,
                    toolbar: 0.0,
                });
                let scene = view.scene(width, height);
                window.set_header(view.header());
                scene
            }
        }
    }

    fn event(&mut self, _window: &mut Window, event: WindowEvent) -> bool {
        match (self, event) {
            (Self::Main(view), WindowEvent::Input(input)) => view.handle(&input),
            (Self::Compose(view), WindowEvent::Input(input)) => view.handle(&input),
            (Self::Main(view), WindowEvent::Accessibility(request)) => view.perform(&request),
            (Self::Compose(view), WindowEvent::Accessibility(request)) => view.perform(&request),
            (Self::Main(view), WindowEvent::Pasted(text)) => view.pasted(&text),
            (Self::Compose(view), WindowEvent::Pasted(text)) => view.pasted(&text),
            (Self::Main(view), WindowEvent::MenuAction(action)) => view.menu_action(&action),
            (Self::Compose(view), WindowEvent::MenuAction(action)) => view.menu_action(&action),
            (Self::Main(view), WindowEvent::MenuClosed) => view.menu_closed(),
            (Self::Compose(view), WindowEvent::MenuClosed) => view.menu_closed(),
            _ => false,
        }
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        match self {
            Self::Main(view) => view.set_appearance(appearance),
            Self::Compose(view) => view.set_appearance(appearance),
        }
    }

    fn set_active(&mut self, active: bool) {
        if let Self::Main(view) = self {
            view.set_window_active(active);
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        match self {
            Self::Main(view) => view.fds(),
            Self::Compose(_) => Vec::new(),
        }
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) -> bool {
        match self {
            Self::Main(view) => view.engine_ready(),
            Self::Compose(_) => false,
        }
    }

    fn deadline(&self) -> Option<Instant> {
        match self {
            Self::Main(view) => view.deadline(),
            Self::Compose(view) => view.deadline(),
        }
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) -> bool {
        true
    }

    fn animating(&self) -> bool {
        match self {
            Self::Main(view) => view.animating(),
            Self::Compose(view) => view.animating(),
        }
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        match self {
            Self::Main(view) => Some(view.accessibility()),
            Self::Compose(view) => Some(view.accessibility()),
        }
    }

    fn text_input_area(&self) -> Option<Rect> {
        match self {
            Self::Main(view) => view.text_input_area(),
            Self::Compose(view) => view.text_input_area(),
        }
    }
}

struct Mail {
    main: WindowId,
    apps: Option<Apps>,
    picker: Picker,
}

fn store_path() -> PathBuf {
    data_home()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("bande-mail/mail.sqlite3")
}

fn announce(summary: String, body: String) {
    let spawned = std::thread::Builder::new()
        .name("notify".to_owned())
        .spawn(move || {
            let shown = zbus::blocking::Connection::session().and_then(|session| {
                notify(
                    &session,
                    &Notification {
                        app_name: &L10N.tr("app-name"),
                        replaces_id: 0,
                        app_icon: APP_ID,
                        summary: &summary,
                        body: &body,
                        actions: &[],
                        urgency: Urgency::Normal,
                        sound: Some(NEW_MAIL_SOUND),
                    },
                )
            });
            if let Err(error) = shown {
                tracing::debug!("cannot show a notification: {error}");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot notify: {error}");
    }
}

fn main_view(cx: &mut Cx<Mail>, main: WindowId) -> Option<&mut MailView> {
    match cx.window(main) {
        Some((_, Pane::Main(view))) => Some(view),
        _ => None,
    }
}

impl Mail {
    fn apps(&mut self) -> &Apps {
        self.apps
            .get_or_insert_with(|| Apps::load(data_dirs(), &config_dirs()))
    }

    fn open_compose(&mut self, cx: &mut Cx<Self>, outgoing: Outgoing) -> Option<WindowId> {
        let senders = main_view(cx, self.main)
            .map(|view| {
                view.accounts()
                    .iter()
                    .map(|account| account.address())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut view = ComposeView::new(
            outgoing,
            senders,
            Appearance::default().palette(),
            TextSystem::new(),
        );
        let title = view.title();
        let opened = cx.open(Kind::Main, &title, COMPOSE_SIZE, |appearance| {
            view.set_appearance(appearance);
            Pane::Compose(Box::new(view))
        });
        opened
            .inspect_err(|error| tracing::warn!("cannot open a message window: {error}"))
            .ok()
    }

    fn open_argument(&mut self, cx: &mut Cx<Self>, argument: &str) {
        if let Some(draft) = from_mailto(argument) {
            if let Some(view) = main_view(cx, self.main) {
                view.compose(draft);
            }
            let main = self.main;
            self.serve(cx, main);
        }
    }

    fn open_url(&mut self, cx: &mut Cx<Self>, window: WindowId, url: &str) {
        if from_mailto(url).is_some() {
            self.open_argument(cx, url);
            return;
        }
        let scheme = url
            .split(':')
            .next()
            .unwrap_or("https")
            .to_ascii_lowercase();
        let token = cx
            .window(window)
            .and_then(|(window, _)| window.activation_token(APP_ID).ok());
        let entry = self
            .apps()
            .default_for(&format!("x-scheme-handler/{scheme}"))
            .cloned();
        match entry {
            Some(entry) => {
                if let Err(error) =
                    bande_xdg::launch_with_arguments(&entry, &[url.to_owned()], token.as_deref())
                {
                    tracing::warn!("cannot open {url}: {error}");
                }
            }
            None => tracing::warn!("no app opens {scheme} links"),
        }
    }

    fn open_file(&mut self, cx: &mut Cx<Self>, window: WindowId, path: &Path) {
        let token = cx
            .window(window)
            .and_then(|(window, _)| window.activation_token(APP_ID).ok());
        let entry = self.apps().default_for_path(path).cloned();
        match entry {
            Some(entry) => {
                if let Err(error) =
                    bande_xdg::launch_with(&entry, &[path.to_path_buf()], token.as_deref())
                {
                    tracing::warn!("cannot open {}: {error}", path.display());
                }
            }
            None => tracing::warn!("no app opens {}", path.display()),
        }
    }

    fn attach(&mut self, cx: &mut Cx<Self>, token: u64, picked: Vec<Picked>) {
        let mut attachments = Vec::new();
        let mut refused = Vec::new();
        for item in picked {
            match item {
                Picked::File(path, data) => {
                    let mime = self.apps().mime_for(&path);
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "attachment".to_owned());
                    attachments.push(Attachment { name, mime, data });
                }
                Picked::TooLarge(path) => refused.push(path),
            }
        }
        let windows: Vec<WindowId> = cx.windows().collect();
        let Some(window) = windows.into_iter().find(
            |window| matches!(cx.view(*window), Some(Pane::Compose(view)) if view.token() == token),
        ) else {
            return;
        };
        if let Some((_, Pane::Compose(view))) = cx.window(window) {
            view.add_attachments(attachments);
            if let Some(path) = refused.first() {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                view.show_error(&L10N.tr_args(
                    "compose-too-large",
                    [
                        ("name", name.into()),
                        ("limit", bande_fs::format_size(MAX_ATTACHMENT).into()),
                    ],
                ));
            }
        }
        refresh(cx, window);
    }

    fn serve_compose(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let Some((shown, Pane::Compose(view))) = cx.window(window) else {
            return;
        };
        let requests = view.take_requests();
        let sent = view.take_sent();
        let finished = view.finished();
        let token = view.token();
        for request in requests {
            let result = match request {
                ComposeRequest::ChooseFiles => {
                    self.picker.pick(token, L10N.tr("compose-attach-title"));
                    Ok(())
                }
                ComposeRequest::ShowMenu(menu, at) => {
                    shown.show_menu(menu, MenuPlacement::AtPoint(at))
                }
                ComposeRequest::Clipboard(request) => shown.serve_clipboard(request),
            };
            if let Err(error) = result {
                tracing::warn!("cannot serve the message window: {error}");
            }
        }
        if let Some(outgoing) = sent {
            let delivered = main_view(cx, self.main).is_some_and(|main| main.send(outgoing));
            if !delivered {
                tracing::warn!("no account sends from this address");
            }
        }
        if finished {
            self.keep_draft(cx, window);
            cx.close(window);
        } else {
            refresh(cx, window);
        }
    }

    fn serve(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let requests = match cx.window(window) {
            Some((_, Pane::Main(view))) => view.take_requests(),
            Some((_, Pane::Compose(_))) => return self.serve_compose(cx, window),
            None => return,
        };
        for request in requests {
            match request {
                Request::Compose(outgoing) => {
                    self.open_compose(cx, *outgoing);
                }
                Request::OpenLink(url) => self.open_url(cx, window, &url),
                Request::OpenAttachment(path) => self.open_file(cx, window, &path),
                Request::Notify { summary, body } => announce(summary, body),
                Request::NotSent { error, outgoing } => {
                    if let Some(opened) = self.open_compose(cx, *outgoing)
                        && let Some((_, Pane::Compose(view))) = cx.window(opened)
                    {
                        view.show_error(&error);
                    }
                }
                Request::ShowMenu(menu, at) => {
                    if let Some((shown, _)) = cx.window(window)
                        && let Err(error) = shown.show_menu(menu, MenuPlacement::AtPoint(at))
                    {
                        tracing::warn!("cannot show the menu: {error}");
                    }
                }
                Request::Clipboard(request) => {
                    if let Some((shown, _)) = cx.window(window)
                        && let Err(error) = shown.serve_clipboard(request)
                    {
                        tracing::warn!("cannot use the clipboard: {error}");
                    }
                }
            }
        }
        refresh(cx, window);
    }

    fn keep_draft(&self, cx: &mut Cx<Self>, window: WindowId) {
        let unsent = match cx.window(window) {
            Some((_, Pane::Compose(view))) => view.unsent(),
            _ => None,
        };
        if let Some(outgoing) = unsent
            && !main_view(cx, self.main).is_some_and(|main| main.save_draft(outgoing))
        {
            tracing::warn!("no account keeps drafts for this address");
        }
    }

    fn finish(&self, cx: &mut Cx<Self>) {
        let windows: Vec<WindowId> = cx.windows().collect();
        for window in windows {
            self.keep_draft(cx, window);
        }
        if let Some(view) = main_view(cx, self.main) {
            view.finish();
        }
    }
}

fn refresh(cx: &mut Cx<Mail>, window: WindowId) {
    if let Some((shown, pane)) = cx.window(window) {
        let (cursor, title) = match pane {
            Pane::Main(view) => (view.cursor(), view.title()),
            Pane::Compose(view) => (view.cursor(), view.title()),
        };
        shown.set_cursor(cursor);
        shown.set_title(&title);
        shown.request_redraw();
    }
    cx.refresh_menus();
}

impl Program for Mail {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let mut view = MailView::new(
            store_path(),
            Appearance::default().palette(),
            TextSystem::new(),
        )?;
        let title = view.title();
        let main = cx.open(Kind::Main, &title, WINDOW_SIZE, |appearance| {
            view.set_appearance(appearance);
            view.start();
            Pane::Main(Box::new(view))
        })?;
        let mut mail = Self {
            main,
            apps: None,
            picker: Picker::new()?,
        };
        for argument in arguments {
            mail.open_argument(cx, &argument);
        }
        mail.serve(cx, main);
        Ok(mail)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        Menus {
            menus: match cx.view(self.main) {
                Some(Pane::Main(view)) => menus::app_menus(view),
                _ => Vec::new(),
            },
            window: Vec::new(),
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, window: WindowId, command: &str) {
        let handled = match cx.window(window) {
            Some((_, Pane::Main(view))) => view.menu_action(command),
            Some((_, Pane::Compose(view))) => view.menu_action(command),
            None => false,
        };
        if !handled && window != self.main {
            if let Some(view) = main_view(cx, self.main) {
                view.menu_action(command);
            }
            let main = self.main;
            self.serve(cx, main);
        }
        self.serve(cx, window);
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, _directory: &Path) {
        for argument in arguments {
            self.open_argument(cx, &argument);
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        self.serve(cx, window);
    }

    fn should_close(&mut self, cx: &mut Cx<Self>, window: WindowId) -> bool {
        if window == self.main {
            self.finish(cx);
            cx.quit();
        } else {
            self.keep_draft(cx, window);
        }
        true
    }

    fn should_quit(&mut self, cx: &mut Cx<Self>) -> bool {
        self.finish(cx);
        true
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.picker.fd()]
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, _index: usize) {
        for (token, picked) in self.picker.take() {
            self.attach(cx, token, picked);
        }
    }
}

fn main() -> ExitCode {
    bande_app::run::<Mail>(&INFO)
}
