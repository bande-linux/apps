# Browser: a Bande-owned Rust web engine

The owner's note (`NOTES/browser.md`) and the owner's decisions (November 2026) set the terms:
- **Goal: full compatibility with today's web**, including YouTube, Google Docs and banking.
- **Our own engine, forking nothing**: modern, hyper-efficient, lean Rust with a clean code base.
- **Process:** private by default, no telemetry, no ads, independent of Google, maintained by agents.

The engine lives in a new repo, `bande-linux/web` (approved; the coordinator creates it when
implementation starts). The app is `apps/browser` here (`bande-browser`, `org.bande.Browser`),
pinned to the engine by git `rev`, as it pins the desktop. Delete these notes as their milestones
finish.

| File | What |
|---|---|
| [browser/build-vs-reuse.md](browser/build-vs-reuse.md) | what we write, what stays a plain dependency and why, with sizes |
| [browser/engine.md](browser/engine.md) | crates, process model, data layouts, pipeline, efficiency targets |
| [browser/js.md](browser/js.md) | the JavaScript engine: sizes and the recommendation |
| [browser/quality.md](browser/quality.md) | module rules, `unsafe`, errors, conformance testing, fuzzing, security, CI |
| [browser/app.md](browser/app.md) | the Safari-like app on bande-app, packaging |
| [browser/roadmap.md](browser/roadmap.md) | phases, milestones and done-criteria, desktop work |

## The size of it, honestly

Full parity with Chromium and Firefox is **21–30 engineer-years** of focused work, then a
permanent load of security fixes and new web features. For comparison:
- SpiderMonkey's `js/src` alone is 1.2 million lines of C++.
- Stylo is 137k lines of Rust.
- Servo has taken a decade with dozens of engineers.
- Ladybird has had about eight full-time engineers since 2022.

The phases below are the delivery path: each ships and is usable, with **Open in System Browser**
as the fallback for pages it can't handle yet.

| Phase | Ships as | Engineer-years | Everyday pages (estimate) |
|---|---|---|---|
| 1 Document browser | HTML, CSS, images, links, no JS: docs, wikis, articles | 3–4 | 25–35% readable |
| 2 Script | DOM, events and fetch on Boa; menus, progressive enhancement | 2–3 | 40–50% |
| 3 Applications | forms, cookies, storage, auth, passwords, flex and grid complete; our own JS engine starts | 3–4 | 55–70% |
| 4 Broad web | canvas, SVG, workers, WebSocket, media and MSE, IndexedDB, editing, WebGL and WebGPU, WASM, HTTP/3, WebExtensions | 6–9 | 85–90% |
| 5 Parity | our own JIT, Speedometer and JetStream parity, WebRTC, WebAuthn, the long tail of quirks | 7–10 | 95%+: YouTube, Docs, banking |

**What "our own" means here.** We own everything that decides speed, memory and architecture,
and the parsers too: the HTML parser, the CSS parser and style system, the DOM, layout, text
layout, the display list and compositing, the scheduler, the process model, network policy, the
JS bindings and, in time, the JS engine. Dependencies are kept only for commodity,
security-sensitive parts where a rewrite adds risk and no speed: TLS and crypto, compression,
image and video codecs, font parsing and shaping, Unicode data, Bande's render stack (wgpu and
vello), and HTTP framing. Each is listed with its reason in `build-vs-reuse.md`. Everything is
used as a plain dependency; nothing is forked or vendored.

## First milestones (details in `roadmap.md`)

1. **W1 Text slice.** Our own HTML parser, arena DOM, built-in default styles, block and inline
   layout, a display list into the window's scene, and fetch over hyper and rustls. A Wikipedia
   article is readable in `bande-browser`, links work, and there are zero idle wake-ups.
2. **W2 CSS.** Our own CSS syntax parser, selectors, cascade, computed-style sharing and
   incremental restyle. Author stylesheets apply.
3. **W3 Layout and images.** Margin collapsing, floats, positioning, replaced elements, the image
   cache with its memory budget.

## Open owner decisions

1. **JavaScript** (`js.md`). Boa as a plain dependency for phases 2–3 while our own engine is
   built, or our own engine from day one. *Recommend Boa first*: it saves about 2–3 years before
   phase 2 ships, and the bindings are engine-neutral.
2. **Conformance as the test suite.** The web repo's tests are WPT, html5lib-tests and test262,
   with an expectations file. Our own tests stay few, as in Bande's rules. *Recommend yes.*
3. **DRM.** Widevine isn't licensed to independent Linux arm64 browsers, so Netflix-type sites
   won't play. YouTube and banking need no DRM. *Recommend no DRM.*
4. **Media decoding.** Debian's GStreamer or FFmpeg in a sandboxed media process for H.264,
   VP9, AAC and Opus; pure-Rust decoders where they're good (rav1d for AV1). *Recommend yes.*
5. **User-Agent.** An honest "Bande" token plus a compatibility token, with per-site overrides
   for sites that sniff. *Recommend yes.*
6. **Extensions.** A full WebExtensions API (MV2 and MV3 with blocking webRequest, so the real
   uBlock Origin runs) in phase 4; built-in uBlock-syntax blocking from phase 1. *Recommend yes.*
7. **Default browser.** Become the default when the top-sites list scores 90% "works"; until then
   firefox-esr stays the default. *Recommend yes.*
8. **Throughput.** How many parallel agent streams the coordinator may run on the web repo. The
   engineer-year figures are calendar years at one stream. *Recommend 3–4 streams:* parser and
   style, layout, net and processes, app.
