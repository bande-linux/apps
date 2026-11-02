# Browser: a Bande-owned Rust web engine and a Safari-like app

The owner's note (`NOTES/browser.md`) asks for a browser that is fast, secure, private by default,
mostly Rust, independent of Google, extensible (uBlock Origin-class blocking), without telemetry
or ads, and maintained mostly by agents. The owner adds: "write our own, fully in Rust and super
fast and efficient". This plan makes that concrete: what "our own" means, what it costs, the
phases that are each usable, and the app. Delete it once its milestones are done.

## The ambition, honestly

- **Full web compatibility (YouTube, Google Docs, Figma, banking) is team-years.** Servo has
  had a decade and dozens of engineers; Ladybird has had about eight full-time engineers since
  2022 and targets its first alpha in 2026. Neither is a daily browser yet. Expect 15–30
  engineer-years to reach 90%+ of real browsing, plus a permanent security team's workload.
- **A fast, efficient, macOS-quality browser for a subset of the web is achievable**: documents,
  wikis, articles, forums, docs sites and server-rendered apps. That's about 2–3 engineer-years for
  phases 1–3 below, if we reuse the commodity parts.
- Every phase ships something usable, and has **Open in System Browser** for pages it can't handle.

## Principle: what "our own engine" means

Every real engine reuses commodity, spec-locked parts: Chromium uses Skia, HarfBuzz, ICU,
libxml, BoringSSL; Firefox uses Stylo, HarfBuzz, Skia, NSS, dav1d. A rewrite of a spec-locked
part (a tokenizer the spec defines to the byte, a TLS stack, a codec) buys nothing but bugs.
**Owning the engine means owning:** the architecture, the DOM, layout and its integration, the
rendering pipeline, the process model and sandbox, the resource and memory budget, and the
scheduler. Those decide speed and efficiency. Everything else is a dependency we keep current.

## Build vs reuse, component by component

Sizes are lines of Rust in the published crates (fetched and counted, November 2026); "eng-mo" is
engineer-months to match or beat it.

