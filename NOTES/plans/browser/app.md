# The app: Browser

`ci/new-app.sh browser Browser "Browser"` makes `apps/browser` (`bande-browser`,
`org.bande.Browser`), a bande-app app.

## bande-app behaviour

- **One process, single instance.** URLs from a second launch arrive through `Program::open`.
- **Without a window.** The app keeps running; Reopen gives a new window, and History restores
  closed ones. With no window, the renderers and the network process exit.
- **Remembered per window kind.** The browser window and the Settings window keep their frames.
  Launch opens the last session or a new window (a setting).
- **Default Apps.** The `.desktop` lists `x-scheme-handler/http`, `https` and `text/html`, so
  Settings → Default Apps can pick it.
- **Close Tab.** Ctrl+W closes the tab through `Program::close_tab`.
- **Keys.** Menu key equivalents win; every other key goes to the page.

## Window

The toolbar lives in the header (`Window::set_header`):
- sidebar button, Back and Forward;
- the smart search field, centered: URL or search, completions from history and bookmarks,
  a padlock, Reader;
- Downloads, New Tab, Tab Overview;
- the ⋮ menu button, from bande-app.

The tab bar (desktop D2) appears below the header with two or more tabs. The start page is a
native view of Favourites. Private windows have an in-memory network session per window, no
history, and a darker search field.

## Menus

- **File**: New Window Ctrl+N, New Private Window Shift+Ctrl+N, New Tab Ctrl+T, Open Location
  Ctrl+L, Open File… Ctrl+O, Close Tab Ctrl+W, Save As… Ctrl+S, Print… Ctrl+P (phase 4), Open in
  System Browser Shift+Ctrl+O.
- **Edit**: Find Ctrl+F, Find Next Ctrl+G, AutoFill.
- **View**: Sidebar, Reload Ctrl+R, Stop, Zoom Ctrl+= / Ctrl+- / Ctrl+0, Reader, Full Screen.
- **History**: Back Ctrl+[, Forward Ctrl+], Reopen Last Closed Tab Shift+Ctrl+T, Show All History
  Ctrl+Y, recent pages, Clear History…
- **Bookmarks**: Add Bookmark Ctrl+D, Show Bookmarks, Favourites.
- **Develop** (when turned on): page source, the DOM, style and network inspector.
- **Window**: Next and Previous Tab Ctrl+Tab and Shift+Ctrl+Tab, tabs 1–9 with Ctrl+1–9.

## Places, downloads, portals

- History and bookmarks live in SQLite in the state folder.
- Downloads go to `~/Downloads`, listed in a popover with Show in Files.
- Uploads and Save As use `bande_app::portal::choose_file`.
- Non-web schemes open through OpenURI.
- Passwords live in `bande-keyring`, keyed by origin and user name. A login is filled only into
  the top frame of the same origin, only after the user picks it, and only over HTTPS.

## Settings (the bande-app Settings window)

- **General**: what launch opens, the homepage, history kept for, the downloads folder.
- **Tabs**: open links in tabs, Ctrl+click opens in the background.
- **AutoFill and Passwords.**
- **Search**: the engine (DuckDuckGo by default), suggestions off by default.
- **Privacy**: third-party cookies blocked, partitioned storage, HTTPS-only with an interstitial.
- **Websites**: per-site camera, microphone, location, notifications, pop-ups, autoplay, zoom,
  memory budget.
- **Extensions** (phase 4): installed extensions, their permissions and site access.
- **Advanced**: show the full address, the Develop menu.

No telemetry, no Safe Browsing (it's Google's), no ads.

## Packaging

`bande-browser` installs the app and the helpers (`/usr/libexec/bande-browser/`: content, netd,
then media and gpu). The helpers are built from the web repo's crates as `[[bin]]` targets of the
app.
- **Depends:** `$auto` plus the three font packages. Phase 4 adds the media and CDM helpers;
  libavcodec is a Recommends, used only without the AVD.
- **`smoke.sh`:** `bande-browser --version`, the helpers present, and `desktop-file-validate` with
  the MimeType line.
- **Also:** the theme icon in the desktop's `apps.py` and a row in the README table. Join
  `default-apps` per decision 7.
