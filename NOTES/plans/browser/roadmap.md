# Roadmap

Each milestone is a short branch that builds and ships. W is the web repo, A is this app, D is
the desktop repo.

**Progress measure.** A fixed list of about 200 sites the owner uses, each scored *readable*,
*interactive* or *works*, plus pass rates per WPT directory. Shares are estimates until the
scorer exists (W1).

## Phase 1: document browser (3–4 engineer-years)

- **W1 Text slice.**
  - Builds: the web repo's skeleton and lints; `web-html` passing ≥ 95% of html5lib
    tree-construction; the arena DOM; built-in default styles for HTML elements; block and
    inline layout over harfrust; a display list into `Scene`; fetch over hyper and rustls on our
    executor. In process for now.
  - In the app: an address field, links, Back/Forward/Reload, scrolling. Also the scorer.
  - **Done:** a Wikipedia article is readable on the mini (headings, paragraphs, lists, links
    that navigate), with zero idle wake-ups. Ctrl+W and Ctrl+Q behave.
- **W2 CSS.**
  - Builds: `web-css` (Syntax 3, the property table with about 120 properties), selectors with
    bloom filter and invalidation sets, the cascade with layers and specificity, computed-style
    sharing, incremental restyle.
  - **Done:** author stylesheets apply; baselines set for WPT `css/selectors`, `css/css-cascade`
    and `css/css-syntax`; hovering a link restyles only that link.
- **W3 Layout and images.**
  - Builds: margin collapsing, floats, positioning, replaced elements; images (PNG, JPEG, WebP,
    GIF, AVIF, JPEG XL) through the budgeted cache.
  - **Done:** WPT `css/CSS2` baseline; most of the list's articles readable.
- **D1** `Scene` gains affine transforms, blend and opacity layers, radial and sweep gradients,
  glyph runs from given outlines, repeating images, and retained sub-scenes translated without
  re-encoding.
- **W4 Processes.**
  - Builds: network process with the paused-request hook for extensions, per-site renderers, IPC, landlock and seccomp, the memory cgroup per
    tab.
  - **Done:** the sandbox regression test passes; a killed renderer shows "page crashed".
- **D2** Terminal's tab bar moves into the toolkit as a widget.
- **A1** Tabs, windows, private windows, session restore, Default Apps entry.
- **D3** The toolkit grafts an external AccessKit subtree under a node.
- **W5** Tables, lists and counters, web fonts, `@media`, the HTTP cache, cookies and HSTS.
- **A2** Smart search field, history, bookmarks, Reader, Find, zoom, downloads, Open in System
  Browser.
- **A3** Accessibility of pages, the e2e test, packaging, the `browser-v0.1.0` release.

## Phase 2: script (2–3 engineer-years)

- WebIDL bindings generator and engine interface on Boa.
- The DOM core: Node, Element, Document, events, `querySelector`, `classList`, `innerHTML`.
- The event loop, timers, `requestAnimationFrame`.
- Fetch and XHR, the History API, CSSOM, `MutationObserver`.
- **Done at phase end:** WPT `dom` and `html/webappapis` baselines rising; 40–50% of the list.

## Phase 3: applications (3–4 engineer-years)

- Forms, validation and autofill; cookies complete; localStorage and sessionStorage; HTTP auth.
- The password filler; permissions with prompts.
- Flex and grid complete; transforms, transitions and animations on the compositor.
- `web-js` starts.
- **Done at phase end:** 55–70% of the list; Browser becomes usable as a daily second browser.

## Phase 4: broad web (7–11 engineer-years)

- **Extensions first:** the WebExtensions surface uBlock Origin needs (`extensions.md`).

- **Graphics and documents:** canvas 2D over vello; SVG DOM, layout and paint; `contenteditable`
  and editing; print.
- **Workers and storage:** workers and service workers; IndexedDB on SQLite.
- **Networking:** WebSocket, HTTP/3.
- **Media** (`media.md`):
  - **K1** (kernel): the AVD driver in the build with `avd-fw`, fuzzed through V4L2.
  - **D5** (desktop): NV12 and P010 multi-planar dmabufs in the compositor, a video subsurface in
    the toolkit, protected dmabufs passed through.
  - **W-Media**: the media element, our demuxers and bitstream parsers, MSE, Opus, symphonia, and
    the decode backends behind one interface: V4L2 stateless first, then VA-API (verified on an
    Intel or AMD machine), then software.
- **DRM** (`media.md`):
  - **W-EME**: the `KeySystem` interface, Clear Key, the capability detector, the 6-tier
    fallback, honest robustness reporting, `bande://media` diagnostics.
  - **W-CDM**: the sandboxed Widevine host and the component updater, turned on once licensed.
  - **D6** (desktop): a content-protection protocol; protected surfaces only on outputs that
    meet them, black in screenshots and screencasts.
  - **K2** (kernel): the DCP `hdcp` endpoint traced, and "Content Protection" and "HDCP Content
    Type" on the DCP connector if the firmware allows.
  - **Done:** Netflix plays at the highest tier the M2 reaches (tier 3 or 4, at Netflix's L3 cap),
    and `bande://media` names why tiers 1–2 are unavailable.
- **GPU:** WebGL and WebGPU through the GPU process.
- **The rest:** WASM through wasmtime; the baseline JIT.
- **Done at phase end:** 85–90% of the list.

## Phase 5: parity (7–10 engineer-years)

- The optimising JIT and `web-js` WASM tiers.
- WebRTC; WebAuthn and passkeys (hidraw through a portal); full accessibility.
- Performance parity on Speedometer and JetStream; the long tail of compatibility quirks.
- **Done:** YouTube, Google Docs and the owner's banks work; the `engine.md` targets are met.

## Risks

1. Scope: compatibility is a moving target; the web adds features faster than a small team ships.
   The phase gates and the system-browser fallback keep each step useful.
2. JavaScript speed before our JIT: heavy sites are slow in phases 2–4.
3. Layout is where compatibility lives: tables, floats, inline and quirks mode are years of edge
   cases.
4. Writing our own style system and HTML parser instead of Stylo and html5ever costs about 3
   engineer-years more than reusing them; the owner accepted this for a clean, efficient core.
5. Security: owning the parsers means owning their bugs. The sandbox, fuzzing and Rust are the
   defence.
6. Sites that block unknown engines (decision 5).
7. Widevine depends on Google's licence; without it, DRM sites stay in the system browser.
8. The AVD driver is held, not yet upstream; until K1, video decodes in software.
9. Widevine L1, and so Netflix 4K, is unreachable on the M2 under Linux: there's no certified
   TEE. The aspiration depends on Google, Apple or other hardware, not on our code.
10. HDCP on the M2 depends on reverse-engineering the DCP firmware's `hdcp` endpoint (K2).
