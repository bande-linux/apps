# JavaScript

JavaScript is the single biggest item.

| Engine | Size | What it is | Fit |
|---|---|---|---|
| SpiderMonkey (mozjs) | `js/src` 1.2M lines of C++; a 175 MB crate | interpreter, baseline and optimising JITs, WASM | C++ and Firefox's monthly CVE cadence: excluded by "fully Rust" |
| V8 | over 1M lines of C++ | the fastest JITs | Google: excluded |
| Boa 0.22 | 154k + 30k parser, 290 `unsafe` | bytecode interpreter, mark-sweep stop-the-world GC, >90% of test262 | pure Rust, a plain dependency, about 10–50× slower than a JIT on heavy scripts |
| Nova 1.0 | 159k, 1122 `unsafe` | data-oriented heap (one vector per object type, index handles), exact GC with scoped handles, interpreter | elegant, young; a design to learn from |
| Our own | 200–300k to parity | see below | the end state the owner wants |

**What our own costs.**
1. A conformant interpreter (parser, bytecode, objects, built-ins, `Intl` over ICU4X, generational
   GC) at test262 parity with Boa: 150–200k lines, **4–6 engineer-years**.
2. A baseline JIT plus inline caches on arm64: **2–3 engineer-years**.
3. An optimising JIT that reaches JetStream and Speedometer parity: **6–10+ engineer-years**.
   V8's and SpiderMonkey's JIT teams each have dozens of engineers.

Google Docs, Figma and heavy sites need step 3.

**Recommendation.**
- Write `web-script` (WebIDL bindings generator, Web APIs, event loop) engine-neutral from day
  one, behind a narrow interface: realms, objects, functions, promises, GC roots, wrappers.
- Phase 2 ships on Boa as a plain dependency.
- `web-js` starts in phase 3 with a data-oriented heap that the DOM wrappers share (one heap, so
  no cross-heap cycles and no wrapper overhead). It replaces Boa when it passes Boa on test262
  and on the top-sites list.
- The JITs follow in phases 4–5.

Starting with our own engine on day one delays phase 2, the first scripted pages, by about 2–3
years and buys nothing, because the bindings don't change.
