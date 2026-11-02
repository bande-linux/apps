# Bande Clock

Alarms, timers, a stopwatch and a world clock for the Bande desktop, modelled on macOS Clock.
Packaged as `bande-clock`; it replaces gnome-clocks.

## What it does

- **Alarms.** A time, the days it repeats (none for a one-off alarm that turns itself off after
  ringing), a label, one of the desktop's alert sounds and snooze (9 minutes). A repeating alarm
  keeps its wall time across daylight saving changes: in a skipped hour it rings at the first
  minute that exists, in a repeated hour once, at the first occurrence. It follows a change of
  the system time zone at once.
- **Timers.** Several at once, from presets (1 minute to 1 hour) or a length, label and sound of
  your own; start, pause, resume, cancel and delete.
- **Stopwatch** with laps, newest first.
- **World clock.** Cities from the tz database (`zone.tab` and `iso3166.tab`, searched by city,
  country or zone), with the day and the difference from here, and whether it is night there.
- **Ringing.** An alarm or a finished timer posts a critical notification through
  `org.freedesktop.Notifications` (bande-notify) with the chosen sound, and repeats the sound
  through bande-sound every 3 seconds for up to 5 minutes, until it is snoozed, stopped, opened
  or dismissed. A timer's notification offers Repeat.
- **With the window closed.** As Calendar's agent, `/etc/xdg/autostart/org.bande.ClockAgent.desktop`
  starts `bande-clock --background` at login: a process with no window that rings alarms and
  timers, and that Quit (Ctrl+Q) returns to. It runs only while an alarm is on, snoozed or a
  timer runs; otherwise `--background` exits at once and Quit quits.
- **The Clock menu.** The app serves `org.bande.Clock` at `/org/bande/Clock` on its bus name,
  read by the shell: `Timers` (`a(sx)`: name and end, Unix seconds, of each running timer) and
  `NextAlarm` (`(sx)`: name and time, or `0` for none), with `PropertiesChanged`. The shell's
  Clock menu lists them and offers Open Clock.

Shortcuts: Ctrl+1…4 for World Clock, Alarms, Stopwatch and Timers, Ctrl+N new alarm,
Shift+Ctrl+N new timer, Ctrl+T start or stop the stopwatch, Ctrl+L lap or reset, Ctrl+W close and
Ctrl+Q quit.

## Deadlines and sleep

Nothing polls. Alarms wait on one `CLOCK_REALTIME` timerfd (absolute, `TFD_TIMER_CANCEL_ON_SET`,
so a clock change re-checks them), timers on one `CLOCK_BOOTTIME` timerfd, which counts time
asleep, so both ring at once on resume when their time passed during sleep. Without a window and
with nothing ringing the agent has no timeouts at all. The window's countdowns tick once a second
and the world clock once a minute, only while the window is visible; the running stopwatch redraws
with the compositor's frames, which stop when the window is hidden.

To wake the computer, Clock first asks for `CLOCK_REALTIME_ALARM` and `CLOCK_BOOTTIME_ALARM`
timerfds, which need `CAP_WAKE_ALARM`; an app does not have it (`EPERM`), so it also keeps one
transient systemd user timer (`bande-clock-wake-*.timer`) at the next alarm or timer end, which
starts `bande-clock --background` should the agent not be running. Debian's user manager holds
`CAP_WAKE_ALARM`, so where an RTC has a wake alarm (`/sys/class/rtc/*/wakealarm`) the timer gets
`WakeSystem=true` and wakes the computer from sleep. On the M2 Mac mini it cannot: the
`macsmc-rtc` RTC has no wake alarm, so a `WakeSystem=true` timer fails to start there
(`Failed to add realtime event source: Operation not supported`) and Clock leaves it out; sleep is
masked there for now anyway (`HARDWARE-GAPS.md`). After a resume the alarms and timers whose time
passed ring at once.

## Layout

| Path | What it is |
|---|---|
| `src/model.rs` | alarms (repeat days, next ring in a time zone), timers, the stopwatch, the saved state in `~/.local/share/bande-clock/clock.json` |
| `src/deadline.rs` | the wall-clock and boot-clock timerfds |
| `src/notice.rs` | ringing notifications and their actions |
| `src/bus.rs` | `org.bande.Clock` for the shell and the systemd wake-up timer |
| `src/cities.rs` | cities from the tz database |
| `src/ui/` | the window: the four pages and the alarm, timer and city sheets |
| `src/main.rs` | the app: window, menus, the agent, ringing |

## Tests

`tests/alarm_time.rs` checks the date maths: repeating alarms across weekends, both daylight
saving changes and another time zone, and that an agent started late rings a missed one-off
alarm once but not one more than an hour old.

## Not done yet

- Alarm sounds are the four short desktop alert sounds; there is no longer ringtone.
- The alarm sheet takes the time in 24-hour steppers whatever the clock format.
- No reordering of world clock cities, and no analogue faces.
