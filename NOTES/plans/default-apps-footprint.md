# Default apps footprint

What `apt install bande-desktop` 0.1.1 pulls into a clean Debian testing, what to cut, and which
cut apps to build as Bande apps. Measured 2026-11-02 in a rootless minbase chroot (mmdebstrap,
unshare) with `bande-apt-source` 1.3, by `apt-get install -s`; sizes are Debian's
`Installed-Size`. A cut is measured by simulating the install without it.

## Totals

| Install | Packages | Installed |
|---|---:|---:|
| `apt install bande-desktop` (recommends, what the owner got) | 917 | 2.52 GB |
| `--no-install-recommends` | 258 | 0.90 GB |
| Lean base: every cut and replacement below, no Firefox, no cups | 477 | 1.31 GB |

By stack, with recommends: GTK 220 MB (21 pkgs), GNOME libs 125 MB (65), GStreamer 47 MB
(16), codecs/ImageMagick/ML 239 MB (118), Qt 64 MB (19), printing 79 MB (31), Perl and Python
88 MB (61), X11 14 MB (51), Firefox 273 MB, fonts 204 MB, Bande 233 MB, mesa/graphics 368 MB.
There is no X server and no XWayland. The X11 *tools* that do arrive (`x11-utils`,
`x11-xserver-utils`, `luit`, `libxaw7`, with `cpp` 44 MB as their dependency) all come from
`xdg-utils`' recommends (below). The X11 *client libraries* left (`libx11-6`, `libxcb*`, 7 MB) come
with mesa, cairo and `gstreamer1.0-plugins-good`, and Debian builds them in; they cannot be
avoided.

## What each item costs

These are the packages one item adds on top of the lean base (the base with every cut applied).
An item's dependencies that other items share are counted for each of them.

| Item | Relation | Adds | Mostly |
|---|---|---:|---|
| gnome-calculator | bande-desktop R | 222 pkgs, 601 MB | its Recommends `yelp` → WebKitGTK 6 (94 MB), JSC (30 MB), gst-plugins-bad, gst-libav/FFmpeg, ImageMagick, flite, TensorFlow Lite, man-db, groff; plus `gvfs`. GTK4, libadwaita |
| firefox-esr | bande-desktop R | 40, 365 MB | Firefox 273 MB, GTK3, libavcodec (it recommends) |
| cups | bande-desktop R | 97, 239 MB | ghostscript, poppler, SANE, gphoto, colord, avahi, lynx, Perl |
| papers | bande-desktop R | 59, 153 MB | GTK4, libadwaita, libnautilus-extension, poppler, djvu, spelling |
| qt6-xdgdesktopportal-platformtheme | bande-desktop R | 60, 155 MB | the 66 kB plugin brings Qt 6 Gui, QML, Quick, Wayland compositor, `qt6-gtk-platformtheme` → GTK3, 15 X11 libs, ICU |
| gnome-clocks | bande-desktop S | 51, 132 MB | GTK4, libadwaita, libgweather, geoclue, GStreamer (not installed by default) |
| gnome-text-editor | bande-desktop R | 48, 115 MB | GTK4, libadwaita, GtkSourceView, ICU, enchant/hunspell/aspell |
| gnome-keyring + libpam-gnome-keyring | bande-desktop R | 50, 111 MB | `gcr` → both GTK3 and GTK4 (through `gcr4`, `pinentry-gnome3`) |
| loupe | bande-desktop R | 37, 93 MB | GTK4, libadwaita, libgweather (22 MB of locations, for the photo location) |
| xdg-utils' recommends | bande-config R → xdg-utils R | 63, 68 MB | x11-utils, x11-xserver-utils → cpp-16 (40 MB); libfile-mimeinfo-perl → libfile-desktopentry-perl R `x-terminal-emulator` → **alacritty**; Perl LWP stack |
| showtime | bande-desktop R | 46, 65 MB | GTK4 via PyGObject, Python, gst-gtk4 (GStreamer core is already in the base, below) |
| needrestart | bande-desktop R | 14, 57 MB | Perl |
| xdg-desktop-portal-gtk | xdg-desktop-portal-bande R, flatpak R | GTK3 | GTK3 is shared with the keyring and the Qt theme; dropping it alone saves little |
| bande-screenshot's GStreamer recommends | bande-screenshot R | 9, 4 MB | the H.264 encoder for screen recording; cheap, keep |
| all six `[debian]` recommended apps together | | 258, 943 MB | the first GTK4 and libadwaita app costs about 90 MB; the calculator's `yelp` is most of the rest |

