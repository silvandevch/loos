//! System Command Center: activity timeline + search, workspace checkpoints, global (settings) undo, health scorecard
//! and time-based rules. The state and the app UI live here; the [`crate::desktop::Desktop`] feeds it facts and
//! executes the [`Cmd`]s it queues (so the UI never touches windows or the config store mid-frame).
#![allow(missing_docs)]

use crate::backend::{cfg_bool, cfg_int, cfg_set_bool, cfg_set_int, ShellBackend};
use crate::draw::Rect;
use crate::settings::pers_slider;
use crate::ui::{self, UiCtx};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;

pub const LOG_MAX: usize = 200;
pub const UNDO_MAX: usize = 24;
pub const WS_MAX: usize = 16;
pub const WS_FILE: &str = "/home/user/.config/loos/workspaces.txt";
pub const WS_DIR: &str = "/home/user/.config/loos";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    App,
    Setting,
    Workspace,
    Rule,
    Undo,
}
impl Kind {
    fn tag(self) -> &'static [u8] {
        match self {
            Kind::App => b"APP",
            Kind::Setting => b"SET",
            Kind::Workspace => b"WSP",
            Kind::Rule => b"RUL",
            Kind::Undo => b"UND",
        }
    }
}

pub struct LogEntry {
    pub t: u32,
    pub kind: Kind,
    pub text: String,
}

/// One window of a checkpoint. `slot` is the Desktop app slot, so restoring re-opens the right app.
#[derive(Clone, PartialEq, Debug)]
pub struct WinRec {
    pub slot: u8,
    pub rect: Rect,
    pub max: bool,
    pub min: bool,
    pub desk: i32,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Checkpoint {
    pub name: String,
    pub look: String,
    pub tiling: bool,
    pub desk: i32,
    pub wins: Vec<WinRec>,
}

/// Values of every tracked `ui.*` / `wm.*` key at one moment.
#[derive(Clone)]
pub struct Snapshot {
    pub vals: Vec<(Vec<u8>, Vec<u8>)>,
}

pub enum Cmd {
    Undo,
    SaveWorkspace(String),
    RestoreWorkspace(usize),
    DeleteWorkspace(usize),
    Action(&'static [u8]),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Good,
    Note,
    Warn,
}

pub struct Health {
    pub score: i32,
    pub rows: Vec<(&'static str, String, Level)>,
}

pub struct Center {
    pub be: *const ShellBackend,
    pub log: Vec<LogEntry>,
    pub ws: Vec<Checkpoint>,
    pub undo: Vec<(u32, Snapshot)>,
    pub shadow: Option<Snapshot>,
    pub health: Health,
    pub cmds: Vec<Cmd>,
    keys: Vec<Vec<u8>>,
    pub last_night: i8,
    pub last_look: i8,
    // UI state
    tab: i32,
    name: [u8; 32],
    name_caret: i32,
    filter: [u8; 32],
    filter_caret: i32,
    scroll: i32,
}

/// (key, default when absent). Keys without a default are only tracked when present.
const TRACKED: [(&str, Option<&str>); 29] = [
    ("ui.base", Some("dark")), ("ui.accent_hex", None), ("ui.tint", Some("0")), ("ui.radius", Some("-1")),
    ("ui.title_h", Some("0")), ("ui.border", Some("0")), ("ui.shadow", Some("100")), ("ui.glow", Some("100")),
    ("ui.opacity", Some("92")), ("ui.icons", Some("0")), ("ui.wallpaper", Some("0")), ("ui.guard", Some("1")),
    ("ui.scale", None), ("ui.reduce_motion", Some("0")), ("ui.night", Some("0")), ("ui.look", None),
    ("ui.taskbar.center", Some("1")), ("ui.taskbar.labels", Some("0")), ("ui.taskbar.top", Some("0")),
    ("ui.taskbar.size", Some("1")), ("wm.tiling", Some("0")), ("wm.gap", Some("6")), ("wm.focus", Some("0")),
    ("wm.master", Some("55")), ("rule.night", Some("0")), ("rule.night.level", Some("50")), ("rule.look", Some("0")),
    ("ui.theme", None), ("ui.accent", None),
];

fn nz(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}
fn until_nul(a: &[u8]) -> &[u8] {
    &a[..a.iter().position(|&b| b == 0).unwrap_or(a.len())]
}

impl Center {
    pub fn new(be: *const ShellBackend) -> Self {
        let mut keys: Vec<Vec<u8>> = TRACKED.iter().map(|(k, _)| nz(k)).collect();
        for t in crate::theme::TOKEN_NAMES.iter() {
            keys.push(crate::theme::key_for(b"ui.c.", t).to_vec());
        }
        Self {
            be,
            log: Vec::new(),
            ws: Vec::new(),
            undo: Vec::new(),
            shadow: None,
            health: Health { score: 100, rows: Vec::new() },
            cmds: Vec::new(),
            keys,
            last_night: -1,
            last_look: -1,
            tab: 0,
            name: [0; 32],
            name_caret: 0,
            filter: [0; 32],
            filter_caret: 0,
            scroll: 0,
        }
    }

