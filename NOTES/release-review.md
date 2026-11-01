# Open review findings

From the pre-release review of Mail and Calendar and the apps component review (2026-11-01).
Fixed items are in git history. Before tagging `mail-v0.1.0` and `calendar-v0.1.0`, decide the
medium items; only the 412 conflict and offline drafts on quit lose data.

## All apps
- Mail and Calendar wake about twice a second at idle (likely a toolkit thread); trace in the desktop.
- Activity Monitor's search field has no text input area (no `text_input_area` on `Screen`).
- Terminal reloads a settings change once per window.

## Calendar
- Medium: occurrences are expanded on every frame from DTSTART (`ui/paint.rs`), up to 50 000 periods per object; cache per range.
- Medium: a 412 conflict keeps the server copy and drops the local edit; keep the local one as a copy.
- Medium: every sync clones all remote calendars for the worker.
- Low: cross-host HTTPS redirects keep the `Authorization` header.
- Low: XML parsing has no recursion depth limit (64 MB body cap); 303 keeps PROPFIND.
- Low: `Credentials`/`Job` derive `Debug` with the password (nothing logs them).
- Low: the keyring prompt has no timeout and blocks the sync thread.
- Low: `fired` alert keys are never pruned; sync-collection runs even when the ctag is unchanged.
- Low: the whole cache is parsed before the first frame.

## Mail
- Medium: every IDLE wake runs `UID SEARCH ALL` and FETCH FLAGS on the whole Inbox (no CONDSTORE/QRESYNC).
- Medium: after the first sync (5000 headers) the rest is fetched, and bodies under 2 MB are fetched ahead in every folder.
- Medium: SRV/MX discovery trusts unauthenticated DNS; confirm a host outside the address's domain.
- Medium: message parsing runs on the UI thread; drafts that cannot reach the server on quit are lost.
- Medium: a concurrent sync can flip a local Seen flag back until the queued change is flushed.
- Medium: the unified Inbox is loaded twice before the first frame.
- Low: without UIDPLUS, `EXPUNGE` removes every `\Deleted` message in the folder.
- Low: an SMTP server that advertises no AUTH skips sign-in and `verify` passes.
- Low: passwords are not zeroized in the worker.
- Low: no TCP keepalive on the IDLE connection (up to 25 min to notice a dead link).