| Part | Crate, size | Good: copy | Bad: avoid | Effort to own | Verdict |
|---|---|---|---|---|---|
| HTML tokenizer, tree builder | html5ever 0.40 (9.8k) + markup5ever (1.3k), 5 `unsafe` | the spec's state machines exactly; tendril zero-copy strings; script pause for `document.write` | `TreeSink` takes `&self`, so the DOM needs interior mutability; a global `string_cache` atom set behind a mutex; no speculative preload scanner | 3–4 eng-mo, html5lib-tests decide | **dependency**; add our own preload scanner |
| DOM | Blitz `blitz-dom` (28.9k, mixes DOM, layout, paint) | slab of nodes with integer ids; Stylo `TElement` glue (`stylo.rs`); changed-node set for incremental work | each `Node` holds a raw `*mut NodeTree` with `unsafe impl Send/Sync`, accessors that `panic!`, ~10 `RefCell`/`ThinVec` layout and paint fields, a boxed text layout and a taffy cache: an estimated 1–2 KB per element | 2–3 eng-mo core, 6–12 more with JS bindings | **own** (start from a Blitz fork): data-oriented arena, 64–96 B node core, layout and paint in side tables only for rendered nodes, generational ids for JS |
| CSS parsing | cssparser 0.38 (6.5k) | spec tokenizer, zero-copy `CowRcStr` | none that matter | 2 eng-mo | **dependency** |
| Selectors, cascade, style | Stylo 0.22 (137k + generated code, 532 `unsafe`), selectors (9k) | the best style system there is (Firefox ships it): bloom-filtered matching, style sharing cache, rule tree, snapshot-based invalidation for incremental restyle, parallel traversal | global state (the `STYLE_THREAD_POOL` static, static prefs, atoms); Gecko/Servo `cfg` doubling; large `ComputedValues` (shared by `Arc`, fine); the build runs Python with Mako; heavy compile | 5–10 eng-years | **dependency**: own only the glue (3–5k), sequential traversal for small documents, verify parked rayon threads cause no idle wake-ups |
| Layout | taffy 0.15 (31k): block, flex, grid; Blitz glue: stylo_taffy (2.4k), `layout/` (tables as grid, inline via parley) | taffy's flex and grid algorithms follow the spec and are generic over a tree trait; per-node cache (final layout plus 9 measure entries) | no floats in core (feature-flagged), no tables, no writing modes, no fragmentation; Blitz converts every `ComputedValues` into a `taffy::Style` (two style copies); layout state lives in the DOM | 12–24 eng-mo for CSS 2.1 + flex + grid at good quality | **own** the box and fragment trees (immutable, cacheable fragments as in Servo's Layout 2020 and LayoutNG), block flow, margin collapsing, inline, floats, positioning, tables (25–40k); **dependency** for taffy's flex and grid algorithms |
| Text | parley 0.12 (20.7k) + harfrust (34k HarfBuzz port) + skrifa (38k) + fontique + ICU4X segmenter | the Bande toolkit's own text stack at the same versions; bidi, shaping, line breaking, per-line widths | per-paragraph allocations; float exclusions and some CSS text features missing | 2+ eng-years | **dependency**; contribute missing CSS features upstream |
| Painting, compositing | vello_cpu/vello_gpu 0.3 (48k) in the toolkit; Blitz paints through anyrender into vello 0.11 | GPU sparse-strip rasterisation with a CPU twin, already tuned for Bande | the whole scene is re-encoded each frame (no retained layers), limited filters and blend layers | renderer: years; display list + compositor: 4–6 eng-mo (8–12k) | **own** the display list, stacking contexts, layers, async scroll and damage; **dependency** on vello through the toolkit |
| Networking | hyper 1.12 (22.8k), h2 (23k), rustls 0.23 (49.6k), quinn + h3 0.0.8 (51k); `blitz-net` is a 466-line reqwest wrapper | correct HTTP/1.1 and HTTP/2, memory-safe TLS, platform certificate verifier (Mail and Calendar already use it) | hyper-util's idle-pool reaper is a timer; h3 is experimental; Blitz has no cookies, CORS or Fetch semantics | Fetch layer: 6–9 eng-mo (10–15k); HTTP or TLS stacks: years | **own** Fetch (CORS, CSP, referrer, mixed content, redirects, cookies RFC 6265bis, cache, HSTS, priorities); **dependency** on hyper, h2, rustls; h3 in phase 4 |
| Images | image 0.25 (37.7k), png (13.6k, safe), zune-jpeg (9.7k, 85 `unsafe` SIMD), image-webp (safe), jxl-oxide, rav1d (54.8k, 502 `unsafe`, a dav1d port) for AVIF, usvg for SVG | fast safe decoders | decode to full RGBA8 at once, no progressive or downscaled decode, no memory budget | decoders: years | **dependency**, decoded in the sandboxed renderer; **own** the image cache (2–4k): decode at display size, drop offscreen, share by memfd |
| Media | symphonia 0.6 (pure-Rust audio, no Opus), rav1d (AV1); no pure-Rust VP9 or H.264 of quality | — | — | — | **GStreamer** (Debian) in a sandboxed media process in phase 4; own the media element and MSE glue |
| JavaScript | Boa 0.22 (154k + 30k parser, mark-sweep GC, bytecode interpreter, >90% of test262); Nova 1.0 (159k, data-oriented heap, exact GC with scoped handles, young); mozjs (SpiderMonkey: 2M lines of C++, a 175 MB crate, JIT) | Boa: pure Rust, conformant, embeddable; Nova: a heap design that suits a data-oriented DOM | Boa: no JIT (about 10–50× slower than a JIT on heavy JavaScript), stop-the-world GC; mozjs: C++, an hour-long build, Firefox's monthly security cadence | matching Boa: 2–4 eng-years; matching a JIT engine: out of reach | **dependency**, behind our own bindings layer (WebIDL generator, event loop, timers as deadlines): Boa first, measured gate at the end of phase 2 |
| Process model, sandbox | none to reuse in Rust; rustix, landlock 0.4, seccompiler 0.5 | Chromium: renderer processes per site, a broker for file access, a seccomp-bpf baseline; WebKit: one network process | Chromium's separate GPU process (here the app process owns the GPU and draws display lists, as WebRender does) | 3–4 eng-mo (5–8k) | **own** |
| Content blocking | adblock-rust 0.13 (Brave, 16k, MPL-2.0) | uBlock Origin and Adblock Plus syntax: network and cosmetic filters | — | 6 eng-mo | **dependency** in the network process |
| WebAssembly | wasmtime 50 (123k + Cranelift) | — | its JIT needs writable then executable memory in the sandbox; it needs the JS API bridge | — | **dependency**, phase 4 |

