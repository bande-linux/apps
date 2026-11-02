# Extensions: running the real uBlock Origin

There is no built-in blocker. Instead, the WebExtensions API surface that **uBlock Origin's
Firefox build** uses, so it runs at full strength. Firefox's MV2, and Firefox's MV3, both keep
*blocking* `webRequest`. That surface also covers most other privacy extensions.

## What uBO needs

| Area | API |
|---|---|
| Manifest V2 | persistent `background` page; `content_scripts` (`all_frames`, `match_about_blank`, `run_at: document_start`); `browser_action` popup; `options_ui` dashboard; `web_accessible_resources` (uBO's `redirect=` resources); `commands`; `unlimitedStorage` |
| Network | `webRequest` with `webRequestBlocking`: `onBeforeRequest` (cancel, `redirectUrl`), `onBeforeSendHeaders`, `onHeadersReceived` (add CSP, remove headers), `onCompleted`, `onErrorOccurred`; **`webRequest.filterResponseData`** (Firefox only, used for HTML filtering and `replace=`); **`dns.resolve`** (Firefox only, used for CNAME uncloaking) |
| Pages | `tabs` (query, update, `insertCSS` with `cssOrigin: "user"`, `executeScript`, `sendMessage`), `webNavigation`, `contextMenus`/`menus`, `privacy` (prefetch, WebRTC IP policy) |
| Plumbing | `runtime` (messages, ports, `getURL`), `storage.local` (large), `i18n`, `alarms`, `permissions`, `management` (self) |
| MV3, for uBO Lite and other extensions | `declarativeNetRequest` (static, dynamic and session rules), `scripting`, service-worker backgrounds |

## Design

- **Extension process.** Each extension runs in its own sandboxed process, like a renderer. Its
  background page and its popup and dashboard pages are ordinary pages of our engine, on an
  `extension://<id>` origin.
- **Isolated worlds.** Content scripts run in a separate JS realm over the same DOM. `web-script`
  is designed for several worlds per document from phase 2, so this needs no redesign.
- **The network hook.** The network process pauses a request at each `webRequest` stage and asks
  the extension process. A deadline bounds how long it waits; the request never polls.
  `filterResponseData` streams the body through the extension. The hook exists from W4 (phase 1),
  with no extension behind it.
- **Install.** `.xpi` files from addons.mozilla.org. Mozilla's add-on signature is verified
  against its root, and updates come through AMO's update service. Extension ids follow Gecko's,
  so uBO's settings import works.
- **Size.** About 1–1.5 engineer-years, the first item of phase 4. It needs phase 2's script and
  phase 3's storage, and uBO's dashboard needs a solid DOM.
- **Done when** uBO from AMO installs, its default lists block on the top-sites list, cosmetic and
  HTML filters apply, and CNAME uncloaking works.

**Until phase 4.** Privacy rests on the defaults:
- third-party cookies blocked;
- partitioned storage;
- HTTPS-only.

In phase 1 no scripts run at all.
