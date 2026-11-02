use std::collections::VecDeque;
use std::io::{self, BufReader};
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use hayro::hayro_interpret::font::{Glyph as PdfGlyph, GlyphRun};
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, SoftMask, interpret_page,
};
use hayro::hayro_syntax::page::Page;
use hayro::hayro_syntax::{LoadPdfError, Pdf};
use hayro::kurbo::{Affine, BezPath, Point, Rect};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings, render_into};

use crate::decode::read_file;
use crate::pdf::outline::{details, outline};
use crate::pdf::protocol::{self, Ask, Glyph, MAX_RENDER_PIXELS, Opened, Render, Reply};

const ADDRESS_SPACE: u64 = 6 << 30;
const MAX_GLYPHS: usize = 200_000;
const ASCENT: f64 = 800.0;
const DESCENT: f64 = -200.0;
const FALLBACK_ADVANCE: f64 = 500.0;

#[derive(Default)]
struct Queue {
    password: Option<String>,
    renders: VecDeque<Render>,
    texts: VecDeque<usize>,
    closed: bool,
}

enum Job {
    Open(String),
    Render(Render),
    Text(usize),
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next(&self, opened: bool) -> Option<Job> {
        let mut queue = self.lock();
        loop {
            if let Some(password) = queue.password.take() {
                return Some(Job::Open(password));
            }
            if opened {
                if let Some(render) = queue.renders.pop_front() {
                    return Some(Job::Render(render));
                }
                if let Some(page) = queue.texts.pop_front() {
                    return Some(Job::Text(page));
                }
            }
            if queue.closed {
                return None;
            }
            queue = self
                .ready
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

fn listen(shared: &Shared) {
    let mut input = BufReader::new(io::stdin().lock());
    while let Ok(Some(ask)) = protocol::read::<Ask>(&mut input) {
        let mut queue = shared.lock();
        match ask {
            Ask::Open { password } => queue.password = Some(password),
            Ask::Want { renders } => queue.renders = renders.into(),
            Ask::Text { pages } => {
                for page in pages {
                    if !queue.texts.contains(&page) {
                        queue.texts.push_back(page);
                    }
                }
            }
        }
        drop(queue);
        shared.ready.notify_all();
    }
    shared.lock().closed = true;
    shared.ready.notify_all();
}

fn confine() {
    let limit = rustix::process::Rlimit {
        current: Some(ADDRESS_SPACE),
        maximum: Some(ADDRESS_SPACE),
    };
    if let Err(error) = rustix::process::setrlimit(rustix::process::Resource::As, limit) {
        tracing::warn!("cannot limit the PDF renderer's memory: {error}");
    }
    if let Err(error) =
        rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL))
    {
        tracing::warn!("cannot tie the PDF renderer to Preview: {error}");
    }
}

fn send(reply: &Reply, payload: &[u8]) -> bool {
    protocol::write(&mut io::stdout().lock(), reply, payload).is_ok()
}

pub fn run(path: &Path) -> ExitCode {
    confine();
    let Ok(bytes) = read_file(path) else {
        send(&Reply::Failed, &[]);
        return ExitCode::FAILURE;
    };
    let bytes = Arc::new(bytes);
    let shared = Arc::new(Shared::default());
    let listener = Arc::clone(&shared);
    if thread::Builder::new()
        .name("requests".to_owned())
        .spawn(move || listen(&listener))
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    let mut retry = false;
    while let Some(job) = shared.next(false) {
        let Job::Open(password) = job else {
            continue;
        };
        match Pdf::new_with_password(Arc::clone(&bytes), &password) {
            Ok(pdf) => return serve(&pdf, &shared),
            Err(LoadPdfError::Decryption(
                hayro::hayro_syntax::DecryptionError::PasswordProtected,
            )) => {
                send(&Reply::Locked { retry }, &[]);
                retry = true;
            }
            Err(_) => {
                send(&Reply::Failed, &[]);
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

fn version(pdf: &Pdf) -> String {
    let named = format!("{:?}", pdf.version());
    let digits: String = named.chars().filter(char::is_ascii_digit).collect();
    match digits.split_at_checked(1) {
        Some((major, minor)) if !minor.is_empty() => format!("{major}.{minor}"),
        _ => digits,
    }
}

fn serve(pdf: &Pdf, shared: &Shared) -> ExitCode {
    let pages = pdf.pages();
    let opened = Opened {
        pages: pages.iter().map(Page::render_dimensions).collect(),
        outline: outline(pdf),
        version: version(pdf),
        details: details(pdf),
    };
    if !send(&Reply::Opened(opened), &[]) {
        return ExitCode::FAILURE;
    }
    let cache = RenderCache::new();
    let text_cache = InterpreterCache::new();
    let settings = InterpreterSettings::default();
    while let Some(job) = shared.next(true) {
        let sent = match job {
            Job::Open(_) => true,
            Job::Render(render) => match pages.get(render.page) {
                Some(page) => {
                    let pixels = draw(page, &cache, &settings, &render);
                    send(&Reply::Rendered { render }, &pixels)
                }
                None => true,
            },
            Job::Text(index) => match pages.get(index) {
                Some(page) => send(
                    &Reply::Text {
                        page: index,
                        glyphs: text(page, &text_cache, &settings),
                    },
                    &[],
                ),
                None => true,
            },
        };
        if !sent {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn draw<'a>(
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    render: &Render,
) -> Vec<u8> {
    let pixels = u64::from(render.width) * u64::from(render.height);
    let fits = pixels > 0
        && pixels <= MAX_RENDER_PIXELS
        && render.width <= u32::from(u16::MAX)
        && render.height <= u32::from(u16::MAX)
        && render.scale.is_finite()
        && render.scale > 0.0;
    if !fits {
        return vec![255; (pixels.min(MAX_RENDER_PIXELS) * 4) as usize];
    }
    let (width, height) = (render.width as u16, render.height as u16);
    let mut context = RenderContext::new(width, height);
    let transform = Affine::translate((-f64::from(render.x), -f64::from(render.y)))
        * Affine::scale(f64::from(render.scale))
        * page.initial_transform(true).to_kurbo();
    render_into(
        page,
        cache,
        settings,
        &RenderSettings::default(),
        &mut context,
        transform,
    );
    context.flush();
    let mut pixmap = Pixmap::new(width, height);
    context.render_with(
        &mut pixmap,
        &mut Resources::default(),
        RasterizerSettings {
            target_init: TargetInit::Clear(WHITE),
            ..Default::default()
        },
    );
    pixmap.data_as_u8_slice().to_vec()
}

struct TextDevice {
    glyphs: Vec<Glyph>,
    last_fill: Option<[f32; 4]>,
}

impl TextDevice {
    fn push(&mut self, text: String, rect: Rect) {
        let height = rect.height().max(1.0);
        if let Some(previous) = self.glyphs.last() {
            let [left, top, right, bottom] = previous.rect;
            let previous_middle = f64::from(top + bottom) / 2.0;
            let middle = (rect.y0 + rect.y1) / 2.0;
            let separator = if (middle - previous_middle).abs() > height * 0.6 {
                Some("\n")
            } else if rect.x0 - f64::from(right) > height * 0.15
                && !previous.text.ends_with(' ')
                && !text.starts_with(' ')
            {
                Some(" ")
            } else {
                None
            };
            if let Some(separator) = separator {
                let gap = [right, top, right.max(rect.x0 as f32).max(left), bottom];
                self.glyphs.push(Glyph {
                    text: separator.to_owned(),
                    rect: gap,
                });
            }
        }
        self.glyphs.push(Glyph {
            text,
            rect: [
                rect.x0 as f32,
                rect.y0 as f32,
                rect.x1 as f32,
                rect.y1 as f32,
            ],
        });
    }
}

impl<'a> Device<'a> for TextDevice {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, mode: &DrawMode) {
        let mut first = true;
        for glyph in run.glyphs() {
            if self.glyphs.len() >= MAX_GLYPHS {
                return;
            }
            let advance = match &**glyph {
                PdfGlyph::Outline(outline) => outline
                    .advance_width()
                    .map_or(FALLBACK_ADVANCE, f64::from)
                    .max(1.0),
                PdfGlyph::Type3(_) => FALLBACK_ADVANCE,
            };
            let transform = props.transform * glyph.transform();
            let corners = [
                Point::new(0.0, DESCENT),
                Point::new(advance, DESCENT),
                Point::new(0.0, ASCENT),
                Point::new(advance, ASCENT),
            ]
            .map(|corner| transform * corner);
            let rect = corners
                .iter()
                .fold(Rect::from_points(corners[0], corners[0]), |rect, point| {
                    rect.union_pt(*point)
                });
            let as_array = [
                rect.x0 as f32,
                rect.y0 as f32,
                rect.x1 as f32,
                rect.y1 as f32,
            ];
            if first {
                first = false;
                if matches!(mode, DrawMode::Stroke(_)) && self.last_fill == Some(as_array) {
                    return;
                }
                if !matches!(mode, DrawMode::Stroke(_)) {
                    self.last_fill = Some(as_array);
                }
            }
            let text = match glyph.as_unicode() {
                Some(BfString::Char(character)) => character.to_string(),
                Some(BfString::String(text)) => text,
                None => continue,
            };
            if !rect.x0.is_finite() || !rect.y0.is_finite() || text.chars().all(char::is_control) {
                continue;
            }
            self.push(text, rect);
        }
    }
}

fn text<'a>(
    page: &'a Page<'a>,
    cache: &InterpreterCache<'a>,
    settings: &InterpreterSettings,
) -> Vec<Glyph> {
    let (width, height) = page.render_dimensions();
    let mut context = Context::new(
        page.initial_transform(true).to_kurbo(),
        Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
        cache,
        page.xref(),
        settings.clone(),
    );
    let mut device = TextDevice {
        glyphs: Vec::new(),
        last_fill: None,
    };
    interpret_page(page, &mut context, &mut device);
    device.glyphs
}
