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
  whatever was found. When Gmail, iCloud, Yahoo, AOL or Fastmail refuse the password, the sheet
  says where to create an app password. Several accounts work side by side.
- **Settings** (Ctrl+,) has four tabs. General: notifications for new mail and their sound.
  Accounts: each account's full name, signature and server settings (user name, hosts, ports,
  TLS or STARTTLS), Add Account… and Remove…; changed servers are checked with the saved
  password before they are kept. Viewing: list preview lines and whether remote content loads
  by itself (off by default). Composing: the account new messages are sent from and whether
  replies quote the original. They are kept in `~/.config/bande/mail.conf`; signatures and
  servers in the offline store.
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
  preview (two lines unless Settings says otherwise), the date, an unread dot, a flag and a
  paperclip. The reader shows the headers, the plain text or sanitised HTML, and the
  attachments. Clicking an attachment opens a private copy in `~/.cache/bande-mail/attachments/`,
  removed when Mail quits unless it was edited; its menu also saves it where you choose (the
  desktop's file chooser) or to the Downloads folder.
- **HTML** is never rendered as a web page: it is turned into paragraphs, headings, lists,
  quotes, links and pictures. Scripts, styles, forms and frames are dropped, and only `http`,
  `https` and `mailto` links survive. Pictures sent inside the message (`cid:`) show at once;
  remote pictures are not fetched until **Load Remote Content** is clicked in the reader's
  banner (or by themselves when Settings → Viewing allows it), and then only over HTTPS, at
  most 20 per message, 10 MB each, decoded with limits on their size.
- **Toolbar and keys.** Get Mail (Shift+Ctrl+N), New Message (Ctrl+N), Archive (Shift+Ctrl+A),
  Delete (Delete or Backspace), Junk (Shift+Ctrl+J), Reply (Ctrl+R), Reply All (Shift+Ctrl+R),
  Forward (Shift+Ctrl+F), Flag (Shift+Ctrl+L), Mark as Read or Unread (Shift+Ctrl+U), Send
  (Shift+Ctrl+D), Close Window (Ctrl+W), Quit (Ctrl+Q) and Search (Ctrl+F), which searches
  sender, recipients, subject and text. Mailbox → Mark All as Read marks the shown mailbox read, and Message → Move To files the
  message in any mailbox of its account. Ctrl+click, Shift+click, Shift+Up/Down and Select All
  (Ctrl+A) select several messages; Delete, Archive, Junk, Move To, Flag and Mark as Read then
  act on all of them, and dragging them onto a mailbox in the sidebar moves them. Undo (Ctrl+Z)
  puts the last moved or deleted messages back, and a mailbox that messages were moved to is
  synced right after the move. Mailbox → New Mailbox… and a mailbox's menu in the sidebar
  create, rename and delete mailboxes on the server; the special mailboxes keep their names.
- **Compose** opens its own window, with From when there are several accounts. Replies quote the
  message and thread it (`In-Reply-To`, `References`); forwards keep the attachments. The account's
  signature goes above the quoted text and changes with From. Files are
  attached through the desktop's file chooser (the portal), up to 25 MB each, and a click on an
  attachment removes it. A sent message is kept in Sent and the original is marked answered. If
  sending fails, the message comes back in a window with the error.
- **Drafts.** Closing a message that was changed and not sent keeps it in the account's Drafts
  mailbox (also on quit). While a message is written, a copy is kept every few seconds in
  `~/.local/share/bande-mail/unsent/` until the server has it, so a message survives a crash or
  quitting offline: it opens again the next time Mail starts. Double-click a draft, or press Return, to carry on; saving it again or
  sending it removes the older copy.
- **Offline store.** One SQLite database in `~/.local/share/bande-mail/` holds the accounts,
  folders, headers, flags and message bodies (bodies up to 2 MB are fetched ahead, larger ones
  when opened). Flag changes, moves and deletions apply at once and are queued until the
  server has them, so they survive going offline or quitting.
- **New mail.** A second connection IDLEs on the Inbox; servers without IDLE are polled every
  five minutes. New unread mail shows a notification through `org.freedesktop.Notifications`.
- **Integration.** `org.bande.Mail.desktop` registers Mail for `mailto:` links (choose it in
  Settings → Default Apps). A second `bande-mail mailto:…` hands the link to the running Mail.

## Robustness

Every connection has a 15 s connect and 60 s read timeout, IMAP responses are parsed by a
small parser that rejects malformed input instead of panicking (deep nesting, short literals,
over-long lines and 128 MB responses are refused), and a lost connection is retried with
backoff from 5 s to 5 min. A changed `UIDVALIDITY` starts the mailbox over, a server without
`MOVE` gets `COPY` + `EXPUNGE`, and one without `SPECIAL-USE` has its folders recognised by
their usual English names.

## Building and testing

```
ci/gate.sh mail
```

`tests/send.rs` starts Debian's `dovecot` as the current user on a free loopback port, with a
small SMTP listener beside it, and checks that a message sent after the IMAP session went stale
goes out once, without injected headers or its Bcc, and is kept in Sent. It skips itself when
`/usr/sbin/dovecot` is not installed. It also checks that a `mailto:` link cannot add Bcc
recipients or attachments. The repo's whole test list is in `NOTES/tests.md`.

Mail is a `bande-app` app; the `bande-*` crates come from
[bande-linux/desktop](https://github.com/bande-linux/desktop) at the workspace's one pin
(`ci/bump-desktop.sh`, see `../../README.md`).

## Dependencies

Each one is in Debian, except where noted:

| Crate | Why |
|---|---|
| `native-tls` | TLS for IMAP, SMTP and HTTPS through the system OpenSSL and CA store; one TLS stack, security updates from Debian |
| `mailparse` | MIME parsing, charsets and encoded words of incoming mail |
| `rusqlite` | the offline store, on the system `libsqlite3` |
| `html5ever` | a correct HTML tokenizer for turning mail HTML into safe text |
| `image` | PNG, JPEG and GIF pictures in mail, with the same version and features as the toolkit |
| `data-encoding` | base64 for MIME, `AUTH PLAIN` and modified UTF-7 (already pulled in by `mailparse`) |
| `zeroize` | keeps the typed password out of freed memory (already used by the toolkit) |
| `jiff`, `roxmltree`, `zbus`, `accesskit`, `tracing` | shared with the desktop |

No IMAP, SMTP, DNS or HTTP crate is used: the maintained IMAP crates are async-only or not in
Debian, and the desktop has no async runtime. The IMAP and SMTP clients, the SRV/MX lookup and
the HTTPS autoconfig fetch are small, synchronous and bounded by timeouts.

## Not yet

- Rich text, drag and drop of files into Compose. The ranked list
  of what users still need is in `../../NOTES/plans/mail-depth.md`.
- Remote pictures over plain `http` (they show their alt text), SVG pictures, printing.
- Conversations (threads).
- OAuth2 (Gmail and Outlook need app passwords for now), CONDSTORE/QRESYNC for faster flag
  sync on large mailboxes, full-text search on the server.
- Recognising localised folder names on servers without `SPECIAL-USE`.

## Licence

MIT OR Apache-2.0.
