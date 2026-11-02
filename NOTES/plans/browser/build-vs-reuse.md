# Build vs reuse

Sizes of existing crates were counted from their published sources in November 2026. "Owned" sizes
are our estimates for a first complete version, in lines of Rust.

## Owned: written from scratch, our architecture

| Part | What we learn from | Design we choose | Owned size | Effort |
|---|---|---|---|---|
| HTML tokenizer and tree builder | html5ever (11k): exact spec state machines, script pause for `document.write` | a byte-oriented tokenizer over UTF-8 input with SIMD scanning of text runs; the tree builder writes straight into our arena (no `TreeSink` with interior mutability); atoms for names; a speculative preload scanner; streaming for big pages | 10–15k | 6–9 eng-mo; html5lib-tests decide |
| CSS syntax and values | cssparser (6.5k), Stylo's value parsers | CSS Syntax 3 tokenizer without copying; property definitions in one Rust table that a build script turns into parsers, computed types and serialisation (no Python and Mako, as Stylo needs) | 40–60k (500+ properties in time) | 1.5–2.5 eng-years over the phases |
| Selectors, cascade, style | Stylo (137k): bloom filter, rule tree, style sharing cache, snapshot invalidation, parallel traversal | selectors compiled to bytecode and matched against the arena; invalidation sets (Chromium's) so a class change restyles only what can match; computed style split into shared inherited and reset groups; a rule tree; a style sharing cache; parallel traversal only above a size threshold; no global state (each document owns its stylist) | 25–40k | 2–3 eng-years |
| DOM | Blitz: its raw tree pointer, panicking accessors and layout fields per node are what we avoid | struct-of-arrays arena with `u32` node ids and generations; the element core in 64 B or less; attributes inline for up to 2; text in a shared rope; layout and paint data in side tables only for rendered nodes; mutation journal for incremental passes | 15–25k core; 60–100k with Web APIs | 1 eng-year core, years for the Web API surface |
| Layout | taffy (31k): flex and grid algorithms, per-node measure cache; Servo's Layout 2020 and Blink's LayoutNG | box tree, then immutable fragment trees cached per formatting context with constraint keys; block with margin collapsing, inline, floats, positioning, flex, grid, tables, multicolumn, writing modes, fragmentation for print | 60–100k | 6–10 eng-years to parity |
| Text layout | parley (21k), Bande's toolkit text stack | the CSS inline formatting context: line breaking around floats, inline boxes, bidi, `text-*` properties, hyphenation; shaped runs cached per font and string | 10–15k | 1–1.5 eng-years |
| Display list, compositing | vello's whole-scene encoding; WebRender's display lists | a display list per stacking context, retained layers, async scroll and animation in the app process, damage per layer; drawn by vello through the toolkit | 10–15k | 1 eng-year |
| Scheduler, event loop | — | the HTML event loop on epoll: tasks, microtasks, timers as deadlines, rendering opportunities on `wl_surface.frame`, background throttling | 3–5k | 4 eng-mo |
| Network policy | hyper's pool, which reaps idle connections on an interval timer: avoid | Fetch (CORS, CSP, referrer, mixed content, redirects, priorities), cookies (RFC 6265bis, partitioned), HTTP cache, HSTS, connection pools with deadlines and no intervals; a small epoll executor instead of tokio | 15–25k | 1.5 eng-years |
| Process model, IPC, sandbox | Chromium's site-per-process, broker and seccomp baseline; WebKit's single network process | see `engine.md` | 6–10k | 6 eng-mo |
| Demuxers, AVD bitstream parsers, Opus, MSE, EME host | — | see `media.md` | 30–45k | 2–3 eng-years |
| WebExtensions | Firefox's API | see `extensions.md` | 20–30k | 1–1.5 eng-years |
| JS bindings, then the JS engine | Boa, Nova, SpiderMonkey | see `js.md` | 30–50k bindings, 200k+ engine | see `js.md` |
| SVG, canvas, forms, media element, editing, accessibility tree | — | part of the DOM and layout, not separate libraries (usvg is for static images, not the SVG DOM) | 40–60k | in phases 3–5 |

## Plain dependencies, and why each stays

| Crate | Why it stays |
|---|---|
| rustls 0.23 with aws-lc-rs, rustls-platform-verifier | memory-safe TLS, audited, constant-time crypto in assembly; owning crypto is pure risk. Mail and Calendar already trust it |
| hyper 1 and h2; quinn and h3 for HTTP/3 | protocol framing is spec-locked and heavily fuzzed; we own everything above it (Fetch, pools, priorities) and drive it with our own executor through `hyper::rt` and quinn's runtime traits. h3 is still experimental (0.0.8), so it waits for phase 4 |
| zlib-rs or flate2, brotli-decompressor, zstd | content encodings; spec-locked, fuzzed, speed tuned |
| png, zune-jpeg, image-webp, gif, jxl-oxide, rav1d (AVIF and AV1) | decoders are a known attack surface where mature fuzzed code beats new code; they run only inside sandboxed processes; our image cache owns decoding at display size and memory budgets |
| symphonia (AAC-LC, MP3, Vorbis, FLAC); Debian's libavcodec only where the AVD is missing | see `media.md`: our own demuxers, Opus and AVD bitstream parsers; these decoders run only in the sandboxed media process |
| skrifa, read-fonts, harfrust, fontique | font parsing and shaping are spec-locked (OpenType) and shared with Bande's toolkit; web fonts are parsed only in sandboxed renderers |
| ICU4X (segmenter, normalizer, collator, plural rules, data), unicode-bidi, encoding_rs | Unicode and WHATWG Encoding data and algorithms; ICU4X also serves JS `Intl` |
| url | the WHATWG URL parser, small and conformant; replace only if WPT `url` shows gaps |
| wgpu, naga, vello (through `bande-toolkit`) | Bande's own render stack; WebGPU maps onto wgpu as Firefox does; vello also draws canvas 2D |
| accesskit | shared with the toolkit, so page accessibility joins the window's tree |
| rustix, landlock, seccompiler | thin system-call layers; the policy is ours |
| rusqlite | history, bookmarks, and the storage under IndexedDB, as Firefox does |
| wasmtime (Cranelift) | WebAssembly in phase 4 until our JS engine has its own WASM tier (WASM GC must share the JS heap) |

Not used: Stylo, taffy, Blitz, html5ever, cssparser, parley, usvg, tokio, reqwest, serde for IPC.
We study their source; we neither fork nor depend on them.
