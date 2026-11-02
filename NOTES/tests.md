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
| `apps/clock/tests/alarm_time.rs` `repeating_alarms_keep_their_wall_time_across_weekends_and_daylight_saving_changes` | Tricky date maths: a repeating alarm must ring at its wall time across a weekend and in another zone, at the first minute that exists when the clock springs forward over it, and once, at the first occurrence, when the clock falls back. |
| `apps/clock/tests/alarm_time.rs` `an_agent_started_late_rings_a_missed_alarm_once_but_not_a_stale_one` | The agent started by the wake-up timer or at login after the alarm's minute must still ring a missed one-off alarm, once, and turn it off, but not ring one more than an hour old. |
| `apps/terminal/tests/restore.rs` `windows_left_behind_are_restored_once_and_running_ones_are_left_alone` | Window restore: when several Terminals start, each saved window must open exactly once, and no Terminal may take the windows of one that is still running. |
| `apps/monitor/tests/signal.rs` `quit_and_force_quit_signal_the_sampled_process_and_never_a_reused_pid` | Against real `/proc`: Quit and Force Quit send `SIGTERM` and `SIGKILL` to a real sampled process, and a reused PID (another start time) or an exited process is never signalled. |
| `apps/software/tests/catalog.rs` `a_dep11_file_with_a_syntax_error_is_read_up_to_the_error_and_no_further` | Regression: a YAML syntax error used to make the catalogue loop forever. |
| `apps/software/tests/catalog.rs` `appstream_xml_that_declares_a_doctype_is_read` | Regression: Debian's AppStream XML declares a DOCTYPE, so the parser refused the file and lost all its apps. |
| `apps/installer/tests/deb.rs` `a_built_deb_opens_with_its_fields_and_a_truncated_download_is_refused` | End to end with `dpkg-deb`: a real `.deb` opens with its fields and its app's name and icon (read from the archive with size caps), and a truncated download is refused instead of shown. |
| `apps/calculator/tests/calculator.rs` `typing_and_clicking_calculate_exactly_with_precedence_in_every_mode` | End to end through the keypad widget, its keyboard bindings and its accessibility tree: decimal arithmetic without binary-float surprises (0.1 + 0.2 = 0.3), precedence and parentheses, exact trigonometry in degrees and radians, RPN, and programmer word sizes, bases and bits. |
| `apps/calculator/tests/calculator.rs` `errors_and_word_sizes_wrap_like_a_real_calculator` | Tricky integer logic: division by zero is an error the next key clears, entry stops at the word size, word flips and narrowing wrap, and a pasted negative number is two's complement in hex and signed in decimal. |
| `apps/velox/tests/engine.rs` `the_list_follows_events_and_reconciles_when_the_engine_stops_and_returns` | End to end against a fake Engine API on a unix socket (`DOCKER_HOST`): with no engine the not-ready view offers copyable steps; when the socket appears the list and live events arrive without a restart; when the engine goes away and comes back, Velox reconnects and lists again. |
| `apps/velox/src/engine/http.rs` `chunked_bodies_decode_across_split_chunks_extensions_and_trailers` | Tricky parser: the engine's chunked streams split JSON across chunks and may carry extensions and trailers. |
| `apps/preview/tests/open.rs` `a_folder_steps_through_pictures_and_hostile_files_never_panic_hang_or_wake` | End to end through Preview's window view: a picture opens, Next steps through its folder and wraps around, an animated GIF plays and schedules no wake-up once paused (nor does a still picture), and a truncated file, a GIF that declares 65535 × 65535 pixels and a FIFO each show an error instead of panicking, allocating the declared size or hanging. |

`ci/gate.sh` runs all of them on a private session bus. `ci/check-app.sh` and `ci/smoke.sh` check
packaging statically and on the installed package.