    fn be(&self) -> &ShellBackend {
        // SAFETY: the backend lives in the desktop that owns this Center.
        unsafe { &*self.be }
    }

    // ------------------------------------------------------------ activity
    /// Appends to the timeline. A repeat of the same text within a second (a slider drag) only refreshes the entry.
    pub fn note(&mut self, now: u32, kind: Kind, text: &str) {
        if let Some(l) = self.log.last_mut() {
            if l.kind == kind && l.text == text && now.wrapping_sub(l.t) < 1000 {
                l.t = now;
                return;
            }
        }
        if self.log.len() >= LOG_MAX {
            self.log.remove(0);
        }
        self.log.push(LogEntry { t: now, kind, text: text.to_string() });
    }

    /// Activity search: newest first, case-insensitive substring over kind tag and text.
    pub fn search(&self, q: &str) -> Vec<&LogEntry> {
        let q = q.to_ascii_lowercase();
        self.log
            .iter()
            .rev()
            .filter(|e| q.is_empty() || e.text.to_ascii_lowercase().contains(&q) || e.kind.tag().eq_ignore_ascii_case(q.as_bytes()))
            .collect()
    }

    // ------------------------------------------------------------ undo
    pub fn snapshot(&self) -> Snapshot {
        let be = self.be();
        let mut vals = Vec::new();
        for (i, k) in self.keys.iter().enumerate() {
            let mut v = [0u8; 96];
            if be.cfg_get(k, &mut v) {
                vals.push((k.clone(), until_nul(&v).to_vec()));
            } else if let Some(Some(d)) = TRACKED.get(i).map(|t| t.1) {
                vals.push((k.clone(), d.as_bytes().to_vec()));
            }
        }
        Snapshot { vals }
    }

    pub fn restore(&self, s: &Snapshot) {
        let be = self.be();
        for (k, v) in &s.vals {
            let mut z = v.clone();
            z.push(0);
            be.cfg_set(k, &z);
        }
    }

    /// Called when the config changed: the state *before* the change goes on the undo stack. Rapid edits of one
    /// control (slider drags) coalesce into a single undo step.
    pub fn changed(&mut self, now: u32, undoing: bool) {
        let cur = self.snapshot();
        if !undoing {
            if let Some(prev) = self.shadow.take() {
                let fresh = self.undo.last().map_or(true, |(t, _)| now.wrapping_sub(*t) > 800);
                if fresh {
                    if self.undo.len() >= UNDO_MAX {
                        self.undo.remove(0);
                    }
                    self.undo.push((now, prev));
                } else if let Some(l) = self.undo.last_mut() {
                    l.0 = now;
                }
            }
        }
        self.shadow = Some(cur);
    }

    pub fn pop_undo(&mut self) -> Option<Snapshot> {
        self.undo.pop().map(|(_, s)| s)
    }

