# Bande Mail

Mail for the Bande desktop, modelled on macOS Mail: mailboxes in a sidebar with a unified
Inbox, the message list, and the reader, in one window. It reads mail over IMAP, sends it over
SMTP, keeps an offline copy of every account and follows new mail with IMAP IDLE.

```
apt install bande-mail
```

## What it does

- **Accounts.** The setup sheet asks for a name, an address and a password, finds the servers
  and checks that both IMAP and SMTP accept the password before the account is added. If no
  servers are found, or they cannot be reached, the sheet shows the server fields, filled with
  whatever was found. Several accounts work side by side.
- **Finding servers**, in this order: the provider's own autoconfig
  (`autoconfig.<domain>`, then `<domain>/.well-known/autoconfig`), Mozilla's ISPDB
  (`autoconfig.thunderbird.net`), DNS SRV records (RFC 6186), the ISPDB entry of the domain's
  mail exchanger (custom domains on Google or Microsoft), and finally `imap.`/`mail.`/`smtp.`
  guesses that answer on the usual ports. Autoconfig is fetched over HTTPS only.
- **Security.** IMAP and SMTP use TLS or STARTTLS, verified against the system's certificate
  store. Plain text is refused for anything but this computer (`localhost`, `127.0.0.1`, `::1`),
  so a local bridge or test server still works. Passwords live in the Secret Service
  (gnome-keyring), under `application=org.bande.Mail`, never in a file.
- **Window.** The sidebar lists Inbox, Flagged, Drafts, Sent, Archive, Junk and Trash across all
  accounts, then each account's folders with unread counts. The list shows sender, subject, a
  two-line preview, the date, an unread dot, a flag and a paperclip. The reader shows the
  headers, the plain text or sanitised HTML, and the attachments; clicking an attachment saves it
  to Downloads and opens it.
- **HTML** is never rendered as a web page: it is turned into paragraphs, headings, lists,
  quotes and links. Scripts, styles, forms and frames are dropped, only `http`, `https` and
  `mailto` links survive, and no remote content is ever fetched; the reader says when a message
  had some.
- **Toolbar and keys.** Get Mail (⇧⌘N), New Message (⌘N), Archive (⇧⌘A), Delete (⌫), Junk
  (⇧⌘J), Reply (⌘R), Reply All (⇧⌘R), Forward (⇧⌘F), Flag (⇧⌘L), Mark as Read or Unread (⇧⌘U)
  and Search (⌘F), which searches sender, recipients, subject and text. ⌘ is the logo key.
- **Compose** opens its own window, with From when there are several accounts. Replies quote the
  message and thread it (`In-Reply-To`, `References`); forwards keep the attachments. A sent
  message is kept in Sent and the original is marked answered. If sending fails, the message
  comes back in a window with the error.
- **Offline store.** One SQLite database in `~/.local/share/bande-mail/` holds the accounts,
  folders, headers, flags and message bodies (bodies up to 2 MB are fetched ahead, larger ones
  when opened). Flag changes, moves and deletions apply at once and are queued until the
  server has them, so they survive going offline or quitting.
- **New mail.** A second connection IDLEs on the Inbox; servers without IDLE are polled every
  five minutes. New unread mail shows a notification through `org.freedesktop.Notifications`.
- **Integration.** `org.bande.Mail.desktop` registers Mail for `mailto:` links (choose it in
  Settings → Default Apps). A second `bande-mail mailto:…` hands the link to the running Mail
  over D-Bus (`org.bande.Mail`).

## Robustness

Every connection has a 15 s connect and 60 s read timeout, IMAP responses are parsed by a
small parser that rejects malformed input instead of panicking (deep nesting, short literals,
over-long lines and 128 MB responses are refused), and a lost connection is retried with
backoff from 5 s to 5 min. A changed `UIDVALIDITY` starts the mailbox over, a server without
`MOVE` gets `COPY` + `EXPUNGE`, and one without `SPECIAL-USE` has its folders recognised by
their usual English names.

## Building and testing

```
cargo test
```

- `tests/engine.rs` runs the engine against a scripted fake IMAP and SMTP server: first sync,
  flags both ways, moves and deletions, IDLE, sending with a copy in Sent, wrong passwords,
  a changed `UIDVALIDITY`, servers without `MOVE`, `SPECIAL-USE` or `IDLE`, and servers that
  never answer or send garbage.
- `tests/dovecot.rs` starts Debian's `dovecot` as the current user on a free loopback port and
  checks sync, flags, moves and IDLE against it. It skips itself when `/usr/sbin/dovecot` is not
  installed, so CI runs only the fake server.
- `tests/secrets.rs` checks the keyring code against a fake Secret Service on a private bus.
- `tests/views.rs` checks the window, the reader, the account sheet and their golden screenshots
  in light and dark, with the desktop's golden harness (`UNBOUND_UPDATE_GOLDENS=1` rewrites
  them).

The toolkit comes from the desktop through a path dependency for now (`unbound-*` crates in
`~/unbound-linux/repos/desktop`); it moves to a git dependency on `bande-linux/desktop` once
that is published.

## Dependencies

Each one is in Debian, except where noted:

| Crate | Why |
|---|---|
| `native-tls` | TLS for IMAP, SMTP and HTTPS through the system OpenSSL and CA store; one TLS stack, security updates from Debian |
| `mailparse` | MIME parsing, charsets and encoded words of incoming mail |
| `rusqlite` | the offline store, on the system `libsqlite3` |
| `html5ever` | a correct HTML tokenizer for turning mail HTML into safe text |
| `data-encoding` | base64 for MIME, `AUTH PLAIN` and modified UTF-7 (already pulled in by `mailparse`) |
| `zeroize` | keeps the typed password out of freed memory (already used by the toolkit) |
| `jiff`, `roxmltree`, `zbus`, `accesskit`, `tracing` | shared with the desktop |

No IMAP, SMTP, DNS or HTTP crate is used: the maintained IMAP crates are async-only or not in
Debian, and the desktop has no async runtime. The IMAP and SMTP clients, the SRV/MX lookup and
the HTTPS autoconfig fetch are small, synchronous and bounded by timeouts.

## Not yet

- Adding attachments in Compose (needs the file chooser portal), drafts saved to the server,
  signatures, rich text.
- Loading remote content on request, inline `cid:` images, printing.
- Conversations (threads), multiple selection, drag and drop to mailboxes, mailbox
  management (create, rename, delete folders).
- OAuth2 (Gmail and Outlook need app passwords for now), CONDSTORE/QRESYNC for faster flag
  sync on large mailboxes, full-text search on the server.
- Recognising localised folder names on servers without `SPECIAL-USE`.

## Licence

MIT OR Apache-2.0.
