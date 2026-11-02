# Mail depth: what a user needs to rely on Mail

Benchmarked against macOS Mail (Settings tabs and everyday features). "Have" is what Mail does
today; the ranked list at the end is what is left. Delete this plan when the list is done.

## Settings (Mail → Settings…, Ctrl+,)

| macOS tab | User need | Bande Mail |
|---|---|---|
| General | Default email reader | Mailto handler via Settings → Default Apps; the General tab points there |
| General | Check for new messages | Not needed: IMAP IDLE on the Inbox, 5 min poll without IDLE, 15 min full sync |
| General | New message sound / notifications | Have: toggles for notifications and their sound |
| General | Downloads folder, remove unedited downloads | Opened attachments are private copies removed on quit unless edited; Save to Downloads Folder in their menu; no folder choice |
| Accounts | Add, edit, remove IMAP/SMTP accounts | Have: list, Add Account… (setup sheet in the viewer), Remove… with confirmation, full name, server settings (user, hosts, ports, TLS/STARTTLS) checked against the servers before they are saved |
| Accounts | Provider presets (Gmail, Outlook, iCloud, Fastmail) | Autodiscovery (provider autoconfig, ISPDB, SRV, MX) finds them; a refused password names the provider's app-password page |
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
| Mailboxes and folders | Have: read all folders, Move To any mailbox of the account (message and Message menus); missing create, rename, delete |
| Flags | One flag; no colours |
| Search | Local, over sender, recipients, subject and text; no server search or tokens |
| Threads | Missing |
| Attachments | Show, open (private copy), Save to Downloads Folder, add from the portal; no Save As… to a chosen folder, no Quick Look |
| Reply / forward | Have, with threading headers |
| Drafts | Kept in Drafts when a message window closes or Mail quits; a local copy is saved while typing and reopens after a crash or an offline quit |
| Notifications | Have, Inbox only |
| Offline | Have: full offline store, queued changes |
| Selection | Ctrl+click and Shift+click select several messages for delete, archive, junk, move, flag and read; Shift+Up/Down and Select All; Undo puts the last move or delete back; no drag to mailbox |

## Remaining, ranked by how much users rely on it

1. Save Attachment… to a chosen folder (needs a portal SaveFile chooser) and Save All.
2. Drag messages to a mailbox; Redo and more than one level of Undo.
3. Conversations (threads) in the list and the reader.
4. OAuth2 for Gmail and Microsoft (owner decision: registered client IDs); Outlook.com refuses passwords.
5. Mailbox management: new, rename, delete; drag messages to a mailbox.
6. Sort order, flag colours (Mark All as Read is in the Mailbox menu).
7. Rules (move/flag/mark by sender, subject, account).
8. Rich text compose and spelling (both need desktop services first).
9. Enable/disable an account and change its password in Settings.
10. Per-sender "always load remote content"; a privacy proxy for remote pictures.
11. Message font and size, fixed-width plain text.
12. Server-side search and search tokens; CONDSTORE/QRESYNC (release review).