    // ------------------------------------------------------------ workspaces
    pub fn serialize_ws(&self) -> String {
        let mut o = String::new();
        for c in &self.ws {
            o += &format!("{}|{}|{}|{}|", c.name.replace(['|', '\n'], " "), c.look, c.tiling as i32, c.desk);
            for w in &c.wins {
                o += &format!("{},{},{},{},{},{},{},{};", w.slot, w.rect.x, w.rect.y, w.rect.w, w.rect.h, w.max as i32, w.min as i32, w.desk);
            }
            o.push('\n');
        }
        o
    }

    /// Parses [`serialize_ws`]; malformed lines are skipped. Returns the number of checkpoints read.
    pub fn load_ws(&mut self, text: &str) -> usize {
        self.ws.clear();
        for line in text.lines() {
            let f: Vec<&str> = line.split('|').collect();
            if f.len() < 5 || f[0].is_empty() || self.ws.len() >= WS_MAX {
                continue;
            }
            let mut wins = Vec::new();
            for rec in f[4].split(';').filter(|r| !r.is_empty()) {
                let n: Vec<i32> = rec.split(',').filter_map(|x| x.parse().ok()).collect();
                if n.len() == 8 && (0..16).contains(&n[0]) {
                    wins.push(WinRec { slot: n[0] as u8, rect: Rect::new(n[1], n[2], n[3], n[4]), max: n[5] != 0, min: n[6] != 0, desk: n[7] });
                }
            }
            self.ws.push(Checkpoint {
                name: f[0].to_string(),
                look: f[1].to_string(),
                tiling: f[2] == "1",
                desk: f[3].parse().unwrap_or(0),
                wins,
            });
        }
        self.ws.len()
    }

    // ------------------------------------------------------------ health
    /// Recomputes the scorecard from live facts. All inputs are local; nothing is measured over the network.
    pub fn refresh_health(&mut self, mem_mb: u32, procs: i32, conns: i32, rules: i32, grants: i32, net_off: bool, updates_on: bool) {
        let mut rows = Vec::new();
        let mut score = 100;
        rows.push(("Telemetry", "none, by design".to_string(), Level::Good));
        if net_off {
            rows.push(("Network", "hard switch ON: nothing can connect".to_string(), Level::Good));
        } else if conns > 0 {
            score -= 5;
            rows.push(("Network", format!("{conns} open connection(s)"), Level::Note));
        } else {
            rows.push(("Network", "idle".to_string(), Level::Good));
        }
        if rules == 0 && !net_off {
            score -= 10;
            rows.push(("Firewall", "no rules: every app will ask".to_string(), Level::Note));
        } else {
            rows.push(("Firewall", format!("{rules} rule(s)"), Level::Good));
        }
        if grants > 0 {
            score -= (grants * 3).min(15);
            rows.push(("Permissions", format!("{grants} grant(s) to review"), Level::Note));
        } else {
            rows.push(("Permissions", "nothing granted".to_string(), Level::Good));
        }
        if updates_on {
            score -= 5;
            rows.push(("Updates", "automatic checks on".to_string(), Level::Note));
        } else {
            rows.push(("Updates", "opt-in (off)".to_string(), Level::Good));
        }
        let mem_level = if mem_mb > 900 { Level::Warn } else { Level::Good };
        if mem_level == Level::Warn {
            score -= 15;
        }
        rows.push(("Memory", format!("{mem_mb} MB across {procs} processes"), mem_level));
        self.health = Health { score: score.clamp(0, 100), rows };
    }

