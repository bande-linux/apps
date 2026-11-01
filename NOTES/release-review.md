# Pre-release review of Mail and Calendar (2026-11-01)

A read of all of `apps/mail/src` and `apps/calendar/src` before the first `mail-v0.1.0` and
`calendar-v0.1.0` tags, looking for crashes, data loss, security gaps and idle cost. Fixed items
say so; the rest is what is left before or after tagging.

## Tooling (M6 of the desktop's apps-split plan)

- `ci/gate.sh [app...]`: fmt, clippy `--all-targets -D warnings`, tests, all `--locked`, on a
  private session bus (`ci/test-bus.conf`), one shared `target/`, under `gentle` and
  `~/bande-linux/kbuild.lock`, waiting while `kbuild.reserve` exists.
- `ci/bump-desktop.sh [REV]`: pins every app to one desktop commit on main (default: its head),
  re-locks only the desktop source and runs the gate. Both apps now pin a6a68588.
- `CLAUDE.md`: the desktop's core directives plus the apps' gate and pin rules.
- Bump fallout fixed: app shortcuts moved from Super to Ctrl (menus, key handling, README, tests),
  Ctrl+W and Ctrl+Q handled by each app through `Standard::for_shortcut` (the desktop no longer
  sends them), the new `Standard::Settings` arm, `danger_text` for red text, and Mail's sidebar
  selection uses `sidebar_selection` with the window's active state.
- Calendar and the template get Mail's release profile (thin LTO, strip) and the
  unwrap/expect/panic lints; Calendar's smoke test checks its desktop file and icon.

## Calendar

### High (fixed)
1. Hostile iCalendar data panicked the app, and the cache reparses at start, so a crash loop:
   jiff's `days()`/`years()`/… panic out of range (`DURATION:P99999999D`, `TRIGGER`,
   `FREQ=YEARLY;INTERVAL=20000`), `BYDAY=€` sliced a char in two. Durations are now parsed with
   checked arithmetic and capped at 100 years, spans use `try_*`, `BYDAY` uses `get`.
2. The password went over plain HTTP: `_caldav._tcp` SRV records produced `http://` candidates
   and typed `http://` servers were accepted, so Basic auth travelled in clear text (and a
   redirect could downgrade). The HTTP client now refuses to attach credentials to anything but
   `https://` or loopback, and discovery only uses `_caldavs._tcp`.
3. Unbounded recurrence candidates: repeated `BYHOUR`/`BYMINUTE`/`BYSECOND` lists multiplied into
   gigabytes per period. `BY*` lists are de-duplicated at parse and a period yields at most
   10 000 candidates.
4. Typing "12€" in a time field panicked (byte slicing).

### Medium
- Fixed: a failed subscription fetch (error page, truncated body, read error) became an empty
  calendar and wiped every event. Body read errors now propagate, a body without `VCALENDAR` is
  an error, and refresh keeps the old events.
- Fixed: a corrupt `accounts.json` was replaced by a new one, orphaning every account and its
  pending changes; corrupt files are now kept as `*.broken`. Store writes are fsynced, use a
  per-process temp name, and are created 0600 in a 0700 directory.
- Fixed: alerts could fire up to an hour late after suspend (monotonic deadline); the next due
  time is now also checked against the wall clock on each minute tick. The clock tick is aligned
  to the minute (60 s instead of 30 s, the now-line moves on time).
- Left: occurrences are expanded on every frame (`ui/paint.rs` → `Library::occurrences`), from
  DTSTART up to the view's end; old MINUTELY/SECONDLY rules cost up to 50 000 periods per
  object per frame and silently stop after 50 000 periods. Cache per range or fast-forward.
- Left: a 412 conflict keeps the server copy and drops the local edit (reported by title only).
  Keep the local version as a copy.
- Left: every sync clones all remote calendars (data and parsed objects) for the worker.

