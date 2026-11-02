# Bande Calendar

A calendar for the Bande desktop, modelled on macOS Calendar: CalDAV accounts that work offline,
a local "On My Computer" calendar, and read-only subscriptions, shown in day, week, month and year
views. Packaged as `bande-calendar`.

## What it does

- **Accounts.** CalDAV (RFC 4791) with discovery: a full URL, a host name or just an e-mail
  address. It tries `_caldavs._tcp` DNS SRV records, `/.well-known/caldav`, then
  `current-user-principal` → `calendar-home-set` → the calendars in the home set. Passwords go to
  the Secret Service (gnome-keyring), never to disk, and are sent only over HTTPS (or to this
  computer, for a local server). Several accounts, plus a local calendar.
- **Subscriptions.** `webcal:` / `webcals:` links and plain `https:` `.ics` URLs, read-only,
  refreshed on a wall-clock deadline kept across restarts (every 5 minutes to every week, or
  never); the name and colour come from `X-WR-CALNAME` and `X-APPLE-CALENDAR-COLOR`.
- **Offline sync.** Every calendar is cached on disk with its pending changes, so it works
  without a network and pushes changes later. Sync is incremental with `sync-token`
  (RFC 6578), falling back to ctag + etag listings on servers without it. Creates use
  `If-None-Match: *`, edits and deletes `If-Match`; when an edit meets a 412, the server's
  version stays and the local edit is kept beside it as a new event, and the sidebar says so.
- **iCalendar.** Its own parser and writer: line folding, escaping, unknown properties and
  components kept on round trip, `RRULE` (every frequency and `BY*` part, `COUNT`, `UNTIL`,
  `BYSETPOS`, week numbers), `EXDATE`, `RDATE`, `RECURRENCE-ID` overrides, all-day events,
  `VALARM`, and time zones: `TZID`s resolve through the system tz database (also Outlook's
  Windows names and prefixed Mozilla names), unknown ones through their `VTIMEZONE` (converted to
  a POSIX rule). Written events carry a `VTIMEZONE` generated from the tz database.
- **Views.** Day, week, month and year, a mini month and the calendar list in the sidebar,
  per-calendar colours and visibility, Today and previous/next.
- **Calendars.** New, rename and delete, on this computer or on a CalDAV server (MKCALENDAR,
  PROPPATCH of the name and colour, DELETE): File ▸ New Calendar…, or right-click a calendar or
  an account heading.
- **Busy hours.** Overlapping events share the column and widen into free lanes; when there are
  more than fit readably, a "+N" chip opens that day in the Day view.
- **Editing.** Drag on empty time to create, drag an event to move it (also between days in the
  month view and into the all-day row), drag its bottom edge to change its length, and an
  inspector popover with title, location, all-day, start and end, repeat, alert, calendar and
  notes. Changing a recurring event asks whether it is for this event only (an override) or
  all events.
- **Search** over title, location and notes, nearest occurrence first.
- **Alerts** through `org.freedesktop.Notifications`, also while the window is closed. Alerts,
  the clock and refreshes wait on one `CLOCK_REALTIME` timer, so they fire on time after a
  suspend or a clock change.
- **Settings** (Calendar ▸ Settings…): default calendar, start of the week, the hours the day
  starts and ends (the rest is shaded, and the week opens at the start), week numbers, default
  alerts for new events and all-day events, how often accounts and subscriptions refresh, and
  adding and removing accounts. Stored in `~/.config/bande/calendar.conf`.
- **Files and links.** `bande-calendar file.ics` offers to add its events; `bande-calendar
  webcal://…` offers to subscribe. The desktop file registers `text/calendar` and the `webcal`
  schemes.

Shortcuts: Ctrl+1…4 for the views, Ctrl+T today, Left/Right previous/next, Ctrl+N new event,
Alt+Ctrl+N new calendar, Ctrl+F search, Ctrl+R refresh, Delete to delete, Enter to open the
selected event, Ctrl+W close and Ctrl+Q quit.

## Layout

| Path | What it is |
|---|---|
| `src/ical/` | iCalendar: content lines, values and durations, time zones, recurrence, events |
| `src/caldav/` | HTTP, WebDAV XML, DNS SRV, discovery and the CalDAV requests |
| `src/store.rs` | accounts, calendars and the offline cache in `~/.local/share/bande-calendar` |
| `src/sync.rs` | push pending changes, pull with sync-token or ctag/etag, apply the outcome |
| `src/edit.rs` | the edits the UI makes: create, move a series or one occurrence, delete, details |
| `src/alerts.rs` | which alarms fall due |
| `src/settings.rs` | the settings and the messages the Settings window sends the app |
| `src/wake.rs` | the wall-clock timer for alerts, the clock and refreshes |
| `src/secrets.rs` | the Secret Service |
| `src/worker.rs` | the network thread the window talks to |
| `src/ui/` | the window: toolbar, sidebar, the four views, inspector, account sheet, input, Settings |
| `src/main.rs` | the app: window, menus, sync timers, alerts, files and links |

It is a `bande-app` app on the desktop's toolkit, design tokens and theme, like Files and
Settings; the desktop pin is shared by the workspace (see `../../README.md`).

## Tests

`tests/untrusted.rs` opens a hostile `.ics` file without panicking, refuses a FIFO or a file over
64 MiB, and never sends a password over plain HTTP to anything but this computer. The repo's
whole test list and the reason for each is in `NOTES/tests.md`.

## Not done yet

- Invitations (attendees, `METHOD:REQUEST`/`REPLY`), free/busy, tasks (`VTODO`).
- OAuth providers (Google, Microsoft); Basic authentication only, so iCloud and Fastmail work with
  an app password.
- Editing the time zone of an event; custom repeat rules (they are kept and shown as Custom,
  but not edited).
