mod instance;
mod menus;

use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_mail::L10N;
use bande_mail::compose::{Draft, from_mailto};
use bande_mail::store::MessageId;
use bande_mail::ui::{COMPOSE_SIZE, ComposeRequest, ComposeView, MailView, Request, SIDEBAR_WIDTH};
use unbound_app_menu::{AppMenu, Standard};
use unbound_notify::Urgency;
use unbound_notify::client::{Request as Notification, notify};
use unbound_toolkit::{
    Appearance, FrameRegions, MenuPlacement, Rect, Role, Scene, Surfaces, TextSystem, Window,
    WindowEvent,
};
use unbound_xdg::{Apps, config_dirs, data_dirs, data_home};

use crate::instance::Instance;

const APP_ID: &str = "org.bande.Mail";
const WINDOW_SIZE: (u32, u32) = (1180, 720);
const NEW_MAIL_SOUND: &str = "message-new-email";

enum Screen {
    Main(Box<MailView>),
    Compose(Box<ComposeView>),
}

struct Mail {
    windows: Vec<Window>,
    screens: Vec<Screen>,
    app_menu: Option<AppMenu>,
    instance: Option<Instance>,
    focused: usize,
    apps: Option<Apps>,
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

impl Mail {
    fn main_view(&mut self) -> Option<&mut MailView> {
        self.screens.iter_mut().find_map(|screen| match screen {
            Screen::Main(view) => Some(view.as_mut()),
            Screen::Compose(_) => None,
        })
    }

    fn apps(&mut self) -> &Apps {
        self.apps
            .get_or_insert_with(|| Apps::load(data_dirs(), &config_dirs()))
    }