**Blitz** (DioxusLabs: blitz-dom, blitz-html, blitz-paint, about 40k lines) already runs Stylo,
taffy and parley at **the same versions as Bande** (parley 0.12, skrifa 0.44, accesskit 0.25,
html5ever 0.40, wgpu 30). Forking it gives a vertical slice in weeks. Its DOM and paint are then
reshaped into the owned core rather than tracked upstream. Blitz has no JavaScript by design.

**Owned core, about 90–135k lines.** DOM and bindings 15–25k; layout 25–40k; style glue 3–5k;
display list, compositor and scrolling 8–12k; Fetch 10–15k; process model, IPC and sandbox 5–8k;
image and resource cache 2–4k; event loop and scheduler 2–4k; accessibility tree 2–3k; the app
15–20k.
**Reused:** html5ever, cssparser, selectors, Stylo, taffy (flex and grid), parley, harfrust,
skrifa, fontique, ICU4X, vello (through the toolkit), hyper, h2, rustls, image codecs, rav1d,
usvg, GStreamer, Boa, adblock-rust, wasmtime, url, encoding_rs, psl.

## Why not embed an existing engine (short)

| Engine | Verdict |
|---|---|
| WPE WebKit 2.54 (`libwpewebkit-2.0-dev` in testing; its WPEPlatform API lets an app subclass `WPEView` and receive dmabuf buffers) | The fastest route to full compatibility, with Debian security updates and a bubblewrap sandbox. But it's C++, not ours, and its web content's accessibility goes over AT-SPI, which AccessKit can't graft. It's the fallback if the owner later picks compatibility over ownership. |
| Servo | Rust, but its script is bound to SpiderMonkey, it's not in Debian, it's a heavy build, and its sandbox and multiprocess mode aren't the default. We reuse its best parts (Stylo, html5ever, selectors) anyway. |
| CEF/Chromium | Chromium 156 is in Debian but CEF isn't: we'd ship an unpackaged blob outside apt's security updates. Heaviest memory use, and it's Google. |
| Gecko, Ladybird | Not embeddable. |

## Architecture

**Repos.** The engine is its own repo, `bande-linux/web`: the Stylo build, fuzzing and WPT
tooling don't belong in the apps workspace or its gate. The app `apps/browser` (`bande-browser`,
`org.bande.Browser`, made with `ci/new-app.sh browser Browser "Browser"`) pins it by git `rev`, as
it pins the desktop.

