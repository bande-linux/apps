# Tests

The apps keep only a few tests (CLAUDE.md, directive 2): end-to-end scenarios or regressions for a
real bug or security boundary, about 500 lines for the whole repo, and no screenshot or golden
tests. Each test is listed here with its reason for staying. If you add or remove a test, update
this list.

| Test | Why it stays |
|---|---|
| `apps/mail/tests/send.rs` `a_message_goes_out_once_without_injected_headers_after_the_imap_session_went_stale` | Uses a real dovecot and an SMTP listener. When the server drops the IMAP session, a message must still go out only once and be filed in Sent; it went out twice before the fix. It also checks that CR/LF in a name or subject cannot inject a `Bcc:` or another header, and that Bcc recipients never appear in the message. Skips itself when `/usr/sbin/dovecot` is not installed. |
| `apps/mail/tests/send.rs` `a_mailto_link_cannot_add_hidden_recipients_or_attachments` | Security boundary: a web page's `mailto:` link must not add Bcc recipients or attach local files. |
| `apps/mail/src/smtp.rs` `addresses_cannot_smuggle_commands` | Security boundary: an address with `>` or CR/LF must not inject SMTP commands. |
| `apps/calendar/tests/untrusted.rs` `a_hostile_fifo_or_huge_calendar_file_opens_without_panicking_hanging_or_being_read_whole` | Regression: huge `INTERVAL` values, `BYDAY` that is not ASCII, oversized `BY*` lists and overflowing `DURATION`s used to panic or blow up during expansion; opening a FIFO used to hang Calendar, and a huge `.ics` file was read whole. |
| `apps/calendar/tests/untrusted.rs` `a_password_is_sent_only_over_https_or_to_this_computer` | Security boundary: Basic authentication must never go over plain HTTP, except to loopback. |
| `apps/calendar/tests/time_zone.rs` `a_series_moved_to_another_zone_keeps_its_times_exceptions_and_end` | Tricky date maths: moving a repeating event to another time zone must keep its wall times, its exceptions (`EXDATE`) and its `UNTIL` end in the new zone, or occurrences shift by hours or reappear. |
| `apps/terminal/tests/restore.rs` `windows_left_behind_are_restored_once_and_running_ones_are_left_alone` | Window restore: when several Terminals start, each saved window must open exactly once, and no Terminal may take the windows of one that is still running. |
| `apps/monitor/tests/signal.rs` `quit_and_force_quit_signal_the_sampled_process_and_never_a_reused_pid` | Against real `/proc`: Quit and Force Quit send `SIGTERM` and `SIGKILL` to a real sampled process, and a reused PID (another start time) or an exited process is never signalled. |
| `apps/software/tests/catalog.rs` `a_dep11_file_with_a_syntax_error_is_read_up_to_the_error_and_no_further` | Regression: a YAML syntax error used to make the catalogue loop forever. |
| `apps/software/tests/catalog.rs` `appstream_xml_that_declares_a_doctype_is_read` | Regression: Debian's AppStream XML declares a DOCTYPE, so the parser refused the file and lost all its apps. |
| `apps/installer/tests/deb.rs` `a_built_deb_opens_with_its_fields_and_a_truncated_download_is_refused` | End to end with `dpkg-deb`: a real `.deb` opens with its fields and its app's name and icon (read from the archive with size caps), and a truncated download is refused instead of shown. |

`ci/gate.sh` runs all of them on a private session bus. `ci/check-app.sh` and `ci/smoke.sh` check
packaging statically and on the installed package.
