# Calendar depth: what users need before they rely on it

Benchmark: macOS Calendar (Sonoma/Sequoia). The question for each area: can someone run their
work and family calendars on it without a second app? Reviewed 2026-11-02.

## Settings (macOS: General, Accounts, Alerts, Advanced)

| macOS | Bande Calendar |
|---|---|
| Default calendar | done: Settings ▸ General, used for new events |
| Start week on, day starts/ends, show N hours | done: start of week, day start/end (shaded, week opens there); hours-at-a-time zoom missing |
| Show week numbers | done: month rows and week header |
| Time zone support | done: Advanced toggle, zone per event in the inspector; whole-view zone missing |
| Default alerts for events / all-day events / birthdays | done for events and all-day; birthdays need Contacts |
| Birthdays calendar | missing (no Contacts store yet) |
| Accounts: add, edit, remove, refresh interval | done: presets (iCloud, Fastmail, Google, Nextcloud), password again, remove, refresh interval (1 min…hourly, manual) |
| Subscriptions: refresh interval, remove alerts | done: global and per subscription (5 min…week, never, kept across restarts); alerts dropped unless turned on |
| Alerts: shared calendar messages, invitation messages | n/a until iMIP |

## Core features

| Need | State |
|---|---|
| Create/edit with repeat, end repeat, invitees, alerts | done, with a Custom repeat sheet (interval, weekdays, nth weekday of month/year) |
| Drag to move or resize | done (also across days, into the all-day row) |
| Day/Week/Month/Year views | done; year view shows no event density |
| Search | done (title, location, notes) |
| Subscriptions (webcal) with a real refresh deadline | done: wall-clock deadline from the last refresh, persisted, 15 min retry on failure |
| .ics import and export | done: File ▸ Import…, File ▸ Export…, Export… on a calendar's menu |
| Alerts while windowless | done while the app runs (window closed); not after login until Calendar is opened |
| Time zones and DST | done: tz database, VTIMEZONE in and out, zone per event, DST-safe wall-clock timers, follows a system time zone change (timedate1) at once |
| Offline edits, conflicts | done: pending changes on disk, pushed as soon as NetworkManager reports the network back; a 412 keeps the server copy and the local edit as a new event |
| Invitations | done via implicit scheduling: invitees and answers shown, Accept/Maybe/Decline (an invitation's title and times stay read-only), add invitees, SEQUENCE raised by the organizer; your addresses come from `calendar-user-address-set` (accounts added before 2026-11-02 fall back to the login name) |
| Wrong/expired password | done: asked again on 401, or from Settings, pending changes kept |

## Landed 2026-11-02 (apps main)

Settings window; 412 keeps both versions; one `CLOCK_REALTIME` timerfd for alerts, clock and
refreshes (on time after suspend); per-calendar persisted subscription refresh; import/export
through the portal; end repeat; account presets; re-sign-in; invitations; time zone support;
occurrence cache per shown range; slim sync jobs; unchanged sync-token skip.

## Remaining, ranked by how much users rely on it

1. **Alerts after login.** Alerts fire only once Calendar has been opened. macOS runs an agent.
   Plan: `bande-calendar --background` starts with the window parked, plus an XDG autostart
   entry. Owner decision: it would show as a running app in the taskbar (needs a desktop
   "background app" notion), and the idle-wake bug (twice a second, release-review) must be
   fixed first so it costs nothing.
2. **View in another time zone** (macOS toolbar zone pop-up when time zone support is on).
3. **Subscription URL edit and published TTL**: honour `REFRESH-INTERVAL` / `X-PUBLISHED-TTL`
   as a subscription's default interval, and let its URL be changed.
4. **Invitee autocomplete and free/busy** (needs Contacts and CalDAV scheduling outbox POST).
5. **OAuth** for Google and Microsoft 365 (Basic only today; Google offered as a subscription).
6. **Birthdays calendar** from Contacts.
7. **Hours shown at a time** (day/week zoom) and event density in the year view.
8. **Shared file chooser helper**: `portal.rs` repeats Mail's; move it to a desktop crate.
