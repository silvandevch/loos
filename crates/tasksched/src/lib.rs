//! Task Scheduler core (task #165): a deterministic state machine. The service host feeds it
//! the wall clock (UTC unix seconds) and launches whatever [`Scheduler::poll`] returns.
//! Persistence is the caller's job (tasks are plain data). No telemetry, no network.
#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// Poll slack: fire times later than this are "missed".
pub const GRACE: i64 = 60;

/// Seconds per day.
pub const DAY: i64 = 86_400;

/// Days since 1970-01-01 for a civil date (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date `(year, month, day)` for days since the epoch.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

/// 0 = Sunday.
fn weekday(days: i64) -> u32 {
    (days + 4).rem_euclid(7) as u32
}

/// When a task fires.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Once at a unix time.
    Once(i64),
    /// Every `every_days` days at `secs_of_day` UTC, starting from day `start_day` (days since epoch).
    Daily { start_day: i64, every_days: u32, secs_of_day: u32 },
    /// On the weekdays in `mask` (bit 0 = Sunday) at `secs_of_day`.
    Weekly { mask: u8, secs_of_day: u32 },
    /// On day-of-month `day` (clamped to month length) at `secs_of_day`.
    Monthly { day: u8, secs_of_day: u32 },
    /// Every `secs` seconds anchored at `anchor`.
    Interval { anchor: i64, secs: u32 },
    /// At system start (fired by [`Scheduler::notify_boot`]).
    Boot,
    /// At user logon (fired by [`Scheduler::notify_logon`]).
    Logon,
}

impl Trigger {
    /// First fire time strictly after `t`, or `None` for event triggers / spent one-shots.
    pub fn next_after(&self, t: i64) -> Option<i64> {
        match *self {
            Trigger::Once(at) => (at > t).then_some(at),
            Trigger::Daily { start_day, every_days, secs_of_day } => {
                let n = every_days.max(1) as i64;
                let mut d = t.div_euclid(DAY);
                if d < start_day {
                    d = start_day;
                }
                // Align to the recurrence, then step until strictly after t.
                d += (n - (d - start_day).rem_euclid(n)) % n;
                loop {
                    let at = d * DAY + secs_of_day as i64;
                    if at > t {
                        return Some(at);
                    }
                    d += n;
                }
            }
            Trigger::Weekly { mask, secs_of_day } => {
                if mask & 0x7f == 0 {
                    return None;
                }
                let d0 = t.div_euclid(DAY);
                (d0..d0 + 8).map(|d| (d, d * DAY + secs_of_day as i64)).find(|&(d, at)| at > t && mask >> weekday(d) & 1 == 1).map(|x| x.1)
            }
            Trigger::Monthly { day, secs_of_day } => {
                let (mut y, mut m, _) = civil_from_days(t.div_euclid(DAY));
                for _ in 0..14 {
                    let dd = (day.max(1) as u32).min(days_in_month(y, m));
                    let at = days_from_civil(y, m, dd) * DAY + secs_of_day as i64;
                    if at > t {
                        return Some(at);
                    }
                    if m == 12 {
                        m = 1;
                        y += 1;
                    } else {
                        m += 1;
                    }
                }
                None
            }
            Trigger::Interval { anchor, secs } => {
                let s = secs.max(1) as i64;
                if t < anchor {
                    return Some(anchor);
                }
                Some(anchor + ((t - anchor) / s + 1) * s)
            }
            Trigger::Boot | Trigger::Logon => None,
        }
    }
}

/// What to do about fire times missed while the machine was off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissedPolicy {
    /// Skip them.
    Skip,
    /// Run once on the next poll.
    RunOnce,
}

/// Task id.
pub type TaskId = u32;