    fn open_compose(&mut self, draft: Draft, answering: Option<MessageId>) -> Option<usize> {
        let senders = self
            .main_view()
            .map(|view| {
                view.accounts()
                    .iter()
                    .map(|account| account.address())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let appearance = self.windows[0].appearance();
        let mut view = ComposeView::new(
            draft,
            answering,
            senders,
            appearance.palette(),
            TextSystem::new(),
        );
        view.set_appearance(appearance);
        let role = Role::toplevel(&view.title(), APP_ID);
        match self.windows[0].open_beside(role, COMPOSE_SIZE) {
            Ok(mut window) => {
                window.set_frame_regions(FrameRegions {
                    sidebar: 0.0,
                    toolbar: 0.0,
                });
                window.set_header(view.header());
                self.windows.push(window);
                self.screens.push(Screen::Compose(Box::new(view)));
                Some(self.windows.len() - 1)
            }
            Err(error) => {
                tracing::warn!("cannot open a message window: {error}");
                None
            }
        }
    }

    fn open_argument(&mut self, argument: &str) {
        if let Some(draft) = from_mailto(argument) {
            if let Some(view) = self.main_view() {
                view.compose(draft);
            }
            self.serve(0);
        }
    }

    fn open_url(&mut self, index: usize, url: &str) {
        if from_mailto(url).is_some() {
            self.open_argument(url);
            return;
        }
        let scheme = url
            .split(':')
            .next()
            .unwrap_or("https")
            .to_ascii_lowercase();
        let token = self.windows[index].activation_token(APP_ID).ok();
        let entry = self
            .apps()
            .default_for(&format!("x-scheme-handler/{scheme}"))
            .cloned();
        match entry {
            Some(entry) => {
                if let Err(error) =
                    unbound_xdg::launch_with_arguments(&entry, &[url.to_owned()], token.as_deref())
                {
                    tracing::warn!("cannot open {url}: {error}");
                }
            }
            None => tracing::warn!("no app opens {scheme} links"),
        }
    }

    fn open_file(&mut self, index: usize, path: &Path) {
        let token = self.windows[index].activation_token(APP_ID).ok();
        let entry = self.apps().default_for_path(path).cloned();
        match entry {
            Some(entry) => {
                if let Err(error) =
                    unbound_xdg::launch_with(&entry, &[path.to_path_buf()], token.as_deref())
                {
                    tracing::warn!("cannot open {}: {error}", path.display());
                }
            }
            None => tracing::warn!("no app opens {}", path.display()),
        }
    }

    fn serve_compose(&mut self, index: usize) {
        let Screen::Compose(view) = &mut self.screens[index] else {
            return;
        };
        let requests = view.take_requests();
        let sent = view.take_sent();
        let finished = view.finished();
        for request in requests {
            let window = &mut self.windows[index];
            let result = match request {
                ComposeRequest::ShowMenu(menu, at) => {
                    window.show_menu(menu, MenuPlacement::AtPoint(at))
                }
                ComposeRequest::Clipboard(request) => window.serve_clipboard(request),
            };
            if let Err(error) = result {
                tracing::warn!("cannot serve the message window: {error}");
            }
        }
        if let Some((draft, answering)) = sent {
            let delivered = self
                .main_view()
                .is_some_and(|main| main.send(draft, answering));
            if !delivered {
                tracing::warn!("no account sends from this address");
            }
        }
        if finished {
            self.close(index);
        } else {
            self.refresh(index);
        }
    }

    fn serve(&mut self, index: usize) {
        if index >= self.screens.len() {
            return;
        }
        let requests = match &mut self.screens[index] {
            Screen::Main(view) => view.take_requests(),
            Screen::Compose(_) => return self.serve_compose(index),
        };
        for request in requests {
            match request {
                Request::Compose { draft, answering } => {
                    self.open_compose(*draft, answering);
                }
                Request::OpenLink(url) => self.open_url(index, &url),
                Request::OpenAttachment(path) => self.open_file(index, &path),
                Request::Notify { summary, body } => announce(summary, body),
                Request::NotSent { error, draft } => {
                    if let Some(opened) = self.open_compose(*draft, None)
                        && let Screen::Compose(view) = &mut self.screens[opened]
                    {
                        view.show_error(&error);
                    }
                }
                Request::ShowMenu(menu, at) => {
                    if let Err(error) =
                        self.windows[index].show_menu(menu, MenuPlacement::AtPoint(at))
                    {
                        tracing::warn!("cannot show the menu: {error}");
                    }
                }
                Request::Clipboard(request) => {
                    if let Err(error) = self.windows[index].serve_clipboard(request) {
                        tracing::warn!("cannot use the clipboard: {error}");
                    }
                }
            }
        }
        self.refresh(index);
    }

    fn refresh(&mut self, index: usize) {
        if let (Some(window), Some(screen)) = (self.windows.get_mut(index), self.screens.get(index))
        {
            let (cursor, title) = match screen {
                Screen::Main(view) => (view.cursor(), view.title()),
                Screen::Compose(view) => (view.cursor(), view.title()),
            };
            window.set_cursor(cursor);
            window.set_title(&title);
            window.request_redraw();
        }
        self.publish_menus();
    }

    fn publish_menus(&mut self) {
        let menus = match self.screens.first() {
            Some(Screen::Main(view)) => menus::app_menus(view),
            _ => return,
        };
        if let Some(app_menu) = &self.app_menu
            && let Err(error) = app_menu.update(menus)
        {
            tracing::warn!("cannot update the menus: {error}");
        }
    }

    fn close(&mut self, index: usize) {
        if index == 0 {
            self.windows[0].close();
            return;
        }
        self.windows.remove(index);
        self.screens.remove(index);
        self.focused = 0;
    }

    fn menu_action(&mut self, index: usize, action: &str) {
        let window = &mut self.windows[index];
        let result = match Standard::parse(action) {
            Some(Standard::Hide | Standard::Minimize) => window.minimize(),
            Some(Standard::Zoom) => window.toggle_zoom(),
            Some(Standard::Quit) => {
                self.windows[0].close();
                Ok(())
            }
            Some(Standard::CloseWindow) => {
                self.close(index);
                return;
            }
            None => {
                let handled = match &mut self.screens[index] {
                    Screen::Main(view) => view.menu_action(action),
                    Screen::Compose(view) => view.menu_action(action),
                };
                if !handled && index > 0 {
                    if let Some(view) = self.main_view() {
                        view.menu_action(action);
                    }
                    self.serve(0);
                }
                self.serve(index);
                Ok(())
            }
        };
        if let Err(error) = result {
            tracing::warn!("cannot {action}: {error}");
        }
    }

    fn appearance(&mut self, appearance: Appearance) {
        for (window, screen) in self.windows.iter_mut().zip(&mut self.screens) {
            match screen {
                Screen::Main(view) => view.set_appearance(appearance),
                Screen::Compose(view) => view.set_appearance(appearance),
            }
            window.request_redraw();
        }
    }
}

impl Surfaces for Mail {
    fn windows(&self) -> &[Window] {
        &self.windows
    }

    fn windows_mut(&mut self) -> &mut [Window] {
        &mut self.windows
    }

    fn scene(&mut self, index: usize) -> Scene {
        let window = &mut self.windows[index];
        let full_screen = window.header_height() == 0;
        let (width, height) = window.size();
        match &mut self.screens[index] {
            Screen::Main(view) => {
                view.set_full_screen(full_screen);
                let scene = view.scene(width, height);
                window.set_header(view.header());
                scene
            }
            Screen::Compose(view) => {
                let scene = view.scene(width, height);
                window.set_header(view.header());
                scene
            }
        }
    }

    fn event(&mut self, index: usize, event: WindowEvent) {
        if index >= self.screens.len() {
            return;
        }
        let changed = match event {
            WindowEvent::AppearanceChanged(appearance) => {
                self.appearance(appearance);
                false
            }
            WindowEvent::CloseRequested => {
                self.close(index);
                return;
            }
            WindowEvent::QuitRequested => {
                self.windows[0].close();
                return;
            }
            WindowEvent::Focus(focused) => {
                if focused {
                    self.focused = index;
                    if let Some(Err(error)) = self.app_menu.as_ref().map(AppMenu::claim) {
                        tracing::warn!("{error}");
                    }
                }
                true
            }
            WindowEvent::Input(input) => match &mut self.screens[index] {
                Screen::Main(view) => view.handle(&input),
                Screen::Compose(view) => view.handle(&input),
            },
            WindowEvent::Accessibility(request) => match &mut self.screens[index] {
                Screen::Main(view) => view.perform(&request),
                Screen::Compose(view) => view.perform(&request),
            },
            WindowEvent::Pasted(text) => match &mut self.screens[index] {
                Screen::Main(view) => view.pasted(&text),
                Screen::Compose(view) => view.pasted(&text),
            },
            WindowEvent::MenuAction(action) => {
                self.menu_action(index, &action);
                return;
            }
            WindowEvent::MenuClosed => match &mut self.screens[index] {
                Screen::Main(view) => view.menu_closed(),
                Screen::Compose(view) => view.menu_closed(),
            },
            _ => false,
        };
        self.serve(index);
        if changed {
            self.refresh(index);
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = Vec::new();
        if let Some(Screen::Main(view)) = self.screens.first() {
            sources.extend(view.fds());
        }
        sources.extend(self.app_menu.as_ref().map(AppMenu::fd));
        sources.extend(self.instance.as_ref().map(Instance::fd));
        sources
    }

    fn source_ready(&mut self, index: usize) {
        let mail = usize::from(matches!(self.screens.first(), Some(Screen::Main(_))));
        let menu = mail + usize::from(self.app_menu.is_some());
        if index < mail {
            if self.main_view().is_some_and(MailView::engine_ready) {
                self.serve(0);
            }
        } else if index < menu {
            let actions = self
                .app_menu
                .as_mut()
                .map(AppMenu::take_actions)
                .unwrap_or_default();
            for action in actions {
                let focused = self.focused.min(self.windows.len() - 1);
                self.menu_action(focused, &action);
            }
        } else {
            let opened = self
                .instance
                .as_mut()
                .map(Instance::take)
                .unwrap_or_default();
            for argument in opened {
                self.open_argument(&argument);
            }
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.screens
            .iter()
            .filter_map(|screen| match screen {
                Screen::Main(view) => view.deadline(),
                Screen::Compose(view) => view.deadline(),
            })
            .min()
    }

    fn tick(&mut self, now: Instant) {
        for (window, screen) in self.windows.iter_mut().zip(&self.screens) {
            let due = match screen {
                Screen::Main(view) => view.deadline(),
                Screen::Compose(view) => view.deadline(),
            };
            if due.is_some_and(|due| due <= now) {
                window.request_redraw();
            }
        }
    }

    fn animating(&self, index: usize) -> bool {
        match self.screens.get(index) {
            Some(Screen::Main(view)) => view.animating(),
            Some(Screen::Compose(view)) => view.animating(),
            None => false,
        }
    }

    fn accessibility(&self, index: usize) -> Option<TreeUpdate> {
        match self.screens.get(index)? {
            Screen::Main(view) => Some(view.accessibility()),
            Screen::Compose(view) => Some(view.accessibility()),
        }
    }

    fn text_input_area(&self, index: usize) -> Option<Rect> {
        match self.screens.get(index)? {
            Screen::Main(view) => view.text_input_area(),
            Screen::Compose(view) => view.text_input_area(),
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|argument| argument == "--version")
    {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let instance = match Instance::claim(&arguments) {
        Ok(Some(instance)) => Some(instance),
        Ok(None) => return Ok(()),
        Err(error) => {
            tracing::warn!("cannot claim {APP_ID}: {error}");
            None
        }
    };
    let palette = Appearance::default().palette();
    let mut view = MailView::new(store_path(), palette, TextSystem::new())?;
    let mut window = Window::open(Role::toplevel(&view.title(), APP_ID), WINDOW_SIZE)?;
    view.set_appearance(window.appearance());
    window.set_frame_regions(FrameRegions {
        sidebar: SIDEBAR_WIDTH,
        toolbar: 0.0,
    });
    window.set_header(view.header());
    view.start();
    let app_menu = AppMenu::export(APP_ID, menus::app_menus(&view))
        .inspect_err(|error| tracing::warn!("{error}"))
        .ok();
    let mut mail = Mail {
        windows: vec![window],
        screens: vec![Screen::Main(Box::new(view))],
        app_menu,
        instance,
        focused: 0,
        apps: None,
    };
    for argument in arguments {
        mail.open_argument(&argument);
    }
    mail.serve(0);
    unbound_toolkit::run_surfaces(&mut mail)?;
    Ok(())
}

fn main() -> ExitCode {
    unbound_log::init("bande-mail");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
