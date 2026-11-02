# Terminal depth: what a user needs to rely on Terminal

Benchmarked against macOS Terminal. "Have" is what Terminal does today; the ranked list at the end
is what is left. Delete this plan when the list is done.

## Settings (Terminal → Settings…, Ctrl+,)

| macOS | User need | Bande Terminal |
|---|---|---|
| Profiles: text, colours, background | Readable, familiar colours | Five built-in profiles (Datum follows light/dark, Paper, Ink, Midnight, High Contrast) with a live preview. Profiles cannot be edited, duplicated or imported |
| Font and size | Comfortable text | Size 9–32 pt (Ctrl+= / Ctrl+-); the font is fixed (JetBrains Mono) |
| Cursor: block / underline / bar, blink | See the cursor | Have shape and Blink the cursor (blinks only while focused, so idle windows stay asleep) |
| Scrollback: limit lines | Keep enough history | Have: 1 000, 10 000, 100 000 or 1 000 000 lines |
| Bell: audible / visual, badge | Notice a bell | Have Play the alert sound (the desktop's alert through bande-sound, at most four a second, off by default), Flash the window, and a badge in the bar |
| Use Option as Meta key | Alt-word movement in shells and Emacs | Have Use Alt as Meta (on by default); off sends the character the layout produces |
| Shell: run command, startup | Use another shell or tmux | Have Run command (through `/bin/sh -c exec …`; empty = login shell) |
| When the shell exits, Ask before closing: Always / Never / Only if processes other than the shell run | Don't lose a running job | Have Always / When a program is running / Never, for panes, tabs, windows and quit |
| New tabs/windows open with: same working directory / home | Keep the folder | Have The same folder / Your home folder |
| Window size in columns × rows | Default size | Have Columns and Rows (80 × 24 by default) |
| Notify when long commands finish (not in macOS) | Background work | Have |

## Everyday features

| macOS | Bande Terminal |
|---|---|
| Tabs (Shift+Ctrl+T), select with Alt+1…9, move | Have; tab context menu, Close Other Tabs; tabs cannot be dragged to reorder |
| Split panes | Have side by side and stacked, keyboard pane switching |
| Find (Shift+Ctrl+F, next/previous) | Have, with regex-safe matching and highlights |
| Copy on select | Off, as macOS: selection fills the primary selection only; middle-click pastes it |
| Bracketed paste, paste safety | Have bracketed paste (escapes stripped); a banner asks before pasting several lines into a program that has not turned bracketed paste on (setting) |
| URLs and paths clickable | Have Ctrl+click and the context menu (Open Link, Copy Link, Show in Files) |
| Drag and drop files | Have: shell-quoted paths |
| Title from OSC 0/2, current folder from OSC 7 | Have |
| Marks and jump between commands | Have Jump to Previous/Next Command (Shift+Ctrl+Up/Down) |
| Option-click moves the cursor | Have Alt+click on the prompt line (arrow keys sent to the shell); Alt+drag still selects a block |
| Window restore | Have: windows and tab folders come back after a restart |
| Big output | Fast: `seq 1 2000000` finishes in 1.2 s (release build, headless); tabs (`\t`) used to draw as missing-glyph boxes, fixed |

## Ranked remaining

1. Editable profiles: duplicate a built-in profile, change colours, font size and cursor per
   profile, choose a profile per window or tab, import iTerm/Terminal colour schemes.
2. Font family choice (any installed monospace font).
3. Drag tabs to reorder and out into a new window.
4. Edit Title for a tab, and a per-tab profile badge.
5. Select a whole command's output (macOS: triple-click on output with marks).
6. Secure Keyboard Entry equivalent (ask the compositor to keep shortcuts out while typing a
   password).
