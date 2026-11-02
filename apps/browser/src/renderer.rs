use std::collections::HashMap;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;

use rustix::event::{EventfdFlags, PollFd, PollFlags, eventfd, poll};
use web_content::{Fonts, Page, Viewport};
use web_net::{FetchError, FetchId, Fetcher, Response, Url};
use web_paint::{DisplayList, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ViewId(pub(crate) u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LoadId(pub(crate) u64);

pub(crate) enum Command {
    Load {
        view: ViewId,
        load: LoadId,
        url: Url,
        viewport: Viewport,
    },
    Resize {
        view: ViewId,
        viewport: Viewport,
    },
    Hover {
        view: ViewId,
        load: LoadId,
        x: f32,
        y: f32,
    },
    Fragment {
        view: ViewId,
        load: LoadId,
        name: String,
    },
    Stop {
        view: ViewId,
    },
    Close {
        view: ViewId,
    },
    Quit,
}

pub(crate) struct Links {
    pub(crate) areas: Vec<(Rect, usize)>,
    pub(crate) targets: Vec<Url>,
}

impl Links {
    pub(crate) fn at(&self, x: f32, y: f32) -> Option<&Url> {
        self.areas
            .iter()
            .rev()
            .find(|(rect, _)| {
                x >= rect.x && y >= rect.y && x < rect.x + rect.width && y < rect.y + rect.height
            })
            .and_then(|(_, target)| self.targets.get(*target))
    }
}

pub(crate) struct Frame {
    pub(crate) url: Url,
    pub(crate) title: Option<String>,
    pub(crate) list: Arc<DisplayList>,
    pub(crate) links: Arc<Links>,
    pub(crate) fragment: Option<f32>,
}

pub(crate) enum Outcome {
    Frame(Frame),
    Relaid(Arc<DisplayList>, Arc<Links>),
    Scroll(f32),
    Failed { url: Url, message: String },
    Unsupported { url: Url, kind: String },
}

pub(crate) struct Update {
    pub(crate) view: ViewId,
    pub(crate) load: LoadId,
    pub(crate) outcome: Outcome,
}

#[derive(Clone)]
pub(crate) struct Link {
    commands: Sender<Command>,
    signal: Arc<OwnedFd>,
}

impl Link {
    pub(crate) fn send(&self, command: Command) {
        if self.commands.send(command).is_ok() {
            signal(&self.signal);
        }
    }
}

pub(crate) struct Renderer {
    link: Link,
    ready: Arc<OwnedFd>,
    updates: Receiver<Update>,
    thread: Option<JoinHandle<()>>,
}

impl Renderer {
    pub(crate) fn new() -> io::Result<Self> {
        let flags = EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK;
        let signal = Arc::new(eventfd(0, flags)?);
        let ready = Arc::new(eventfd(0, flags)?);
        let fetcher = Fetcher::new()?;
        let (commands, received) = mpsc::channel();
        let (sent, updates) = mpsc::channel();
        let mut thread = Thread {
            commands: received,
            signal: Arc::clone(&signal),
            updates: sent,
            ready: Arc::clone(&ready),
            fetcher,
            tabs: HashMap::new(),
        };
        let thread = std::thread::Builder::new()
            .name("renderer".into())
            .spawn(move || {
                if let Err(error) = thread.run() {
                    tracing::warn!("the renderer stopped: {error}");
                }
            })?;
        Ok(Self {
            link: Link { commands, signal },
            ready,
            updates,
            thread: Some(thread),
        })
    }

    pub(crate) fn link(&self) -> Link {
        self.link.clone()
    }

    pub(crate) fn fd(&self) -> BorrowedFd<'_> {
        self.ready.as_fd()
    }

    pub(crate) fn updates(&self) -> Vec<Update> {
        clear(&self.ready);
        self.updates.try_iter().collect()
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.link.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn signal(fd: &OwnedFd) {
    let _ = rustix::io::write(fd, &1u64.to_ne_bytes());
}

fn clear(fd: &OwnedFd) {
    let mut count = [0; 8];
    let _ = rustix::io::read(fd, &mut count);
}

const MAX_STYLESHEETS: usize = 64;
const MAX_IMAGES: usize = 256;

struct Shown {
    load: LoadId,
    url: Url,
    page: Page,
}

struct Styling {
    shown: Shown,
    fetches: Vec<(FetchId, String)>,
    arrived: Vec<(String, Vec<u8>)>,
    attempted: Vec<String>,
}

#[derive(Default)]
struct Tab {
    shown: Option<Shown>,
    pending: Option<(LoadId, FetchId, Url)>,
    styling: Option<Styling>,
    images: Vec<(FetchId, String)>,
    images_arrived: Vec<(String, Vec<u8>)>,
    images_attempted: usize,
    viewport: Option<Viewport>,
    resized: bool,
    hover: Option<(LoadId, f32, f32)>,
}

impl Tab {
    fn fetches(&self) -> impl Iterator<Item = FetchId> + '_ {
        let document = self.pending.as_ref().map(|(_, fetch, _)| *fetch);
        let sheets = self
            .styling
            .iter()
            .flat_map(|styling| styling.fetches.iter().map(|(fetch, _)| *fetch));
        let images = self.images.iter().map(|(fetch, _)| *fetch);
        document.into_iter().chain(sheets).chain(images)
    }

    fn set_viewport(&mut self, viewport: Viewport) {
        self.resized |= self
            .viewport
            .is_none_or(|current| current.width != viewport.width);
        self.viewport = Some(viewport);
    }
}

fn body(result: Result<Response, FetchError>) -> Vec<u8> {
    match result {
        Ok(response) if (200..300).contains(&response.status) => response.body,
        _ => Vec::new(),
    }
}

fn borrowed(arrived: &[(String, Vec<u8>)]) -> Vec<(&str, &[u8])> {
    arrived
        .iter()
        .map(|(href, bytes)| (href.as_str(), bytes.as_slice()))
        .collect()
}

fn resolved(base: &Url, href: &str) -> Option<Url> {
    base.join(href.trim())
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
}

struct Thread {
    commands: Receiver<Command>,
    signal: Arc<OwnedFd>,
    updates: Sender<Update>,
    ready: Arc<OwnedFd>,
    fetcher: Fetcher,
    tabs: HashMap<ViewId, Tab>,
}

impl Thread {
    fn run(&mut self) -> io::Result<()> {
        let mut fonts = Fonts::new();
        loop {
            let fetched = self.fetcher.fd();
            let mut sources = [
                PollFd::new(&*self.signal, PollFlags::IN),
                PollFd::new(&fetched, PollFlags::IN),
            ];
            match poll(&mut sources, None) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(error.into()),
            }
            clear(&self.signal);
            if !self.take_commands() {
                return Ok(());
            }
            for (fetch, result) in self.fetcher.completed() {
                self.fetched(fetch, result, &mut fonts);
            }
            let views: Vec<ViewId> = self.tabs.keys().copied().collect();
            for view in views {
                self.style(view, &mut fonts);
                self.add_images(view, &mut fonts);
            }
            self.relayout(&mut fonts);
        }
    }

    fn take_commands(&mut self) -> bool {
        loop {
            match self.commands.try_recv() {
                Ok(Command::Quit) => return false,
                Ok(command) => self.command(command),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn cancel(&mut self, view: ViewId) {
        let Some(tab) = self.tabs.get_mut(&view) else {
            return;
        };
        let fetches: Vec<FetchId> = tab.fetches().collect();
        tab.pending = None;
        tab.styling = None;
        tab.images.clear();
        tab.images_arrived.clear();
        for fetch in fetches {
            self.fetcher.cancel(fetch);
        }
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Load {
                view,
                load,
                url,
                viewport,
            } => {
                self.cancel(view);
                let fetch = self.fetcher.fetch(url.clone());
                let tab = self.tabs.entry(view).or_default();
                tab.pending = Some((load, fetch, url));
                tab.set_viewport(viewport);
            }
            Command::Resize { view, viewport } => {
                self.tabs.entry(view).or_default().set_viewport(viewport);
            }
            Command::Hover { view, load, x, y } => {
                if let Some(tab) = self.tabs.get_mut(&view) {
                    tab.hover = Some((load, x, y));
                }
            }
            Command::Fragment { view, load, name } => {
                let y = self
                    .tabs
                    .get(&view)
                    .and_then(|tab| tab.shown.as_ref())
                    .filter(|shown| shown.load == load)
                    .map(|shown| shown.page.fragment_y(&name).unwrap_or(0.0));
                if let Some(y) = y {
                    self.send(view, load, Outcome::Scroll(y));
                }
            }
            Command::Stop { view } => self.cancel(view),
            Command::Close { view } => {
                self.cancel(view);
                self.tabs.remove(&view);
            }
            Command::Quit => {}
        }
    }

    fn fetched(&mut self, fetch: FetchId, result: Result<Response, FetchError>, fonts: &mut Fonts) {
        let Some(view) = self
            .tabs
            .iter()
            .find(|(_, tab)| tab.fetches().any(|pending| pending == fetch))
            .map(|(view, _)| *view)
        else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(&view) else {
            return;
        };
        if let Some(index) = tab.images.iter().position(|(pending, _)| *pending == fetch) {
            let (_, source) = tab.images.swap_remove(index);
            tab.images_arrived.push((source, body(result)));
            return;
        }
        if let Some(styling) = &mut tab.styling
            && let Some(index) = styling
                .fetches
                .iter()
                .position(|(pending, _)| *pending == fetch)
        {
            let (_, href) = styling.fetches.swap_remove(index);
            styling.arrived.push((href, body(result)));
            return;
        }
        let Some((load, _, requested)) = tab.pending.take() else {
            return;
        };
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                let url = requested;
                let message = error.to_string();
                return self.send(view, load, Outcome::Failed { url, message });
            }
        };
        let charset =
            response
                .content_type
                .as_ref()
                .and_then(|content_type| match content_type.is_html() {
                    true => Some(content_type.charset.map(|encoding| encoding.name())),
                    false => None,
                });
        let charset = match (&response.content_type, charset) {
            (None, _) => None,
            (Some(_), Some(charset)) => charset,
            (Some(content_type), None) => {
                let kind = content_type.essence.clone();
                let url = response.url;
                return self.send(view, load, Outcome::Unsupported { url, kind });
            }
        };
        let document = web_content::parse_with_charset(&response.body, charset);
        let viewport = tab.viewport.unwrap_or(Viewport {
            width: 1.0,
            height: 1.0,
        });
        tab.resized = false;
        tab.images_attempted = 0;
        tab.styling = Some(Styling {
            shown: Shown {
                load,
                url: response.url,
                page: Page::with_viewport(document, viewport, fonts),
            },
            fetches: Vec::new(),
            arrived: Vec::new(),
            attempted: Vec::new(),
        });
    }

    fn style(&mut self, view: ViewId, fonts: &mut Fonts) {
        let Some(styling) = self
            .tabs
            .get_mut(&view)
            .and_then(|tab| tab.styling.as_mut())
        else {
            return;
        };
        if !styling.fetches.is_empty() {
            return;
        }
        if !styling.arrived.is_empty() {
            let arrived = std::mem::take(&mut styling.arrived);
            styling
                .shown
                .page
                .add_stylesheets(&borrowed(&arrived), fonts);
        }
        for href in styling.shown.page.stylesheet_links() {
            if styling.attempted.contains(&href) || styling.attempted.len() >= MAX_STYLESHEETS {
                continue;
            }
            styling.attempted.push(href.clone());
            match resolved(&styling.shown.url, &href) {
                Some(url) => styling.fetches.push((self.fetcher.fetch(url), href)),
                None => styling.arrived.push((href, Vec::new())),
            }
        }
        if !styling.fetches.is_empty() || !styling.arrived.is_empty() {
            return;
        }
        let Some(tab) = self.tabs.get_mut(&view) else {
            return;
        };
        let Some(Styling { shown, .. }) = tab.styling.take() else {
            return;
        };
        let frame = Frame {
            title: shown.page.title().filter(|title| !title.is_empty()),
            list: Arc::clone(&shown.page.display_list),
            links: Arc::new(links(&shown.page, &shown.url)),
            fragment: shown
                .url
                .fragment()
                .and_then(|name| shown.page.fragment_y(name)),
            url: shown.url.clone(),
        };
        let load = shown.load;
        tab.shown = Some(shown);
        self.request_images(view);
        self.send(view, load, Outcome::Frame(frame));
    }

    fn request_images(&mut self, view: ViewId) {
        let Some(tab) = self.tabs.get_mut(&view) else {
            return;
        };
        let Some(shown) = &tab.shown else {
            return;
        };
        let requested: Vec<String> = tab
            .images
            .iter()
            .map(|(_, source)| source.clone())
            .collect();
        for source in shown.page.image_links() {
            if requested.contains(&source) || tab.images_attempted >= MAX_IMAGES {
                continue;
            }
            tab.images_attempted += 1;
            match resolved(&shown.url, &source) {
                Some(url) => tab.images.push((self.fetcher.fetch(url), source)),
                None => tab.images_arrived.push((source, Vec::new())),
            }
        }
    }

    fn add_images(&mut self, view: ViewId, fonts: &mut Fonts) {
        let Some(tab) = self.tabs.get_mut(&view) else {
            return;
        };
        if tab.images_arrived.is_empty() {
            return;
        }
        let arrived = std::mem::take(&mut tab.images_arrived);
        let Some(shown) = &mut tab.shown else {
            return;
        };
        shown.page.add_images(&borrowed(&arrived), fonts);
        let list = Arc::clone(&shown.page.display_list);
        let links = Arc::new(links(&shown.page, &shown.url));
        let load = shown.load;
        self.send(view, load, Outcome::Relaid(list, links));
    }

    fn relayout(&mut self, fonts: &mut Fonts) {
        let mut relaid = Vec::new();
        for (view, tab) in &mut self.tabs {
            let resized = std::mem::take(&mut tab.resized);
            let hover = tab.hover.take();
            let width = tab.viewport.map(|viewport| viewport.width.max(1.0));
            let Some(shown) = &mut tab.shown else {
                continue;
            };
            let mut changed = false;
            if resized && let Some(width) = width {
                shown.page.resize(width, fonts);
                changed = true;
            }
            if let Some((load, x, y)) = hover
                && load == shown.load
            {
                changed |= shown.page.hover_at(x, y, fonts);
            }
            if changed {
                let list = Arc::clone(&shown.page.display_list);
                let links = Arc::new(links(&shown.page, &shown.url));
                relaid.push((*view, shown.load, list, links));
            }
        }
        for (view, load, list, links) in relaid {
            self.send(view, load, Outcome::Relaid(list, links));
        }
    }

    fn send(&self, view: ViewId, load: LoadId, outcome: Outcome) {
        if self
            .updates
            .send(Update {
                view,
                load,
                outcome,
            })
            .is_ok()
        {
            signal(&self.ready);
        }
    }
}

fn links(page: &Page, base: &Url) -> Links {
    let mut targets: Vec<Url> = Vec::new();
    let mut resolved = HashMap::new();
    let mut areas = Vec::new();
    for item in &page.display_list.items {
        let web_paint::Item::Glyphs {
            rect,
            node: Some(node),
            ..
        } = item
        else {
            continue;
        };
        let target = *resolved.entry(*node).or_insert_with(|| {
            let url = page
                .link_of(*node)
                .and_then(|href| base.join(href.trim()).ok())
                .filter(|url| matches!(url.scheme(), "http" | "https"))?;
            targets.push(url);
            Some(targets.len() - 1)
        });
        if let Some(target) = target {
            areas.push((*rect, target));
        }
    }
    Links { areas, targets }
}