**Engine crates.** `web-dom` (arena, tree mutation, Stylo glue), `web-layout` (box tree,
fragments, taffy for flex and grid, parley for inline), `web-paint` (display list), `web-net`
(Fetch over hyper and rustls, cookies, cache, HSTS, adblock), `web-script` (Boa, WebIDL bindings,
event loop; phase 2), `web-ipc` (typed messages over Unix sockets with fd passing, memfd sharing,
sandbox launcher), `web-content` (the renderer's main loop).

**Processes.**
- The **app process** is the bande-app UI. It owns the GPU, and draws each tab's display list into
  the window's toolkit `Scene`.
- **One network process** holds cookies, the cache and blocking.
- **One renderer per site** (scheme plus registrable domain) parses, styles, lays out, runs script
  and emits display lists and the accessibility tree.

Renderers have no files (landlock, read-only fonts only), no sockets (seccomp), no GPU and no
Wayland. They get `PR_SET_NO_NEW_PRIVS` and their own cgroup under the app's scope. They parse
web fonts themselves and send glyph outlines, so the app process never parses untrusted fonts
or images. Each helper is a binary from the browser package, in `/usr/libexec/bande-browser/`.

**Rendering and input.**
- **Rendering.** Display lists are retained in the app process and scrolled there: scrolling never
  waits on a renderer. Damage follows the toolkit's item diff.
- **Input.** Key, pointer, scroll and text-input-v3 events go from the `View` to the renderer.
  Menu key equivalents win, and every other key goes to the page.
- **Accessibility.** The renderer builds an AccessKit subtree (accesskit 0.25, the same as the
  toolkit), and the app grafts it under the web area node, so Orca reads pages natively.

**Event-driven.**
- Every process sleeps in `epoll`. The network process runs a current-thread runtime with no
  interval timers.
- Page timers and `requestAnimationFrame` become deadlines and `wl_surface.frame` callbacks.
- Hidden tabs get no frames and throttled timers. After 10 minutes hidden, or under memory
  pressure (decision 0004), a tab's renderer is discarded: it keeps its URL, scroll position and
  a snapshot.
- With no window, the renderers and the network process exit (`Program::window_closed` drops the
  views).

## The app (Safari-like, Bande style)

**Window and toolbar.** The toolbar lives in the window header (`Window::set_header`):
- sidebar button, Back and Forward;
- the smart search field, centered: URL or search, completions from history and bookmarks,
  a padlock, the reader toggle;
- Downloads, New Tab, Tab Overview;
- the ⋮ menu button, which comes from bande-app.

The tab bar sits below the header once a window has two tabs.

**Menus** (Ctrl shortcuts):
- **File**: New Window Ctrl+N, New Private Window Shift+Ctrl+N, New Tab Ctrl+T, Open Location
  Ctrl+L, Open File Ctrl+O, Close Tab Ctrl+W through `Program::close_tab`, Save As Ctrl+S, Open in
  System Browser Shift+Ctrl+O.
- **Edit**: Find Ctrl+F, Find Next Ctrl+G, AutoFill.
- **View**: Sidebar, Reload Ctrl+R, Stop, Zoom Ctrl+= / Ctrl+- / Ctrl+0, Reader, Full Screen.
- **History**: Back Ctrl+[, Forward Ctrl+], Reopen Last Closed Tab Shift+Ctrl+T, Show All History
  Ctrl+Y, recent pages, Clear History.
- **Bookmarks**: Add Bookmark Ctrl+D, Show Bookmarks, Favourites.
- **Develop** (when turned on): page source, the DOM and style inspector, disable blockers.
- **Window**: Next and Previous Tab Ctrl+Tab and Shift+Ctrl+Tab, tabs 1–9 with Ctrl+1–9.

**Places.**
- History and bookmarks in SQLite (`rusqlite`) in the state folder.
- The start page is a native toolkit view of Favourites, not HTML.
- Downloads go to `~/Downloads`, with a list in a popover and Show in Files. Uploads and Save As
  use `bande_app::portal::choose_file`. Non-web schemes open through OpenURI.

**Private windows.** An in-memory network session per window, no history, and a darker
search field.

**bande-app behaviour.**
- One process.
- The app keeps running with no window. Reopen gives a new window; History restores closed ones.
- The window frame and the Settings window are remembered per window kind. On launch the app
  reopens either the last session or a new window (a setting).
- `.desktop` lists `x-scheme-handler/http(s)` and `text/html`, so Settings → Default Apps can pick
  it.

**Settings** (bande-app Settings window):
- **General**: what launch opens, the homepage, history kept for, the downloads folder.
- **Tabs**: open links in tabs, Ctrl+click opens in the background.
- **AutoFill and Passwords**: from `bande-keyring`, keyed by origin and user name. A login is
  filled only into the top frame of the same origin, only after the user picks it, and only over
  HTTPS.
- **Search**: the engine (DuckDuckGo by default), suggestions off by default.
- **Privacy**: block third-party cookies (on), HTTPS-only with an interstitial, filter lists
  (EasyList, EasyPrivacy, uBlock filters).
- **Websites**: per-site camera, microphone, location, notifications, pop-ups, autoplay, zoom,
  blocking.
- **Advanced**: show the full address, the Develop menu.

No telemetry, no Safe Browsing (it's Google's), no ads.

## Phases, compatibility and targets

**Measuring progress.** A fixed list of about 200 sites the owner uses, each scored *readable*,
*interactive* or *works*, plus WPT pass rates on chosen directories. The WPT runner lives in the
web repo and runs on the mini. Reftests compare two renders at run time, so no images are
stored. Shares below are estimates until that list exists; its scorer is milestone W1.

| Phase | Usable as | WPT focus | Share of everyday pages (estimate) |
|---|---|---|---|
| 1 Document browser: HTML, CSS, images, links, no JS | docs, wikis, articles, Debian and Rust docs, LWN, HN | html/syntax, css/CSS2, css-flexbox, css-grid, url, encoding | 25–35% readable |
| 2 Basic JS and DOM (Boa) | menus, progressive enhancement, simple fetch | dom, html/webappapis, xhr, fetch basics | 40–50% |
| 3 Forms, cookies, storage, auth, passwords | forums, forges, webmail with HTML views, admin panels | html/semantics/forms, cookies, webstorage, IndexedDB | 55–65% |
| 4 Broad compatibility: canvas, WebSocket, workers, media (GStreamer), WASM, HTTP/3, WebExtensions subset | SPAs, video | each area's directory | 80%+, open-ended |

**Performance targets for phase 1 on the M2 mini.**
- Window in under 150 ms.
- A 100 KB article styled and laid out in under 30 ms after the response arrives.
- Scrolling at the display's refresh rate with no relayout.
- The app process under 40 MB, each renderer under 30 MB for an article. Firefox uses about 150
  MB for one tab.
- Zero wake-ups in every process on a static page.
- With no window, 6 MB or less, as Mail.
- Benchmarks against Debian's firefox-esr and chromium, reported in releases, not stored in the
  repo.

## Milestones

Each milestone is a short branch with its done-criteria.

**Phase 1.**
- **W1 Slice** (web repo, the app's first merge). Fork Blitz; `bande-browser <url>` fetches over
  rustls and draws HTML and CSS in the window, in process. It has an address field, links,
  Back/Forward/Reload and scrolling, plus the top-sites list and scorer. **Done:** a Wikipedia
  article and docs.rs render readably on the mini, links navigate, there are zero idle wake-ups,
  and Ctrl+W and Ctrl+Q behave.
- **D1 (desktop)** `Scene` gains affine transforms, blend and opacity layers, radial and sweep
  gradients, glyph runs from given outlines, repeating images, and retained sub-scenes that are
  encoded once and translated per frame. **Done:** a long page scrolls without re-encoding.
- **W2 Owned DOM and display list.** Arena DOM, side-table layout and paint, display list into
  `Scene`, async scroll. **Done:** memory per element is measured and at most 30% of Blitz's;
  the slice's pages still render.
- **W3 Processes and sandbox.** Network process and per-site renderers, IPC, landlock and
  seccomp. **Done:** a renderer can't open a file or a socket (one regression test for this
  security boundary); a crashed renderer shows "page crashed".
- **D2 (desktop)** Terminal's tab bar moves into the toolkit as a widget.
- **A1 Tabs and windows.** Tabs, private windows, session restore, per-window-kind memory, URLs
  through `Program::open`, the `.desktop` handler entry.
- **D3 (desktop)** The toolkit grafts an external AccessKit subtree under a node.
- **W4 Owned layout.** Box and fragment trees, floats, positioning, tables, writing direction.
  **Done:** WPT css/CSS2 pass rate set as a baseline and rising.
- **A2 Places.** Smart search field, history, bookmarks, Favourites, Reader, Find, zoom, Open in
  System Browser.
- **A3 Resources.** Image cache (AVIF, WebP, JPEG XL, SVG), web fonts, HTTP cache, downloads,
  file chooser.
- **A4 Privacy.** Cookies policy, HTTPS-only, HSTS preload, adblock-rust lists updated by a daily
  deadline, Websites settings.
- **A5 Accessibility, test, release.** One e2e test (about 40 lines: open a local page on the
  headless compositor and find its heading and a link over AT-SPI), packaging, release
  `browser-v0.1.0`.

**Phases 2–4.** Each phase is a sequence of the same shape, planned when the previous one ships:
- phase 2: bindings generator, DOM core, events, timers, fetch/XHR, and a JS performance gate;
- phase 3: forms, storage, HTTP auth, passwords;
- phase 4: canvas over vello, WebSocket, workers, GStreamer media (D4 in the desktop repo: a
  video subsurface with dmabuf), wasmtime, h3, extensions.

## Packaging

`bande-browser` installs `bande-browser` and the helpers in `/usr/libexec/bande-browser/`.
- **Depends:** `$auto` plus the three font packages. GStreamer comes only in phase 4.
- **`build-depends`:** `python3`, for Stylo's Mako templates.
- **`smoke.sh`:** `bande-browser --version`, the helpers present, and `desktop-file-validate` with
  the MimeType line.
- **Also:** the icon in the desktop's `apps.py` and a row in the README table. Not in
  `default-apps` before phase 3.
- **Size and build:** the binary is about 30–40 MB (Stylo, Boa), and Stylo's build must fit the
  `gentle` 6 GB cap; measure it in W1.

## Security burden

- We own every bug in the owned core, plus timely updates of Stylo, html5ever, rustls, hyper,
  the codecs and Boa: `cargo audit` on every web-repo change, and an agent that watches RustSec
  and the upstreams.
- Fuzzing with cargo-fuzz on the HTML, CSS, URL, HTTP and image paths, run on the mini and not
  counted as tests.
- The sandbox is the backstop: Rust removes most memory bugs, but not logic bugs in Fetch, CORS
  or the same-origin checks, which get the second review.
- Boa has no JIT, so a whole class of JIT bugs is gone; choosing mozjs would bring Firefox's
  monthly CVE cadence.

## Risks

1. Scope creep toward full compatibility before phase 1 is excellent: hold the phase gates.
2. Phase 2 JavaScript speed with Boa; mitigated by the gate and a narrow bindings layer so the
   engine can change.
3. Stylo's build time and memory on the mini, and its global state against our process model.
4. Layout is where compatibility lives: tables, floats and inline are years of edge cases.
5. Users meet broken pages; mitigated by Open in System Browser and an honest phase label.
6. Forking Blitz means we stop getting its fixes; keep Stylo, taffy and parley upstream.
7. The apps test budget is 482 of about 500 lines.

## Owner decisions

1. **Ambition.** Subset-first (phases 1–3, about 2–3 engineer-years) with a system-browser
   fallback, or full compatibility (15–30 engineer-years). *Recommend subset-first, then decide on
   phase 4 with measured data.*
2. **Base.** Fork Blitz and reshape it, depend on Blitz, or clean-room. *Recommend fork.*
3. **JavaScript.** Boa, Nova, mozjs or QuickJS. *Recommend Boa, with a measured gate at the end of
   phase 2; keep Nova under watch.*
4. **Repo.** The engine in `bande-linux/web`. *Recommend yes.*
5. **Conformance.** WPT, including run-time reftests with no stored images, as the engine's
   metric, outside the apps test budget. *Recommend yes.*
6. **Tests.** One e2e test for the app, raising the apps budget to about 540 lines. *Recommend
   yes.*
7. **Media.** GStreamer in a sandboxed process in phase 4, rather than pure Rust (AV1 and part of
   the audio only). *Recommend GStreamer.*
8. **Extensions.** Built-in uBlock-syntax blocking now; a WebExtensions subset no earlier than
   phase 4. *Recommend yes.*
9. **Name and default.** "Browser" (`org.bande.Browser`), not the default browser before phase 3.
   *Recommend yes.*
10. **Agent maintenance** (note §7). Use the existing Bande agent workflow plus the RustSec and
    upstream watcher; no separate agent architecture. *Recommend yes.*
