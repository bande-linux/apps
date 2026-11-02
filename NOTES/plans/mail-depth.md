# Mail depth: what a user needs to rely on Mail

Benchmarked against macOS Mail (Settings tabs and everyday features). "Have" is what Mail does
today; the ranked list at the end is what is left. Delete this plan when the list is done.

## Settings (Mail → Settings…, Ctrl+,)

| macOS tab | User need | Bande Mail |
|---|---|---|
| General | Default email reader | Mailto handler via Settings → Default Apps; the General tab points there |
| General | Check for new messages | Not needed: IMAP IDLE on the Inbox, 5 min poll without IDLE, 15 min full sync |
| General | New message sound / notifications | Have: toggles for notifications and their sound |
| General | Downloads folder, remove unedited downloads | Opened attachments are private copies removed on quit unless edited or still open in another app; Save Attachment… and Save to Downloads Folder in their menu |
| Accounts | Add, edit, remove IMAP/SMTP accounts | Have: list, Add Account… (setup sheet in the viewer), Remove… with confirmation, full name, server settings (user, hosts, ports, TLS/STARTTLS) checked against the servers before they are saved |
| Accounts | Provider presets (Gmail, Outlook, iCloud, Fastmail) | Autodiscovery (provider autoconfig, ISPDB, SRV, MX) finds them; a refused password names the provider's app-password page |
| Accounts | OAuth sign-in | Missing: Gmail and Outlook need app passwords |
| Accounts | Enable/disable an account, change its password | Have "Use this account"; the password is changed in the sheet that appears when the server refuses it |
| Junk | Junk filtering | Server-side only; Move to Junk files a message |
| Fonts & Colors | Message font and size, fixed-width plain text | Fixed-width plain text and "show plain text" in Viewing; sizes follow the desktop text size |
| Viewing | Remote content off by default | Have: blocked until Load Remote Content, remembered per sender (Forget in Viewing), or always loaded when turned on in Viewing |
| Viewing | Privacy protection (hide IP) | Partly: remote pictures only over HTTPS, capped; no proxy, so loading reveals the IP |
| Viewing | List preview lines | Have: None to 5 lines |
| Viewing | Threading (conversations) | Have: one row per conversation with a count, expanded in place; on by default |
| Composing | Message format (rich text) | Plain text only |
| Composing | Quote original, default send account | Have: quote toggle, "Send new messages from" |
| Composing | Check spelling | Missing (no desktop spell service) |
| Signatures | Per-account signature | Have: in Accounts, added to new messages, replies and forwards above the quote, swapped when From changes |
| Rules | Filters | Have: any or all of several From/To/Subject contains conditions → move, flag or mark read, on new unread Inbox mail while Mail runs, or on chosen messages with Apply Rules; Edit and Remove each rule |

## Core features

| Feature | Bande Mail |
|---|---|
| Unified inbox, smart mailboxes | Have: Inbox, Flagged, Drafts, Sent, Archive, Junk, Trash across accounts |
| Mailboxes and folders | Have: read all folders, Move To any mailbox of the account, drag onto a mailbox, New/Rename/Delete Mailbox |
| Flags | Have: seven colours (Message → Flag), kept on the server as `$MailFlagBit0-2` like macOS |
| Search | Local, every word over sender, recipients, subject and text, with from:/to:/subject: and quoted phrases; no server search |
| Threads | Grouped by the first References entry, or by subject for a reply without one; the reader stacks the whole conversation with Sent replies, earlier messages folded to a line, quoted text folded; message commands act on the reader's message while it has focus |
| Attachments | Show, open (private copy), Save Attachment…, Save to Downloads Folder, add from the portal; no Save All, no Quick Look |
| Reply / forward | Have, with threading headers |
| Drafts | Kept in Drafts when a message window closes or Mail quits; a local copy is saved while typing and reopens after a crash or an offline quit |
| Notifications | Have, Inbox only |
| Offline | Have: full offline store, queued changes |
| Selection | Ctrl+click and Shift+click select several messages for delete, archive, junk, move, flag and read; Shift+Up/Down and Select All; Undo puts moves and deletes back, 20 steps; drag onto a sidebar mailbox moves |

## Remaining, ranked by how much users rely on it

1. OAuth2 for Gmail and Microsoft (owner decision: registered client IDs); Outlook.com refuses
   passwords.
2. Rich text compose and spelling (both need desktop services first).
3. Server-side search; CONDSTORE/QRESYNC (release review).
4. A privacy proxy for remote pictures.
5. Redo; dragging messages out to other apps.
6. A Password field in Settings → Accounts; a message font apart from the desktop text size.