/// A scheduled task.
#[derive(Clone, Debug)]
pub struct Task {
    /// Id (assigned by [`Scheduler::add`]).
    pub id: TaskId,
    /// Display name (unique).
    pub name: String,
    /// Command line the service host launches.
    pub command: String,
    /// Triggers; any one firing runs the task.
    pub triggers: Vec<Trigger>,
    /// Enabled flag.
    pub enabled: bool,
    /// Missed-run policy.
    pub missed: MissedPolicy,
    /// Last time it was started.
    pub last_run: Option<i64>,
    /// Next scheduled time (cached).
    pub next_run: Option<i64>,
    /// Do not start if still running.
    pub no_overlap: bool,
    running: bool,
    pending_event: bool,
}

/// Scheduler errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Name already used.
    Exists,
    /// No such task.
    NotFound,
}

/// The scheduler.
#[derive(Default)]
pub struct Scheduler {
    tasks: Vec<Task>,
    next_id: TaskId,
}

impl Scheduler {
    /// Empty scheduler.
    pub fn new() -> Self {
        Self { tasks: Vec::new(), next_id: 1 }
    }

    /// Add a task as of time `now`.
    pub fn add(&mut self, name: &str, command: &str, triggers: Vec<Trigger>, missed: MissedPolicy, now: i64) -> Result<TaskId, Error> {
        if self.tasks.iter().any(|t| t.name == name) {
            return Err(Error::Exists);
        }
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        let mut t = Task {
            id,
            name: name.into(),
            command: command.into(),
            triggers,
            enabled: true,
            missed,
            last_run: None,
            next_run: None,
            no_overlap: true,
            running: false,
            pending_event: false,
        };
        t.next_run = Self::compute_next(&t, now);
        self.tasks.push(t);
        Ok(id)
    }

    fn compute_next(t: &Task, from: i64) -> Option<i64> {
        t.triggers.iter().filter_map(|g| g.next_after(from)).min()
    }

    /// Remove a task.
    pub fn remove(&mut self, id: TaskId) -> Result<(), Error> {
        let i = self.tasks.iter().position(|t| t.id == id).ok_or(Error::NotFound)?;
        self.tasks.remove(i);
        Ok(())
    }

    /// Enable/disable; enabling recomputes the next run from `now`.
    pub fn set_enabled(&mut self, id: TaskId, on: bool, now: i64) -> Result<(), Error> {
        let t = self.tasks.iter_mut().find(|t| t.id == id).ok_or(Error::NotFound)?;
        t.enabled = on;
        t.next_run = if on { Self::compute_next(t, now) } else { None };
        Ok(())
    }

    /// Look a task up.
    pub fn get(&self, id: TaskId) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    /// All tasks.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    fn event(&mut self, f: impl Fn(&Trigger) -> bool) {
        for t in self.tasks.iter_mut().filter(|t| t.enabled) {
            if t.triggers.iter().any(&f) {
                t.pending_event = true;
            }
        }
    }

    /// Fire `Boot` triggers on the next poll.
    pub fn notify_boot(&mut self) {
        self.event(|g| *g == Trigger::Boot);
    }

    /// Fire `Logon` triggers on the next poll.
    pub fn notify_logon(&mut self) {
        self.event(|g| *g == Trigger::Logon);
    }

    /// Report that a started task finished.
    pub fn finished(&mut self, id: TaskId) {
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
            t.running = false;
        }
    }

    /// Return tasks to start at `now` and advance their schedules. A fire time older than
    /// [`GRACE`] seconds counts as missed (machine was off/suspended) and is handled per
    /// [`MissedPolicy`].
    pub fn poll(&mut self, now: i64) -> Vec<TaskId> {
        let mut out = Vec::new();
        for t in self.tasks.iter_mut().filter(|t| t.enabled) {
            let mut fire = core::mem::take(&mut t.pending_event);
            if let Some(n) = t.next_run {
                if n <= now {
                    fire |= now - n <= GRACE || t.missed == MissedPolicy::RunOnce;
                    t.next_run = Self::compute_next(t, now);
                }
            }
            if fire {
                if t.no_overlap && t.running {
                    continue;
                }
                t.running = true;
                t.last_run = Some(now);
                out.push(t.id);
            }
        }
        out
    }
}
