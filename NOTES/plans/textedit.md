# TextEdit

Bande TextEdit (`apps/textedit`, `bande-textedit`) is the default text handler, in the spirit of
macOS TextEdit. Delete this plan when the gaps below are done.

## What it does

- **Plain text.** Opens any file up to 512 MB on a thread: a BOM, then UTF-16 by its zero bytes,
  then UTF-8, else Western (Windows Latin 1); Reopen with Encoding and Format › Text Encoding
  choose any of 21 encodings, and a save that the encoding cannot hold offers UTF-8. Line
  endings (LF, CRLF, CR) are kept and can be changed. A 50 MB log opens in about 0.2 s and types
  without delay (the editor lays out only visible paragraphs). Find and replace with match case
  and regular expressions, Find Next/Previous, Use Selection for Find, Go to Line, a word and
  character count, Monospaced/Proportional, Bigger/Smaller, Wrap to Window/Page. Tabs line up
  at four-space stops.
- **Never lose text.** Every window has a record in `~/.local/state/bande/textedit/documents`:
  1.5 s after the last edit its unsaved text is written there on a thread, and Quit writes the
  rest. The next start reopens every window of the last session, with unsaved text, untitled
  ones included, as macOS does. Closing an edited window asks Save / Don't Save / Cancel; only
  then is its record dropped. Saves go to a temporary file in the same folder, keep the mode,
  are synced and renamed over the original (through symlinks).
- **Rich text.** New documents are rich text (RTF), as on macOS. Format has Bold, Italic,
  Underline, Strikethrough, Bigger/Smaller, typeface (System, Serif, Display, Monospaced), colour,
  alignment, bullet and numbered lists with indent levels, and Title/Heading/Subheading/Body
  styles; Make Plain Text and Make Rich Text convert. RTF, Markdown and HTML open as styled text
  and save back in their own format; a Markdown file can be shown as its source with Make Plain
  Text.

## Libraries

- `encoding_rs` (already in the lock through the web engine): every WHATWG encoding, decoding and
  encoding without replacement, so unmappable characters are found instead of written as `&#…;`.
  Detection is our own few lines (BOM, UTF-16 zero bytes, UTF-8 validity); `chardetng` would add a
  dependency for legacy CJK and Cyrillic guessing that Reopen with Encoding covers.
- `regex`: find and replace; patterns are limited to 16 MB compiled.
- RTF, Markdown and HTML are small parsers and writers of our own (`src/formats`): the subset
  TextEdit writes and macOS/Word produce, linear in the input, with nesting capped. HTML reuses
  `html5ever`'s tokenizer (as Mail does), so entities and broken markup are handled by the
  standard algorithm; scripts and styles are skipped.

## Gaps

- Windows only, no tabs in one window (the Window menu lists the documents).
- No spell checking: the desktop has no spell service yet.
- Word and OpenDocument files come next.
- No format bar: styles are in the Format menu.
- Pictures and tables are not shown; there is no page setup or printing.
- A file changed by another program while open is not noticed.