    // ------------------------------------------------------------ rules
    /// Evaluates the time rules for `hour`. Returns the (key, value) pairs that must be written; transitions only,
    /// so a manual change between hours is never fought.
    pub fn eval_rules(&mut self, hour: i32) -> Vec<(&'static str, String, String)> {
        let (mut last_night, mut last_look) = (self.last_night, self.last_look);
        let be = self.be();
        let mut out = Vec::new();
        let inside = |h: i32, from: i32, to: i32| if from <= to { h >= from && h < to } else { h >= from || h < to };
        if cfg_bool(be, b"rule.night\0", false) {
            let on = inside(hour, cfg_int(be, b"rule.night.from\0", 20), cfg_int(be, b"rule.night.to\0", 7)) as i8;
            if on != last_night {
                last_night = on;
                let lvl = cfg_int(be, b"rule.night.level\0", 50).clamp(0, 100);
                out.push(("ui.night", (if on == 1 { lvl } else { 0 }).to_string(), format!("Night light {}", if on == 1 { "on" } else { "off" })));
            }
        } else {
            last_night = -1;
        }
        if cfg_bool(be, b"rule.look\0", false) {
            let eve = inside(hour, cfg_int(be, b"rule.look.from\0", 19), cfg_int(be, b"rule.look.to\0", 7)) as i8;
            if eve != last_look {
                last_look = eve;
                let mut v = [0u8; 24];
                let key: &[u8] = if eve == 1 { b"rule.look.eve\0" } else { b"rule.look.day\0" };
                let def = if eve == 1 { "midnight" } else { "loos" };
                let id = if be.cfg_get(key, &mut v) { core::str::from_utf8(until_nul(&v)).unwrap_or(def).to_string() } else { def.to_string() };
                out.push(("look", id.clone(), format!("Look {id} (by time)")));
            }
        } else {
            last_look = -1;
        }
        self.last_night = last_night;
        self.last_look = last_look;
        out
    }

    // ------------------------------------------------------------ UI
    pub fn frame(&mut self, u: &mut UiCtx, w: i32, h: i32) {
        let t = unsafe { *u.theme };
        ui::clear(u, t.surface);
        const TABS: [&[u8]; 4] = [b"Overview", b"Workspaces", b"History", b"Rules"];
        for (i, n) in TABS.iter().enumerate() {
            let r = Rect::new(16 + i as i32 * 116, 14, 108, 30);
            let hit = if self.tab == i as i32 { ui::button_accent(u, r, n) } else { ui::button(u, r, n) };
            if hit {
                self.tab = i as i32;
                self.scroll = 0;
            }
        }
        let c = Rect::new(20, 60, w - 40, h - 76);
        let saved = ui::clip_push(u, c);
        match self.tab {
            0 => self.page_overview(u, c),
            1 => self.page_workspaces(u, c),
            2 => self.page_history(u, c),
            _ => self.page_rules(u, c),
        }
        ui::clip_pop(u, saved);
    }