## Top heavy packages

| Package | MB | Comes from | Verdict |
|---|---:|---|---|
| firefox-esr | 273 | bande-desktop R | keep until Bande Browser can stand in |
| mesa-vulkan-drivers, libllvm21, mesa-libgallium, libz3 | 344 | bande-compositor D (Vulkan, GBM) | required |
| libwebkitgtk-6.0-4 + libjavascriptcoregtk | 124 | gnome-calculator R → yelp | cut |
| fonts-noto-cjk, fonts-noto-core | 131 | bande-desktop D | keep (every script) |
| cpp-16 | 40 | bande-config R → xdg-utils R → x11-xserver-utils | cut |
| libicu78 | 38 | text editor, Qt, others | goes with them |
| libgtk-3-common + libgtk-3-0 | 41 | firefox, gcr, Qt theme, portal-gtk | kept by Firefox only |
| libflite1, libonnxruntime, libdnnl, libtensorflow-lite | 84 | flite/TFLite: gst-plugins-bad (yelp). onnxruntime/dnnl (39 MB): **pipewire's libspa-0.2-modules D** | onnx stays (Debian's PipeWire) |
| libperl5.42 + perl-modules | 48 | needrestart, cups, xdg-utils | cut with them |
| gweather-locations | 22 | loupe | cut |
| libgs10, poppler-data, fonts-urw-base35, libsane1 | 62 | cups | see cups |

## Classification

**Required** (no desktop without it): bande-compositor, -config, -power, -sound, -notify,
-clipboard, -shell, -lock, -greeter (or a display manager), xdg-desktop-portal-bande,
xdg-desktop-portal, dbus-user-session, logind, libpam-systemd, pipewire-audio, mesa/Vulkan,
greetd, polkitd (bande-polkit), locales (bande-settings).

**Useful, keep**: network-manager (+ modemmanager, ppp, dnsmasq-base come with it), bluez,
upower, power-profiles-daemon (brings Python 3, 25 MB), udisks2, accountsservice, packagekit,
appstream, flatpak (bande-software R), at-spi2-core, xdg-user-dirs, systemd-oomd,
systemd-zram-generator, every font (Noto, Inter, Manrope, JetBrains Mono, emoji),
adwaita-icon-theme (fallback icons, 13 MB), bande-apt-source, apt-config-icons-hidpi, every
bande-* app and service, bande-screenshot's GStreamer recommends, firefox-esr for now.

**Cut**:

| Item | Saves | How |
|---|---:|---|
| gnome-calculator | 288 MB from today's install (601 MB on the lean base) | drop from `[debian]`; a Bande Calculator replaces it |
| papers, loupe, gnome-text-editor, showtime | together with the calculator and Firefox, 943 MB on the lean base | drop from `[debian]` as each Bande app lands; showtime only when there is a media stack |
| xdg-utils' recommends | 68 MB, all the X11 tools, alacritty | Bande's `xdg-open` callers (config, screenshot, captive portal, terminal) open through Bande's own handler (bande-xdg's mimeapps), then drop `Recommends: xdg-utils` from bande-config. Until then `bande-terminal` should `Provides: x-terminal-emulator`, which stops apt picking alacritty |
| qt6-xdgdesktopportal-platformtheme | 107 MB with no Qt app installed | move to Suggests; a Qt app brings Qt, and Software (or a small `Enhances` package) adds the plugin with the first Qt app |
| xdg-desktop-portal-gtk | GTK3 once the keyring is gone | xdg-desktop-portal-bande `Provides: xdg-desktop-portal-backend` (flatpak recommends `-gtk \| -backend`; without it apt pulls `-lomiri`, which brings Qt) and drops `Recommends: -gtk`; keep `-gtk` as the fallback for interfaces Bande lacks (Print, AppChooser, Inhibit…) only after a check of which ones Firefox and Flatpak apps call |
| cups | 239 MB | decision for the owner: keep (printing works out of the box, as on macOS) or Suggests plus install on demand from Settings › Printers through PackageKit |