### Low (left)
- Cross-host HTTPS redirects keep the `Authorization` header (needed for some providers'
  `.well-known` redirects; consider asking when the host leaves the user's domain).
- XML parsing recursion has no depth limit (64 MB body cap); 303 keeps PROPFIND.
- `Credentials`/`Job` derive `Debug` with the password; nothing logs them today.
- The keyring prompt waits without a timeout and blocks the sync thread.
- `fired` alert keys are never pruned; sync-collection runs even when the ctag is unchanged.
- The whole cache is read and parsed before the first frame (startup grows with the cache).

### Idle
CalDAV every 5 min per account, subscriptions hourly, both once at start; no network with only
the local calendar. Alerts sleep until the next due alarm (≤ 1 h). One minute-aligned clock
repaint. No busy loops; the worker blocks on its channel.

## Mail

### High (fixed)
1. A sent mail could go out twice: after SMTP succeeded, a stale IMAP session failing while
   filing the copy re-ran the whole Send, SMTP included, and could report "not sent". Send is
   never retried now; filing (Sent copy, answered flag, draft removal) is retried once on a
   fresh session and only logged when it fails.
2. A wrong password (or a dismissed keyring prompt) was retried every 5 s forever with the same
   password, risking lockouts. The account now waits for a new password or a user action.
3. A `mailto:` link could add hidden Bcc recipients (no Bcc field in Compose); `bcc=` is ignored.
4. Quit (Ctrl+Q, the menu, the compositor) skipped saving open drafts; it now goes through the
   same path as closing the main window.
5. Nested `<ul>` in a ~1 MB HTML mail grew to tens of GB of indentation on the sync thread, on
   every sync. List indentation is capped at 8 levels.
6. Inline pictures were decoded once per `<img>`, without limits, on the UI thread. Sources are
   de-duplicated, at most 20 per message and 24 MP in total, each fitted into 1600×4800.

### Medium
- Fixed: every `Changed`/`Arrived`/`Body` event reloaded the whole list; a batch of events now
  reloads once. Unread counts use `COUNT(*)` instead of loading every summary.
- Fixed: IDLE stopped for good after a password change or a login error (`idling` never reset);
  the IDLE thread now clears it when it ends, a new password interrupts it, and its offline
  backoff parks instead of waking 4 times a second.
- Fixed: a malicious literal length could wrap the response-size check.
- Fixed: CR/LF in display names reached the headers (header injection); control characters are
  stripped.
- Fixed: the store was created with umask permissions; now 0700/0600.
- Left: every IDLE wake does `UID SEARCH ALL` and FETCH FLAGS for the whole Inbox (no
  CONDSTORE/QRESYNC). Fine for thousands of messages, slow for 100k.
- Left: `FIRST_SYNC_LIMIT` (5000 headers) holds for the first sync only; the next one fetches
  the rest, and bodies under 2 MB are fetched ahead for every folder (Gmail's All Mail too).
- Left: SRV/MX discovery trusts unauthenticated DNS; confirm a host outside the address's domain.
- Left: message parsing (`Content::parse`, `Reading::new`) runs on the UI thread, and drafts that
  cannot reach the server on quit are lost (no local draft store).
- Left: a concurrent sync can flip a local Seen flag back until the queued change is flushed.
- Left: the unified Inbox is loaded twice before the first frame.

### Low (left)
- Without UIDPLUS, `EXPUNGE` removes every `\Deleted` message in the folder.
- An SMTP server that advertises no AUTH skips sign-in, and `verify` passes.
- Passwords are not zeroized in the worker; never logged.
- No TCP keepalive on the IDLE connection (a dead link can take up to the 25 min re-IDLE).
- Fixed: attachments saved with `create_new` (no symlink follow) into the real XDG Downloads
  folder (`user-dirs.dirs`); every type still opens automatically after saving.

### Verified fine
TLS through native-tls with default verification; STARTTLS required when configured and data
before the handshake rejected; plain text only to exact loopback names; HTML sanitised (http,
https, mailto links only; remote pictures blocked until asked, HTTPS only, 20 × 10 MB); no
plaintext password fallback (Secret Service only); UIDVALIDITY change resets the mailbox; IDLE
re-issued every 25 min; the list paints only visible rows; the UI has no timers at idle.
