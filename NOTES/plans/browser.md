# Browser: a Bande-owned Rust web engine

The owner's note (`NOTES/browser.md`) and the owner's decisions (November 2026) set the terms:
- **Goal: full compatibility with today's web**, including YouTube, Google Docs and banking.
- **Our own engine, forking nothing**: modern, hyper-efficient, lean Rust with a clean code base.
- **Process:** private by default, no telemetry, no ads, independent of Google, maintained by agents.

The engine lives in `bande-linux/web` (its `CLAUDE.md` and README are on `main`). The app is `apps/browser` here (`bande-browser`, `org.bande.Browser`),
pinned to the engine by git `rev`, as it pins the desktop. Delete these notes as their milestones
finish.

| File | What |
|---|---|
| [browser/build-vs-reuse.md](browser/build-vs-reuse.md) | what we write, what stays a plain dependency and why, with sizes |
| [browser/engine.md](browser/engine.md) | crates, process model, data layouts, pipeline, efficiency targets |
| [browser/js.md](browser/js.md) | the JavaScript engine: sizes and the recommendation |
| [browser/quality.md](browser/quality.md) | module rules, `unsafe`, errors, conformance testing, fuzzing, security, CI |
| [browser/media.md](browser/media.md) | media stack, MSE, EME and Widevine |
| [browser/extensions.md](browser/extensions.md) | the WebExtensions surface that runs uBlock Origin |
| [browser/app.md](browser/app.md) | the Safari-like app on bande-app, packaging |
| [browser/roadmap.md](browser/roadmap.md) | phases, milestones and done-criteria, desktop work |

## DRM, Premium Streaming and Video Playback

The browser must aim for the highest possible DRM security level and streaming quality, with Netflix 4K playback on Linux as a primary compatibility target.

### Primary objectives

Design the architecture to support:

- Widevine L1 integration, subject to Google's licensing and provisioning.
- Hardware-backed DRM and trusted execution environments.
- Secure video decoding and protected media paths.
- Linux VA-API hardware acceleration.
- HEVC, AV1, VP9 and H.264 codec support.
- HDCP 2.2/2.3 output protection where supported by hardware and operating system.
- Encrypted Media Extensions (EME).
- Netflix 4K compatibility, subject to Netflix authorization.
- DRM capability detection, reporting and diagnostics.

### Adaptive fallback strategy

The browser must automatically determine the highest available combination of DRM security, codec, resolution and video acceleration. Establish a fallback hierarchy:

1. Widevine L1 with hardware-backed secure decoding and 4K playback.
2. Widevine L1 with lower-resolution playback.
3. Widevine L3 with hardware-accelerated decoding where permitted.
4. Widevine L3 with software decoding where permitted.
5. Lower resolutions and alternative supported codecs.
6. Unprotected playback when DRM is not required.

The browser must never falsely report DRM capabilities or circumvent content-provider security restrictions.

### Architectural requirements

- Keep DRM integration modular and independent of the rendering engine.
- Support multiple DRM systems in the future.
- Separate DRM authorization from codec and hardware capabilities.
- Detect hardware, driver, OS and display limitations.
- Expose detailed diagnostics explaining why a higher quality level is unavailable.
- Make it possible to upgrade DRM components without redesigning the browser.

### Acceptance criteria

The browser should achieve the highest officially authorized streaming quality available on each supported platform.

Netflix 4K on Linux is the aspirational target, not a guaranteed capability. Where licensing, hardware, operating system or provider restrictions prevent it, the browser must gracefully fall back to the highest permitted quality.

Do not sacrifice security or violate DRM requirements to achieve higher resolution.

## The size of it, honestly

Full parity with Chromium and Firefox is **22–32 engineer-years** of focused work, then a
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
| 4 Broad web | WebExtensions (uBlock Origin) first, then canvas, SVG, workers, WebSocket, our media stack and MSE, IndexedDB, editing, WebGL and WebGPU, WASM, HTTP/3, EME and Widevine | 7–11 | 85–90% |
| 5 Parity | our own JIT, Speedometer and JetStream parity, WebRTC, WebAuthn, the long tail of quirks | 7–10 | 95%+: YouTube, Docs, banking |

**What "our own" means here.** We own everything that decides speed, memory and architecture,
and the parsers too: the HTML parser, the CSS parser and style system, the DOM, layout, text
layout, the display list and compositing, the scheduler, the process model, network policy, the
JS bindings and, in time, the JS engine, plus demuxers and codec bitstream parsers. Dependencies are kept only for commodity,
security-sensitive parts where a rewrite adds risk and no speed: TLS and crypto, compression,
the decoders we don't write (rav1d, symphonia, libavcodec without hardware), font parsing and shaping, Unicode data, Bande's render stack (wgpu and
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

## Owner decisions (November 2026)

Decided:
1. **JavaScript:** Boa first, then our own engine (`js.md`).
2. **Tests:** WPT including reftests, html5lib-tests and test262 are the web repo's tests.
3. **DRM is wanted:** EME with the Widevine CDM, licence permitting (`media.md`).
4. **Media:** whichever is best, even if it is more work. The recommendation (`media.md`): our
   own demuxers and bitstream parsers driving the M2's video decoder, rav1d for AV1, our own
   Opus, symphonia for the other audio codecs. Open item 2 is its only remaining choice.
5. **User-Agent:** a "Bande" token plus a compatibility token, with per-site overrides.
6. **Extensions:** no built-in blocker. The real uBlock Origin runs through WebExtensions in
   phase 4 (`extensions.md`).
7. **Default browser:** Browser becomes the default and ships with bande-desktop in time, no
   rush.
8. **Throughput:** 3–4 parallel agent streams (parser and style, layout, net and processes, app).
9. **Widevine:** build full support (W-CDM) now; the owner contacts Google about a licence
   directly, and testing Netflix is one of the last steps.
10. **Software fallback without the AVD:** Debian's libavcodec in the sandboxed media process.
11. **VA-API:** tested later on normal amd64 PCs, together with bande-desktop on amd64.