**Replace with Bande code** (later, M each): gnome-keyring (111 MB, GTK3+GTK4 through gcr) with
a Bande Secret Service (with PAM unlock from the greeter); needrestart (57 MB of Perl) with the
restart logic in Software Update (PackageKit plus `/proc/*/maps` deleted-file scan).

The lean base (every cut, no Firefox, no cups): 477 packages, 1.31 GB, no GTK, Qt, libadwaita, GNOME apps, X11 tools or
WebKit. Left from those stacks: GLib, gdk-pixbuf/glycin (through librsvg for the icon theme),
GStreamer base/good (a hard dependency of xdg-desktop-portal), dconf, Python (power-profiles-daemon).

## Bande apps to build

The framework (`bande_app::run`, `ci/new-app.sh`, the template) gives windows, menus, keys,
Settings and single instance; the toolkit has a multi-line editor (`widgets/editor`, undo
history and selection) and `bande-preview` decodes PNG, JPEG, GIF and WebP (plus SVG through
resvg).

| Order | App | Effort | Value | Notes |
|---:|---|:---:|---|---|
| 1 | Calculator | done | 288 MB today, the biggest single cut | `apps/calculator`; left: unit conversions, the decimal-places setting, non-integer x! (gamma) |
| 2 | Preview: images | done | 93 MB (loupe), and the default image handler | built (`apps/preview`): one decoder path, the `image` crate with allocation limits as in `bande-preview` and `bande-thumbnails` (SVG through resvg, as the toolkit), not `web-image`, which decodes first frames only; a 2× pyramid tiled at 512 px keeps big photos fast and GPU memory at the window's size. Left: HEIC/AVIF, saving a rotation, touchpad pinch and a full-screen slideshow (the toolkit has neither pinch events nor a full-screen call), images over 100 MP |
| 3 | TextEdit | rich text done | 115 MB, the default text handler | `apps/textedit`: encodings, line endings, find and replace, autosave and restore, RTF, Markdown and HTML; office documents next. Gaps in `textedit.md` |
| 4 | Clock | done | small (gnome-clocks was only a Suggests) | built as an app (`apps/clock`) rather than in the shell; the shell's Clock menu lists its running timers and next alarm through `org.bande.Clock`. Left: waking from sleep on the M2 (no RTC wake alarm), longer ringtones |
| 5 | Preview: PDF | L | 153 MB (papers), the default PDF handler | needs a renderer: a pure-Rust one (hayro) or Debian's poppler-cpp/MuPDF without GTK; then text selection, search, thumbnails, annotations, forms. Same app as images, as on macOS |
| 6 | Video player | L, later | 65 MB (showtime) | waits for the browser's media stack (`web/NOTES/plans/browser/media.md`: web-media, V4L2 AVD) |

Staged, simulated: 1–3 plus the packaging cuts (xdg-utils, Qt theme, portal backend,
gnome-clocks) give 694 packages, 2.06 GB (Papers, Showtime, Firefox, cups, keyring still in);
also replacing the keyring and needrestart and adding PDF to Preview, 651, 1.97 GB (Showtime
still brings GTK4); without Showtime and cups too, 525, 1.68 GB, where Firefox is the only GTK
(GTK3) app left. Without Firefox that is the 1.31 GB lean base.