    fn page_overview(&mut self, u: &mut UiCtx, c: Rect) {
        let t = unsafe { *u.theme };
        let sc = format!("{}", self.health.score);
        ui::heading(u, c.x, c.y, b"System health");
        ui::label(u, c.x, c.y + 34, sc.as_bytes(), if self.health.score >= 85 { t.accent } else { t.danger });
        ui::label(u, c.x + 40, c.y + 34, b"/ 100  (local checks only)", t.text_dim);
        for (i, (name, text, lvl)) in self.health.rows.iter().enumerate() {
            let y = c.y + 64 + i as i32 * 22;
            let col = match lvl {
                Level::Good => t.accent,
                Level::Note => t.text,
                Level::Warn => t.danger,
            };
            ui::label(u, c.x, y, name.as_bytes(), t.text_dim);
            ui::label(u, c.x + 130, y, text.as_bytes(), col);
        }
        let by = c.y + 64 + self.health.rows.len() as i32 * 22 + 18;
        ui::label(u, c.x, by, b"Quick actions", t.text_dim);
        let acts: [(&[u8], &'static [u8]); 5] = [
            (b"Night light", b"act:night"),
            (b"Tiling", b"act:tiling"),
            (b"Network off", b"act:network"),
            (b"Reduce motion", b"act:motion"),
            (b"Focus follows mouse", b"act:focus"),
        ];
        let mut x = c.x;
        for (label, act) in acts {
            let wdt = 28 + label.len() as i32 * 7;
            if ui::button(u, Rect::new(x, by + 22, wdt, 28), label) {
                self.cmds.push(Cmd::Action(act));
            }
            x += wdt + 8;
        }
        let undo = format!("Undo last change ({})", self.undo.len());
        if ui::button(u, Rect::new(c.x, by + 62, 220, 28), undo.as_bytes()) {
            self.cmds.push(Cmd::Undo);
        }
    }

    fn page_workspaces(&mut self, u: &mut UiCtx, c: Rect) {
        let t = unsafe { *u.theme };
        ui::heading(u, c.x, c.y, b"Workspace checkpoints");
        ui::label(u, c.x, c.y + 30, b"Save the open apps, their positions, the desktop, tiling and the Look; restore it any time.", t.text_dim);
        let tf = ui::textfield(u, Rect::new(c.x, c.y + 56, 260, 28), &mut self.name, &mut self.name_caret, b"Name, e.g. Writing");
        let n = until_nul(&self.name);
        let name = core::str::from_utf8(n).unwrap_or("").trim().to_string();
        if (ui::button_accent(u, Rect::new(c.x + 272, c.y + 56, 160, 28), b"Save current layout") || tf) && !name.is_empty() {
            self.cmds.push(Cmd::SaveWorkspace(name));
            self.name = [0; 32];
            self.name_caret = 0;
        }
        if self.ws.is_empty() {
            ui::label(u, c.x, c.y + 104, b"No checkpoints yet.", t.text_dim);
        }
        for i in 0..self.ws.len() {
            let y = c.y + 100 + i as i32 * 40;
            let r = Rect::new(c.x, y, c.w, 34);
            unsafe { crate::ffi::fb_fill_round(u.fb, u.clip, r, 8, crate::draw::mix(t.surface, t.surface_hi, 140)) };
            let w = &self.ws[i];
            ui::label(u, r.x + 12, y + 9, w.name.as_bytes(), t.text);
            let sub = format!("{} window(s), {}{}", w.wins.len(), w.look, if w.tiling { ", tiled" } else { "" });
            ui::label(u, r.x + 190, y + 9, sub.as_bytes(), t.text_dim);
            if ui::button(u, Rect::new(r.x + r.w - 170, y + 3, 80, 28), b"Restore") {
                self.cmds.push(Cmd::RestoreWorkspace(i));
            }
            if ui::button(u, Rect::new(r.x + r.w - 84, y + 3, 76, 28), b"Delete") {
                self.cmds.push(Cmd::DeleteWorkspace(i));
            }
        }
    }

    fn page_history(&mut self, u: &mut UiCtx, c: Rect) {
        let t = unsafe { *u.theme };
        ui::heading(u, c.x, c.y, b"Activity");
        ui::textfield(u, Rect::new(c.x, c.y + 34, 300, 28), &mut self.filter, &mut self.filter_caret, b"Search activity (apps, settings, workspaces...)");
        let undo = format!("Undo last change ({})", self.undo.len());
        if ui::button(u, Rect::new(c.x + c.w - 220, c.y + 34, 220, 28), undo.as_bytes()) {
            self.cmds.push(Cmd::Undo);
        }
        let q = core::str::from_utf8(until_nul(&self.filter)).unwrap_or("").to_string();
        let mut scroll = self.scroll;
        let rows = self.search(&q);
        if rows.is_empty() {
            ui::label(u, c.x, c.y + 78, b"Nothing matches.", t.text_dim);
        }
        let visible = ((c.h - 76) / 22).max(1) as usize;
        let mut px = scroll * 22;
        ui::vscroll(u, b"center_scroll", Rect::new(c.x, c.y + 76, c.w, visible as i32 * 22), rows.len() as i32 * 22, &mut px);
        scroll = (px / 22).clamp(0, rows.len().saturating_sub(visible) as i32);
        for (i, e) in rows.iter().skip(scroll as usize).take(visible).enumerate() {
            let y = c.y + 76 + i as i32 * 22;
            let s = e.t / 1000;
            let when = format!("{:02}:{:02}", s / 60 % 100, s % 60);
            ui::label(u, c.x, y, when.as_bytes(), t.text_dim);
            ui::label(u, c.x + 56, y, e.kind.tag(), t.accent);
            ui::label(u, c.x + 104, y, e.text.as_bytes(), t.text);
        }
        self.scroll = scroll;
    }

    fn page_rules(&mut self, u: &mut UiCtx, c: Rect) {
        let t = unsafe { *u.theme };
        let be = unsafe { &*self.be };
        ui::heading(u, c.x, c.y, b"Rules");
        ui::label(u, c.x, c.y + 30, b"If it is this time of day, do this. Rules act when the hour is crossed, so manual changes stay yours.", t.text_dim);
        let half = (c.w - 16) / 2;
        let mut on = cfg_bool(be, b"rule.night\0", false);
        if ui::toggle(u, Rect::new(c.x, c.y + 62, half, 26), b"Night light by schedule", &mut on) {
            cfg_set_bool(be, b"rule.night\0", on);
            self.last_night = -1;
            self.cfg_changed();
        }
        for (i, (label, key, def, lo, hi)) in [
            (&b"From hour"[..], &b"rule.night.from\0"[..], 20, 0, 23),
            (b"Until hour", b"rule.night.to\0", 7, 0, 23),
            (b"Strength %", b"rule.night.level\0", 50, 0, 100),
        ]
        .into_iter()
        .enumerate()
        {
            let mut v = cfg_int(be, key, def);
            let col = c.x + (i as i32 % 2) * (half + 16);
            if pers_slider(u, col, c.y + 98 + (i as i32 / 2) * 44, half - 8, label, &mut v, lo, hi) {
                cfg_set_int(be, key, v);
                self.last_night = -1;
                self.cfg_changed();
            }
        }
        let y2 = c.y + 200;
        let mut on = cfg_bool(be, b"rule.look\0", false);
        if ui::toggle(u, Rect::new(c.x, y2, half, 26), b"Switch Look by time", &mut on) {
            cfg_set_bool(be, b"rule.look\0", on);
            self.last_look = -1;
            self.cfg_changed();
        }
        for (i, (label, key, def, lo, hi)) in [(&b"Evening starts"[..], &b"rule.look.from\0"[..], 19, 0, 23), (b"Day starts", b"rule.look.to\0", 7, 0, 23)].into_iter().enumerate() {
            let mut v = cfg_int(be, key, def);
            if pers_slider(u, c.x + i as i32 * (half + 16), y2 + 36, half - 8, label, &mut v, lo, hi) {
                cfg_set_int(be, key, v);
                self.last_look = -1;
                self.cfg_changed();
            }
        }
        for (i, (label, key, def)) in [(&b"Day look"[..], &b"rule.look.day\0"[..], "loos"), (b"Evening look", b"rule.look.eve\0", "midnight")].into_iter().enumerate() {
            let mut v = [0u8; 24];
            let cur = if be.cfg_get(key, &mut v) { core::str::from_utf8(until_nul(&v)).unwrap_or(def).to_string() } else { def.to_string() };
            let idx = crate::look::look_index(&cur).unwrap_or(0);
            let txt = format!("{}: {} (click to change)", core::str::from_utf8(label).unwrap_or(""), crate::look::LOOKS[idx].name);
            if ui::button(u, Rect::new(c.x + i as i32 * (half + 16), y2 + 100, half - 8, 28), txt.as_bytes()) {
                let next = crate::look::LOOKS[(idx + 1) % crate::look::LOOKS.len()].id;
                be.cfg_set(key, &nz(next));
                self.last_look = -1;
                self.cfg_changed();
            }
        }
    }

    fn cfg_changed(&mut self) {
        self.cmds.push(Cmd::Action(b"act:refresh"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_mem::MemBackend;
    use alloc::boxed::Box;
    use alloc::vec;

    fn setup() -> (Box<MemBackend>, ShellBackend) {
        let mut m = Box::new(MemBackend::new());
        let be = m.vtable();
        (m, be)
    }

    #[test]
    fn undo_restores_and_slider_drags_coalesce() {
        let (_m, be) = setup();
        let mut c = Center::new(&be);
        c.changed(0, false); // baseline
        cfg_set_int(&be, b"ui.radius\0", 4);
        c.changed(100, false);
        cfg_set_int(&be, b"ui.radius\0", 5);
        c.changed(300, false); // within 800 ms of the last step: coalesced
        assert_eq!(c.undo.len(), 1);
        cfg_set_int(&be, b"ui.radius\0", 9);
        c.changed(5000, false);
        assert_eq!(c.undo.len(), 2);
        let s = c.pop_undo().unwrap();
        c.restore(&s);
        assert_eq!(cfg_int(&be, b"ui.radius\0", 0), 5);
        let s = c.pop_undo().unwrap();
        c.restore(&s);
        assert_eq!(cfg_int(&be, b"ui.radius\0", 0), -1, "absent keys snapshot their default");
    }

    #[test]
    fn workspaces_round_trip_and_bad_lines_are_skipped() {
        let (_m, be) = setup();
        let mut c = Center::new(&be);
        c.ws.push(Checkpoint {
            name: "Writing | draft".into(),
            look: "paper".into(),
            tiling: true,
            desk: 2,
            wins: vec![WinRec { slot: 3, rect: Rect::new(10, 20, 640, 480), max: false, min: true, desk: 1 }],
        });
        let text = c.serialize_ws();
        let mut d = Center::new(&be);
        assert_eq!(d.load_ws(&format!("{text}garbage\n|x|0|0|\n")), 1);
        assert_eq!(d.ws[0].name, "Writing   draft");
        assert_eq!(d.ws[0].wins, c.ws[0].wins);
        assert!(d.ws[0].tiling && d.ws[0].desk == 2);
    }

    #[test]
    fn activity_search_and_coalescing() {
        let (_m, be) = setup();
        let mut c = Center::new(&be);
        c.note(0, Kind::App, "Opened Files");
        c.note(10, Kind::Setting, "Look Tiler");
        c.note(20, Kind::Setting, "Look Tiler");
        assert_eq!(c.log.len(), 2);
        assert_eq!(c.search("files").len(), 1);
        assert_eq!(c.search("set").len(), 1, "kind tag matches");
        assert_eq!(c.search("")[0].text, "Look Tiler", "newest first");
    }

    #[test]
    fn rules_fire_only_on_transitions() {
        let (_m, be) = setup();
        cfg_set_bool(&be, b"rule.night\0", true);
        cfg_set_int(&be, b"rule.night.level\0", 40);
        let mut c = Center::new(&be);
        let a = c.eval_rules(12);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].1, "0");
        assert!(c.eval_rules(13).is_empty());
        let b = c.eval_rules(21);
        assert_eq!((b[0].0, b[0].1.as_str()), ("ui.night", "40"));
        assert!(c.eval_rules(23).is_empty() && c.eval_rules(3).is_empty());
        assert_eq!(c.eval_rules(8)[0].1, "0");
        cfg_set_bool(&be, b"rule.look\0", true);
        let l = c.eval_rules(8);
        assert_eq!((l[0].0, l[0].1.as_str()), ("look", "loos"));
        assert!(c.eval_rules(20).iter().any(|r| r.0 == "look" && r.1 == "midnight"));
    }

    #[test]
    fn health_penalises_grants_and_open_network() {
        let (_m, be) = setup();
        let mut c = Center::new(&be);
        c.refresh_health(100, 4, 0, 0, 0, true, false);
        assert_eq!(c.health.score, 100);
        c.refresh_health(100, 4, 3, 0, 5, false, true);
        assert!(c.health.score < 70 && c.health.score > 0);
    }
}
