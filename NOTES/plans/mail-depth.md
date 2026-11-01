# Mail depth: what a user needs to rely on Mail

Benchmarked against macOS Mail (Settings tabs and everyday features). "Have" is what Mail does
today; the ranked list at the end is what is left. Delete this plan when the list is done.

## Settings (Mail → Settings…, Ctrl+,)

| macOS tab | User need | Bande Mail |
|---|---|---|
| General | Default email reader | Mailto handler via Settings → Default Apps; the General tab points there |
| General | Check for new messages | Not needed: IMAP IDLE on the Inbox, 5 min poll without IDLE, 15 min full sync |
| General | New message sound / notifications | Have: toggles for notifications and their sound |
| General | Downloads folder, remove unedited downloads | Missing: opening an attachment writes it into Downloads and leaves it there |
| Accounts | Add, edit, remove IMAP/SMTP accounts | Have: list, Add Account… (setup sheet in the viewer), Remove… with confirmation, full name, server settings (user, hosts, ports, TLS/STARTTLS) checked against the servers before they are saved |
| Accounts | Provider presets (Gmail, Outlook, iCloud, Fastmail) | Autodiscovery (provider autoconfig, ISPDB, SRV, MX) finds them; no provider hint for app passwords yet |
| Accounts | OAuth sign-in | Missing: Gmail and Outlook need app passwords |
| Accounts | Enable/disable an account, change its password | Missing in Settings (the password sheet appears when the server refuses it) |
| Junk | Junk filtering | Server-side only; Move to Junk files a message |
| Fonts & Colors | Message font and size, fixed-width plain text | Missing (follows the desktop text size) |
| Viewing | Remote content off by default | Have: blocked until Load Remote Content, or always loaded when turned on in Viewing |
| Viewing | Privacy protection (hide IP) | Partly: remote pictures only over HTTPS, capped; no proxy, so loading reveals the IP |
| Viewing | List preview lines | Have: None to 5 lines |
| Viewing | Threading (conversations) | Missing |
| Composing | Message format (rich text) | Plain text only |
| Composing | Quote original, default send account | Have: quote toggle, "Send new messages from" |
| Composing | Check spelling | Missing (no desktop spell service) |
| Signatures | Per-account signature | Have: in Accounts, added to new messages, replies and forwards above the quote, swapped when From changes |
| Rules | Filters | Missing |

## Core features

| Feature | Bande Mail |
|---|---|
| Unified inbox, smart mailboxes | Have: Inbox, Flagged, Drafts, Sent, Archive, Junk, Trash across accounts |
| Mailboxes and folders | Have: read all folders; missing create, rename, delete, move between folders |
| Flags | One flag; no colours |
| Search | Local, over sender, recipients, subject and text; no server search or tokens |
| Threads | Missing |
| Attachments | Show, open (saved to Downloads), add from the portal; no Save As, no Quick Look |
| Reply / forward | Have, with threading headers |
| Drafts | Kept when a message window closes or Mail quits; not saved while typing |
| Notifications | Have, Inbox only |
| Offline | Have: full offline store, queued changes |
| Selection | Single message only; no multiple selection, drag to mailbox, undo |

## Remaining, ranked by how much users rely on it

1. Drafts saved while typing (a crash loses the message today).
2. Attachments: open from a private cache, Save Attachment… to a chosen folder, remove unedited downloads.
3. Multiple selection, drag to mailbox, Undo for delete/move.
4. Conversations (threads) in the list and the reader.
5. App-password hint for Gmail, Outlook, iCloud and Yahoo when sign-in fails; OAuth2 later (owner decision: client IDs).
6. Mailbox management: new, rename, delete, move to any mailbox.
7. Mark All as Read, sort order, flag colours.
8. Rules (move/flag/mark by sender, subject, account).
9. Rich text compose and spelling (both need desktop services first).
10. Enable/disable an account and change its password in Settings.
11. Per-sender "always load remote content"; a privacy proxy for remote pictures.
12. Message font and size, fixed-width plain text.
13. Server-side search and search tokens; CONDSTORE/QRESYNC (release review).
