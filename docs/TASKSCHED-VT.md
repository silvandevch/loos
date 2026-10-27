# Task Scheduler core and VT console core

## `crates/tasksched` (`loos-tasksched`, task #165)
Deterministic scheduler state machine. Triggers: Once, Daily(every N days), Weekly(mask), Monthly(day, clamped), Interval, Boot, Logon. `Scheduler::poll(now)` returns task ids to launch and advances schedules; fire times older than `GRACE` (60 s) are "missed" and obey `MissedPolicy::{Skip,RunOnce}`. `no_overlap` suppresses starts while a run is active (`finished(id)` clears). UTC unix seconds; service host owns persistence, clock, and process launch. Local-only, no network.

## `crates/vt` (`loos-vt`, task #161)
`parser::Parser` – incremental ECMA-48/xterm state machine with UTF-8 (CSI, ESC, OSC, DCS ignored; CAN/SUB abort). `screen::Screen` – cell grid: cursor moves, ED/EL, IL/DL, SU/SD, wrap, scrollback (1000), SGR (16/256/RGB, bold/underline/reverse), save/restore cursor, `?25` cursor visibility, OSC 0/2 title. Not yet: alt screen, scroll regions, mouse modes, DEC graphics, input-side key encoding, wide-char width.

Verify: `cargo +1.97.1 test -p loos-tasksched -p loos-vt` (from `crates/`).

## `crates/timesvc` (`loos-timesvc`, task #212)
SNTPv4 request/reply with origin-timestamp echo check (anti-spoof), kiss-o'-death/unsynchronized rejection, offset/delay math, best-of-N selection, discipline policy (step >128 ms, ignore <1 ms, else slew ≤500 ppm), poll backoff 64→1024 s. POSIX-TZ strings (`CET-1CEST,M3.5.0,M10.5.0/3`, quoted names, southern hemisphere) and a UTC calendar. No NTP servers are built in (privacy: admin configures them). Not done: tz database (zoneinfo) files, leap-second smear, RTC driver, the socket service. Test: `cargo +1.97.1 test -p loos-timesvc`.
