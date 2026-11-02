use std::collections::HashMap;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;

use rustix::event::{EventfdFlags, PollFd, PollFlags, eventfd, poll};
use web_content::{Fonts, Page};
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
        width: f32,
    },
    Resize {
        view: ViewId,
        width: f32,
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

#[derive(Default)]
struct Tab {
    shown: Option<(LoadId, Url, Page)>,
    pending: Option<(LoadId, FetchId, Url)>,
    width: f32,
    resized: bool,
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

    fn command(&mut self, command: Command) {
        match command {
            Command::Load {
                view,
                load,
                url,
                width,
            } => {
                let fetch = self.fetcher.fetch(url.clone());
                let tab = self.tabs.entry(view).or_default();
                if let Some((_, previous, _)) = tab.pending.replace((load, fetch, url)) {
                    self.fetcher.cancel(previous);
                }
                tab.resized |= tab.width != width;
                tab.width = width;
            }
            Command::Resize { view, width } => {
                let tab = self.tabs.entry(view).or_default();
                tab.resized |= tab.width != width;
                tab.width = width;
            }
            Command::Fragment { view, load, name } => {
                let y = self
                    .tabs
                    .get(&view)
                    .and_then(|tab| tab.shown.as_ref())
                    .filter(|(shown, _, _)| *shown == load)
                    .map(|(_, _, page)| page.fragment_y(&name).unwrap_or(0.0));
                if let Some(y) = y {
                    self.send(view, load, Outcome::Scroll(y));
                }
            }
            Command::Stop { view } => {
                if let Some((_, fetch, _)) =
                    self.tabs.get_mut(&view).and_then(|tab| tab.pending.take())
                {
                    self.fetcher.cancel(fetch);
                }
            }
            Command::Close { view } => {
                if let Some((_, fetch, _)) = self.tabs.remove(&view).and_then(|tab| tab.pending) {
                    self.fetcher.cancel(fetch);
                }
            }
            Command::Quit => {}
        }
    }

    fn fetched(&mut self, fetch: FetchId, result: Result<Response, FetchError>, fonts: &mut Fonts) {
        let Some((view, tab)) = self.tabs.iter_mut().find(|(_, tab)| {
            tab.pending
                .as_ref()
                .is_some_and(|(_, pending, _)| *pending == fetch)
        }) else {
            return;
        };
        let view = *view;
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
        let mut page = Page::new(document, tab.width.max(1.0), fonts);
        tab.resized = false;
        let url = response.url;
        let links = Arc::new(links(&page, &url));
        let frame = Frame {
            title: page.title().filter(|title| !title.is_empty()),
            list: Arc::new(take_list(&mut page)),
            links,
            fragment: url.fragment().and_then(|name| page.fragment_y(name)),
            url: url.clone(),
        };
        tab.shown = Some((load, url, page));
        self.send(view, load, Outcome::Frame(frame));
    }

    fn relayout(&mut self, fonts: &mut Fonts) {
        let mut relaid = Vec::new();
        for (view, tab) in &mut self.tabs {
            if !std::mem::take(&mut tab.resized) {
                continue;
            }
            if let Some((load, url, page)) = &mut tab.shown {
                page.resize(tab.width.max(1.0), fonts);
                let links = Arc::new(links(page, url));
                relaid.push((*view, *load, Arc::new(take_list(page)), links));
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

fn take_list(page: &mut Page) -> DisplayList {
    let list = &page.display_list;
    let emptied = DisplayList {
        width: list.width,
        height: list.height,
        background: list.background,
        items: Vec::new(),
        glyphs: Vec::new(),
        fonts: Vec::new(),
    };
    std::mem::replace(&mut page.display_list, emptied)
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
