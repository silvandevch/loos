//! LoOS Switch UI. The visual flow is deliberately short: choose a safe
//! target, review exactly what will happen, then one cancellable operation.

use crate::install::{self, InstallEvent, InstallOutcome};
use crate::migration::{self, CopyProgress};
use crate::model::{
    valid_hostname, valid_login_name, ConflictPolicy, Disk, Firmware, InstallMethod, InstallPlan,
    MigrationManifest, MigrationOptions, MigrationPlan, MigrationSource, OsFamily, SystemInfo,
};
use crate::platform::{self, human_bytes, utc_now_string, APP_VERSION};
use crate::theme;
use eframe::egui::{self, Color32, FontId, Frame, Margin, RichText, ScrollArea, Stroke, Vec2};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Install,
    Migration,
    Safety,
    Logs,
    About,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InstallStage {
    Choose,
    Review,
    Running,
    Finished,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MigrationStage {
    Choose,
    Running,
    Finished,
}



enum JobMsg {
    Refreshed(Result<(SystemInfo, Vec<Disk>), String>),
    SourceMeasured(usize, u64, u64),
    MeasureDone,
    Install(InstallEvent),
    InstallDone(Result<InstallOutcome, String>),
    Migrate(CopyProgress),
    MigrateDone(Result<MigrationManifest, String>),
}

struct Job {
    rx: Receiver<JobMsg>,
    cancel: Arc<AtomicBool>,
}

struct FolderPicker {
    path: PathBuf,
    title: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PickerMode {
    Destination,
    Source,
}

pub struct LoosSwitchApp {
    screenshot: Option<Capture>,
    capture_frames: u32,
    page: Page,
    system: Option<SystemInfo>,
    disks: Vec<Disk>,
    load_error: Option<String>,
    loading: bool,

    job: Option<Job>,
    logs: Vec<String>,
    toast: Option<(String, Instant)>,

    install_stage: InstallStage,
    method: InstallMethod,
    firmware: Firmware,
    firmware_override: bool,
    selected_disk: Option<String>,
    plan: Option<InstallPlan>,
    boot_label: String,
    verify_readback: bool,
    reboot_when_done: bool,
    unattended: bool,
    admin_user: String,
    admin_password: String,
    computer_name: String,
    backup_ack: bool,
    phrase: String,
    install_progress: f32,
    install_phase: String,
    install_detail: String,
    install_done: Option<Result<InstallOutcome, String>>,
    cancel_requested: bool,

    migration_stage: MigrationStage,
    sources: Vec<MigrationSource>,
    destination: Option<PathBuf>,
    target_home: String,
    migration_options: MigrationOptions,
    migration_plan: Option<MigrationPlan>,
    migration_progress: CopyProgress,
    migration_done: Option<Result<MigrationManifest, String>>,
    migration_free: u64,
    direct_home: bool,
    folder_picker: Option<FolderPicker>,
    exclusions_text: String,
    picker_mode: PickerMode,

    expert: bool,
    last_refresh: Option<Instant>,
}

struct Capture {
    path: PathBuf,
    requested: bool,
}

impl LoosSwitchApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::new_inner(cc, None, None)
    }

    pub fn new_capture(cc: &eframe::CreationContext<'_>, path: PathBuf, page: Option<&str>) -> Self {
        let page = match page {
            Some("install") => Some(Page::Install),
            Some("migration") => Some(Page::Migration),
            Some("safety") => Some(Page::Safety),
            Some("logs") => Some(Page::Logs),
            Some("about") => Some(Page::About),
            _ => Some(Page::Home),
        };
        Self::new_inner(cc, Some(Capture { path, requested: false }), page)
    }

    fn new_inner(cc: &eframe::CreationContext<'_>, screenshot: Option<Capture>, page: Option<Page>) -> Self {
        theme::install(&cc.egui_ctx);
        let mut app = Self {
            screenshot,
            capture_frames: 0,
            page: page.unwrap_or(Page::Home),
            system: None,
            disks: Vec::new(),
            load_error: None,
            loading: true,
            job: None,
            logs: Vec::new(),
            toast: None,
            install_stage: InstallStage::Choose,
            method: InstallMethod::InstallMedia,
            firmware: Firmware::Uefi,
            firmware_override: false,
            selected_disk: None,
            plan: None,
            boot_label: "LoOS Setup".into(),
            verify_readback: true,
            reboot_when_done: false,
            unattended: false,
            admin_user: String::new(),
            admin_password: String::new(),
            computer_name: "loos-pc".into(),
            backup_ack: false,
            phrase: String::new(),
            install_progress: 0.0,
            install_phase: String::new(),
            install_detail: String::new(),
            install_done: None,
            cancel_requested: false,
            migration_stage: MigrationStage::Choose,
            sources: Vec::new(),
            destination: None,
            target_home: "user".into(),
            migration_options: MigrationOptions::default(),
            migration_plan: None,
            migration_progress: CopyProgress {
                files_done: 0,
                files_total: 0,
                bytes_done: 0,
                bytes_total: 0,
                current: String::new(),
                skipped: 0,
                errors: 0,
            },
            migration_done: None,
            migration_free: 0,
            direct_home: false,
            folder_picker: None,
            exclusions_text: "node_modules, __pycache__, .cache, Cache, GPUCache, Crashpad, Temp, $Recycle.Bin, System Volume Information".into(),
            picker_mode: PickerMode::Destination,
            expert: false,
            last_refresh: None,
        };
        app.log("LoOS Switch started. Local-only mode: no network access, no telemetry.");
        app.refresh();
        app
    }

    fn log(&mut self, line: impl Into<String>) {
        let line = line.into();
        self.logs.push(format!("{}  {}", short_time(), line));
        if self.logs.len() > 1200 {
            self.logs.drain(..200);
        }
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    fn refresh(&mut self) {
        if self.job.is_some() {
            return;
        }
        self.loading = true;
        self.load_error = None;
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        std::thread::spawn(move || {
            let r = (|| -> Result<(SystemInfo, Vec<Disk>), String> {
                let sys = platform::system_info().map_err(|e| e.to_string())?;
                let disks = platform::scan_disks().map_err(|e| e.to_string())?;
                Ok((sys, disks))
            })();
            let _ = tx.send(JobMsg::Refreshed(r));
        });
        self.log("Scanning this computer and its disks…");
        self.job = Some(Job { rx, cancel });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if self.job.is_none() {
            return;
        }
        let mut msgs = Vec::new();
        let mut disconnected = false;
        if let Some(job) = &self.job {
            loop {
                match job.rx.try_recv() {
                    Ok(msg) => msgs.push(msg),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        let mut done = disconnected;
        for msg in msgs {
            done |= self.handle_msg(msg);
        }
        if done {
            self.job = None;
        }
        if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    fn handle_msg(&mut self, msg: JobMsg) -> bool {
        match msg {
            JobMsg::Refreshed(Ok((sys, disks))) => {
                if self.admin_user.is_empty() {
                    self.admin_user = sanitise_login(&sys.user);
                    self.target_home = sanitise_login(&sys.user);
                    self.computer_name = sanitise_host(&sys.hostname);
                }
                self.firmware = sys.firmware.unwrap_or(Firmware::Uefi);
                self.system = Some(sys);
                self.disks = disks;
                self.loading = false;
                self.last_refresh = Some(Instant::now());
                self.ensure_default_sources();
                self.log(format!("Found {} disk(s).", self.disks.len()));
                true
            }
            JobMsg::Refreshed(Err(e)) => {
                self.load_error = Some(e.clone());
                self.loading = false;
                self.log(format!("Environment scan failed: {e}"));
                true
            }
            JobMsg::SourceMeasured(i, files, bytes) => {
                if let Some(s) = self.sources.get_mut(i) {
                    s.size_bytes = Some(bytes);
                    s.file_count = Some(files);
                }
                false
            }
            JobMsg::MeasureDone => {
                self.log("Folder size scan complete.");
                true
            }
            JobMsg::Install(ev) => {
                self.handle_install_event(ev);
                false
            }
            JobMsg::InstallDone(r) => {
                self.install_stage = InstallStage::Finished;
                match &r {
                    Ok(o) => self.log(format!("Deployment complete: {}", o.summary)),
                    Err(e) => self.log(format!("Deployment failed: {e}")),
                }
                self.install_done = Some(r);
                self.cancel_requested = false;
                true
            }
            JobMsg::Migrate(p) => {
                self.migration_progress = p;
                false
            }
            JobMsg::MigrateDone(r) => {
                self.migration_stage = MigrationStage::Finished;
                match &r {
                    Ok(m) => self.log(format!(
                        "Migration complete: {} files, {}.",
                        m.stats.files_copied,
                        human_bytes(m.stats.bytes_copied)
                    )),
                    Err(e) => self.log(format!("Migration failed: {e}")),
                }
                self.migration_done = Some(r);
                self.cancel_requested = false;
                true
            }
        }
    }

    fn handle_install_event(&mut self, ev: InstallEvent) {
        match ev {
            InstallEvent::Progress { phase, done, total, detail } => {
                self.install_phase = phase.into();
                self.install_detail = detail.clone();
                self.install_progress = if total == 0 { 0.0 } else { (done as f64 / total as f64) as f32 };
                if done == 0 || self.logs.last().map(|l| !l.contains(phase)).unwrap_or(true) {
                    self.log(format!("{phase} — {detail}"));
                }
            }
            InstallEvent::Log(line) => self.log(line),
            InstallEvent::Finished { summary } => {
                self.install_phase = "Complete".into();
                self.install_progress = 1.0;
                self.log(summary);
            }
        }
    }

    fn ensure_default_sources(&mut self) {
        if !self.sources.is_empty() {
            return;
        }
        let Some(sys) = &self.system else { return };
        let home = sys.home.clone();
        let names: &[&str] = match sys.os {
            OsFamily::Windows => &["Desktop", "Documents", "Downloads", "Pictures", "Music", "Videos", "OneDrive", "Projects"],
            OsFamily::Linux => &["Desktop", "Documents", "Downloads", "Pictures", "Music", "Videos", "Projects"],
        };
        for name in names {
            let p = home.join(name);
            if p.is_dir() {
                self.sources.push(MigrationSource {
                    path: p,
                    destination: (*name).into(),
                    selected: matches!(*name, "Desktop" | "Documents" | "Downloads" | "Pictures"),
                    size_bytes: None,
                    file_count: None,
                });
            }
        }
        // A custom folder can be added for anything else. Hidden config is
        // deliberately not auto-selected: it is often machine-specific.
        self.destination = None;
        self.log(format!(
            "Discovered {} user folder(s). Sizes are measured only when you ask.",
            self.sources.len()
        ));
    }

    fn selected_disk(&self) -> Option<&Disk> {
        self.selected_disk.as_deref().and_then(|id| self.disks.iter().find(|d| d.id == id))
    }

    fn build_plan(&mut self) -> Result<InstallPlan, String> {
        let system = self.system.clone().ok_or("The environment scan has not finished.")?;
        let target = self.selected_disk().cloned().ok_or("Choose a target disk.")?;
        if target.system_disk {
            return Err("The selected disk hosts the running operating system. Choose another disk.".into());
        }
        #[cfg(target_os = "linux")]
        if self.method == InstallMethod::InstallMedia && target.mount_count() > 0 {
            return Err("Unmount the target disk's partitions first.".into());
        }
        let mut plan = InstallPlan::fresh(self.firmware, self.method);
        plan.target_disk = Some(target);
        plan.esp_mount = system.esp_mount.clone();
        plan.boot_label = self.boot_label.clone();
        plan.verify_readback = self.verify_readback;
        plan.reboot_when_done = self.reboot_when_done;
        plan.unattended = self.unattended && self.method == InstallMethod::LocalSetup;
        plan.admin_user = self.admin_user.clone();
        plan.admin_password = self.admin_password.clone();
        plan.computer_name = self.computer_name.clone();
        let blockers = plan.blockers(&system);
        if !blockers.is_empty() {
            return Err(blockers.join(" "));
        }
        Ok(plan)
    }

    fn go_review(&mut self) {
        self.phrase.clear();
        match self.build_plan() {
            Ok(p) => {
                self.plan = Some(p);
                self.install_stage = InstallStage::Review;
                self.log("Deployment plan created. Nothing has been written yet.");
            }
            Err(e) => {
                self.toast(e.clone());
                self.log(format!("Cannot continue: {e}"));
            }
        }
    }

    fn start_install(&mut self) {
        let Some(plan) = self.plan.clone() else { return };
        let Some(system) = self.system.clone() else { return };
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        self.install_stage = InstallStage::Running;
        self.install_progress = 0.0;
        self.install_done = None;
        self.cancel_requested = false;
        self.install_phase = "Preparing".into();
        self.install_detail = "Checking the plan one last time".into();
        self.log("Starting deployment. The cancel button stops new writes at the next block boundary.");
        std::thread::spawn(move || {
            let mut emit = |e| {
                let _ = tx.send(JobMsg::Install(e));
            };
            let result = install::execute(&plan, &system, &c2, &mut emit).map_err(|e| e.to_string());
            let _ = tx.send(JobMsg::InstallDone(result));
        });
        self.job = Some(Job { rx, cancel });
    }

    fn cancel_job(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::SeqCst);
            self.cancel_requested = true;
            self.log("Cancel requested. Waiting for the current block to finish…");
            self.toast("Cancelling safely at the next block boundary");
        }
    }

    fn scan_sources(&mut self) {
        if self.job.is_some() {
            self.toast("Another operation is already running.");
            return;
        }
        let opts = self.migration_options.clone();
        let sources = self.sources.clone();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        self.log("Measuring selected folders. No files are copied during a size scan.");
        std::thread::spawn(move || {
            for (i, s) in sources.iter().enumerate() {
                if c2.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok((files, bytes)) = migration::measure_source(&s.path, &opts, &c2, |_, _, _| {}) {
                    let _ = tx.send(JobMsg::SourceMeasured(i, files, bytes));
                }
            }
            let _ = tx.send(JobMsg::MeasureDone);
        });
        self.job = Some(Job { rx, cancel });
    }

    fn start_migration(&mut self) {
        if self.system.is_none() {
            return;
        }
        let selected: Vec<_> = self.sources.iter().filter(|s| s.selected).cloned().collect();
        if selected.is_empty() {
            self.toast("Select at least one folder.");
            return;
        }
        let Some(dest) = self.destination.clone() else {
            self.toast("Choose a destination drive/folder first.");
            return;
        };
        if !valid_login_name(&self.target_home) {
            self.toast("LoOS user name must start with a-z and use a-z, 0-9, '-' or '_'.");
            return;
        }
        if let Err(e) = std::fs::create_dir_all(&dest) {
            self.toast(format!("Cannot create destination: {e}"));
            return;
        }
        let total: u64 = selected.iter().map(|s| s.size_bytes.unwrap_or(0)).sum();
        let free = fs2::available_space(&dest).unwrap_or(0);
        if total > 0 && free > 0 && total > free {
            self.toast(format!("Not enough space: need {}, free {}.", human_bytes(total), human_bytes(free)));
            return;
        }
        let plan = MigrationPlan {
            sources: selected,
            destination_root: dest,
            target_home: self.target_home.clone(),
            options: self.migration_options.clone(),
            free_bytes: free,
            total_bytes: total,
            direct_to_home: self.direct_home,
        };
        self.migration_plan = Some(plan.clone());
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        self.migration_stage = MigrationStage::Running;
        self.migration_done = None;
        self.cancel_requested = false;
        self.migration_progress = CopyProgress {
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: total,
            current: "Preparing".into(),
            skipped: 0,
            errors: 0,
        };
        self.log("Starting copy-only migration. Sources are opened read-only and hashes are verified.");
        std::thread::spawn(move || {
            let result = migration::execute(&plan, &c2, |p| {
                let _ = tx.send(JobMsg::Migrate(p));
            })
            .map_err(|e| e.to_string());
            let _ = tx.send(JobMsg::MigrateDone(result));
        });
        self.job = Some(Job { rx, cancel });
    }

    fn save_log(&mut self) {
        let dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join("LoOS-Switch-Logs");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.toast(format!("Cannot create log folder: {e}"));
            return;
        }
        let path = dir.join(format!("loos-switch-{}.log", utc_now_string().replace(':', "-")));
        let text = self.logs.join("\n") + "\n";
        match std::fs::write(&path, text) {
            Ok(_) => {
                self.toast(format!("Log saved to {}", path.display()));
                self.log(format!("Diagnostics saved locally to {}", path.display()));
            }
            Err(e) => self.toast(format!("Could not save log: {e}")),
        }
    }

    // ---------------------------------------------------------------- chrome

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top")
            .exact_height(64.0)
            .frame(Frame::new().fill(theme::TOPBAR).inner_margin(Margin::symmetric(22, 0)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    theme::logo(ui, 30.0);
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("LoOS Switch").font(FontId::new(17.0, egui::FontFamily::Name("bold".into()))).color(theme::TEXT));
                        ui.label(theme::small("local-only installer & migration"));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        theme::pill(ui, "0% telemetry", theme::SUCCESS, theme::SUCCESS_DIM);
                        if let Some(sys) = &self.system {
                            let (txt, fg, bg) = if sys.elevated {
                                ("Administrator", theme::SUCCESS, theme::SUCCESS_DIM)
                            } else {
                                ("Standard user", theme::WARN, theme::WARN_DIM)
                            };
                            theme::pill(ui, txt, fg, bg);
                            theme::pill(ui, sys.firmware.map(|f| f.label()).unwrap_or("Firmware ?"), theme::MUTED, theme::CARD_ALT);
                        }
                        if self.loading {
                            ui.add(egui::Spinner::new().size(16.0));
                        }
                    });
                });
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("side")
            .exact_width(232.0)
            .frame(Frame::new().fill(theme::SIDEBAR).inner_margin(Margin::symmetric(14, 18)))
            .show(ctx, |ui| {
                ui.add_space(2.0);
                let navs = [
                    (Page::Home, "Start", "01"),
                    (Page::Install, "Install LoOS", "02"),
                    (Page::Migration, "Move my files", "03"),
                    (Page::Safety, "Safety & recovery", "04"),
                    (Page::Logs, "Activity log", "05"),
                    (Page::About, "About & privacy", "06"),
                ];
                for (p, label, number) in navs {
                    let active = self.page == p;
                    let fill = if active { theme::ACCENT_DIM } else { Color32::TRANSPARENT };
                    let fg = if active { theme::TEXT } else { theme::MUTED };
                    let text = format!("{number}   {label}");
                    let resp = ui.add_sized(
                        [ui.available_width(), 40.0],
                        egui::Button::new(RichText::new(text).font(FontId::new(14.5, egui::FontFamily::Name("medium".into()))).color(fg))
                            .fill(fill)
                            .stroke(Stroke::NONE)
                            .corner_radius(9),
                    );
                    if active {
                        let r = resp.rect;
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(egui::pos2(r.left(), r.top() + 9.0), egui::vec2(3.0, r.height() - 18.0)),
                            egui::CornerRadius::same(2),
                            theme::ACCENT_HOVER,
                        );
                    }
                    if resp.clicked() {
                        self.page = p;
                    }
                    ui.add_space(2.0);
                }
                ui.add_space(18.0);
                theme::divider(ui);
                ui.add_space(4.0);
                ui.label(theme::small("MODE"));
                ui.add_space(4.0);
                let mode_label = if self.expert { "Power user" } else { "Simple" };
                if ui
                    .add_sized(
                        [ui.available_width(), 34.0],
                        egui::Button::new(RichText::new(format!("MODE   {mode_label}"))
                            .font(FontId::new(13.5, egui::FontFamily::Name("medium".into())))
                            .color(theme::TEXT))
                            .fill(theme::CARD_ALT)
                            .stroke(Stroke::new(1.0_f32, theme::BORDER))
                            .corner_radius(8),
                    )
                    .on_hover_text("Simple hides the options most people never need. Power user shows exact disk, firmware and boot settings.")
                    .clicked()
                {
                    self.expert = !self.expert;
                    if !self.expert {
                        self.unattended = false;
                    }
                }
                ui.add_space(4.0);
                ui.label(theme::small(if self.expert {
                    "Exact targets and options are visible."
                } else {
                    "Safe defaults. Nothing hidden that writes."
                }));
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(theme::small(format!("v{APP_VERSION} · payload built {}", payload_date())));
                    ui.label(theme::small("no network · no telemetry"));
                    ui.add_space(6.0);
                    ui.label(theme::small("LoOS Switch"));
                });
            });
    }

    fn central(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(30, 24)))
            .show(ctx, |ui| {
                if let Some(e) = &self.load_error {
                    theme::danger_box(ui, "Could not read this computer's disks", e);
                    if ui.button("Retry scan").clicked() {
                        self.refresh();
                    }
                    return;
                }
                if self.system.is_none() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(120.0);
                        ui.add(egui::Spinner::new().size(34.0));
                        ui.add_space(14.0);
                        ui.label(theme::h2("Reading this computer…"));
                        ui.label(theme::muted("Enumerating disks and firmware. No network access."));
                    });
                    return;
                }
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_max_width(ui.available_width().min(1120.0));
                    match self.page {
                        Page::Home => self.page_home(ui),
                        Page::Install => self.page_install(ui),
                        Page::Migration => self.page_migration(ui),
                        Page::Safety => self.page_safety(ui),
                        Page::Logs => self.page_logs(ui),
                        Page::About => self.page_about(ui),
                    }
                    ui.add_space(36.0);
                });
            });
    }

    fn folder_picker_window(&mut self, ctx: &egui::Context) {
        let Some(mut picker) = self.folder_picker.take() else { return };
        let mut open = true;
        let mut window_visible = true;
        let mut chosen: Option<PathBuf> = None;
        egui::Window::new(picker.title.clone())
            .open(&mut window_visible)
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .show(ctx, |ui| {
                ui.label(theme::muted("Choose a folder. Nothing is copied until you start a migration."));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("↑ Up").clicked() {
                        if let Some(p) = picker.path.parent() {
                            picker.path = p.to_path_buf();
                        }
                    }
                    if ui.button("Home").clicked() {
                        if let Some(h) = dirs::home_dir() {
                            picker.path = h;
                        }
                    }
                    let mut path_text = picker.path.to_string_lossy().to_string();
                    if ui.add(egui::TextEdit::singleline(&mut path_text).desired_width(360.0).hint_text("Path")).changed() {
                        picker.path = PathBuf::from(path_text);
                    }
                });
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(theme::inset().fill)
                    .stroke(Stroke::new(1.0_f32, theme::BORDER))
                    .corner_radius(9)
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                            if let Ok(rd) = std::fs::read_dir(&picker.path) {
                                let mut dirs: Vec<_> = rd
                                    .filter_map(Result::ok)
                                    .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                                    .collect();
                                dirs.sort_by_key(|e| e.file_name());
                                for e in dirs {
                                    if ui.selectable_label(false, format!("📁  {}", e.file_name().to_string_lossy())).clicked() {
                                        picker.path = e.path();
                                    }
                                }
                            } else {
                                ui.label(theme::danger("This folder cannot be read."));
                            }
                        });
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button("Use this folder")).clicked() {
                        chosen = Some(picker.path.clone());
                    }
                    if ui.button("Cancel").clicked() {
                        open = false;
                    }
                    ui.label(theme::small(picker.path.display().to_string()));
                });
            });
        if let Some(p) = chosen {
            match self.picker_mode {
                PickerMode::Destination => {
                    self.destination = Some(p.clone());
                    self.migration_free = fs2::available_space(&p).unwrap_or(0);
                    self.log(format!("Migration destination set to {}", p.display()));
                }
                PickerMode::Source => {
                    let dest = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".into());
                    if !self.sources.iter().any(|s| s.path == p) {
                        self.sources.push(MigrationSource {
                            path: p.clone(),
                            destination: dest,
                            selected: true,
                            size_bytes: None,
                            file_count: None,
                        });
                        self.log(format!("Added migration source {}", p.display()));
                    }
                }
            }
            open = false;
        }
        if !window_visible {
            open = false;
        }
        if open {
            self.folder_picker = Some(picker);
        }
    }
}

// ---------------------------------------------------------------------- pages

impl LoosSwitchApp {
    fn page_home(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let total_w = ui.available_width().min(1120.0);
        ui.set_width(total_w);
        let wide = total_w > 900.0;
        if wide {
            let status_w = 360.0;
            let left_w = total_w - status_w - 24.0;
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(left_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(left_w);
                        self.hero(ui);
                    },
                );
                ui.add_space(20.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(status_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(status_w);
                        self.status_card(ui);
                    },
                );
            });
        } else {
            self.hero(ui);
            ui.add_space(16.0);
            self.status_card(ui);
        }
        ui.add_space(22.0);
        let gap = 16.0;
        let card_w = ((total_w - gap * 2.0) / 3.0).max(220.0);
        let mut clicked_page = None;
        ui.horizontal_top(|ui| {
            for (title, text, cta, page) in [
                (
                    "Your files come with you",
                    "Pick folders and copy them into a verified LoOS migration bundle. Copy-only: nothing on the source is ever moved or deleted.",
                    "Move my files",
                    Page::Migration,
                ),
                (
                    "A real installer, not a script",
                    "Write the official LoOS boot payload to a USB or dedicated disk. Every byte is read back and hashed before success.",
                    "Install LoOS",
                    Page::Install,
                ),
                (
                    "You can stop at any time",
                    "Cancel is checked before every 4 MiB block. The running system disk is never a target. No cloud, no account, no telemetry.",
                    "Read the safety rules",
                    Page::Safety,
                ),
            ] {
                ui.allocate_ui_with_layout(
                    Vec2::new(card_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(card_w);
                        if home_card(ui, title, text, cta).clicked() {
                            clicked_page = Some(page);
                        }
                    },
                );
                ui.add_space(gap);
            }
        });
        if let Some(p) = clicked_page {
            self.page = p;
        }
        ui.add_space(18.0);
        ui.label(theme::small("LoOS Switch is offline by construction. The binary contains no HTTP client, no analytics SDK and no update checker."));
    }

    fn hero(&mut self, ui: &mut egui::Ui) {
        ui.add_space(18.0);
        theme::pill(ui, "Local only. Finally.", theme::ACCENT_HOVER, theme::ACCENT_DIM);
        ui.add_space(12.0);
        ui.label(theme::title("Switch to LoOS\nwithout losing your stuff."));
        ui.add_space(12.0);
        ui.label(
            RichText::new("No terminal. No Microsoft account. No telemetry. A guided installer and a copy-only migration manager in one window.")
                .size(16.0)
                .color(theme::MUTED),
        );
        ui.add_space(22.0);
        if ui.add_sized([240.0, 48.0], theme::primary_button("Start setup →")).clicked() {
            self.page = Page::Install;
            self.install_stage = InstallStage::Choose;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let _ = ui.add_sized([210.0, 40.0], theme::secondary_button("Move my files first")).clicked().then(|| self.page = Page::Migration);
            if ui.add(theme::secondary_button("What gets erased?")).clicked() {
                self.page = Page::Safety;
            }
        });
        ui.add_space(18.0);
        ui.horizontal_wrapped(|ui| {
            theme::pill(ui, "works UEFI + BIOS", theme::MUTED, theme::CARD_ALT);
            theme::pill(ui, "Windows 10/11 · Linux", theme::MUTED, theme::CARD_ALT);
            theme::pill(ui, "cancel any time", theme::MUTED, theme::CARD_ALT);
        });
    }

    fn status_card(&mut self, ui: &mut egui::Ui) {
        theme::card().show(ui, |ui| {
            ui.set_max_width(320.0);
            ui.horizontal(|ui| {
                ui.label(theme::h2("This computer"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Refresh").clicked() {
                        self.refresh();
                    }
                });
            });
            ui.add_space(6.0);
            if let Some(sys) = self.system.clone() {
                kv(ui, "System", &format!("{} · {}", sys.os.label(), sys.os_name));
                kv(ui, "Firmware", sys.firmware.map(|f| f.label()).unwrap_or("unknown"));
                kv(ui, "Secure Boot", match sys.secure_boot {
                    Some(true) => "on · unsigned LoOS build needs it off",
                    Some(false) => "off",
                    None => "unknown / legacy",
                });
                kv(
                    ui,
                    "Permission",
                    if sys.elevated { "administrator" } else { "standard user (install needs elevation)" },
                );
                kv(ui, "ESP", &sys.esp_mount.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "not mounted / BIOS".into()));
                kv(ui, "Disks", &format!("{} visible · {} non-system", self.disks.len(), self.disks.iter().filter(|d| !d.system_disk).count()));
                if !sys.elevated {
                    ui.add_space(10.0);
                    if ui.add(theme::primary_button("Restart as administrator")).clicked() {
                        match platform::elevate_self() {
                            Ok(_) => {}
                            Err(e) => self.toast(e.to_string()),
                        }
                    }
                }
                if sys.secure_boot == Some(true) {
                    ui.add_space(8.0);
                    theme::warning_box(
                        ui,
                        "Secure Boot is on",
                        "This LoOS build is unsigned. Firmware will refuse the LoOS loader. Turn Secure Boot off in firmware settings (or use a signed build) before staging local setup.",
                    );
                }
                if sys.bitlocker_suspected {
                    ui.add_space(8.0);
                    theme::warning_box(ui, "BitLocker detected", "Suspend BitLocker before changing partitions or boot entries. This app never touches the running system disk, but firmware boot changes can still require a recovery key.");
                }
            }
        });
    }
}

fn home_card(ui: &mut egui::Ui, title: &str, text: &str, cta: &str) -> egui::Response {
    let resp = theme::card()
        .show(ui, |ui| {
            ui.set_min_height(150.0);
            ui.label(theme::h2(title));
            ui.add_space(6.0);
            ui.label(RichText::new(text).size(13.5).color(theme::MUTED));
            ui.add_space(12.0);
            ui.label(RichText::new(cta).size(13.0).color(theme::ACCENT_HOVER));
        })
        .response;
    resp.interact(egui::Sense::click())
}

fn kv(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal_top(|ui| {
        ui.add_sized([92.0, 18.0], egui::Label::new(theme::small(key)));
        let avail = (ui.available_width() - 2.0).max(120.0);
        ui.allocate_ui_with_layout(
            Vec2::new(avail, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.add(egui::Label::new(RichText::new(value).size(13.0).color(theme::TEXT)).wrap());
            },
        );
    });
}

// --------------------------------------------------------------------- install

impl LoosSwitchApp {
    fn page_install(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(theme::h1("Install LoOS"));
                ui.label(theme::muted("Choose a safe target, review the exact plan, then let it run."));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                theme::pill(ui, if self.expert { "Power user" } else { "Simple mode" }, theme::MUTED, theme::CARD_ALT);
            });
        });
        ui.add_space(12.0);
        steps(ui, self.install_stage);
        ui.add_space(16.0);
        match self.install_stage {
            InstallStage::Choose => self.install_choose(ui),
            InstallStage::Review => self.install_review(ui),
            InstallStage::Running => self.install_running(ui),
            InstallStage::Finished => self.install_finished(ui),
        }
    }

    fn install_choose(&mut self, ui: &mut egui::Ui) {
        theme::card().show(ui, |ui| {
            ui.label(theme::h2("1. How should LoOS get onto this PC?"));
            ui.add_space(8.0);
            method_button(
                ui,
                self.method == InstallMethod::InstallMedia,
                "Install media (recommended)",
                "Write a verified LoOS installer to a USB drive or dedicated disk. Boot it, then LoOS Setup installs the OS. This is the only supported way to replace the disk Windows/Linux is currently running from.",
                "UEFI + BIOS",
                &mut self.method,
                InstallMethod::InstallMedia,
            );
            if self.firmware == Firmware::Uefi || self.firmware_override {
                method_button(
                    ui,
                    self.method == InstallMethod::LocalSetup,
                    "Stage setup on this PC (UEFI only)",
                    "Add a small LoOS Setup loader to the existing EFI System Partition and a firmware boot entry. Nothing is repartitioned or erased. Use it to install onto a second disk without a USB stick.",
                    "no repartitioning",
                    &mut self.method,
                    InstallMethod::LocalSetup,
                );
            }
            if self.method == InstallMethod::LocalSetup {
                if let Some(sys) = &self.system {
                    ui.add_space(6.0);
                    theme::inset().show(ui, |ui| {
                        ui.label(theme::small("EFI System Partition"));
                        ui.label(RichText::new(sys.esp_mount.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "not mounted".into())).size(13.0).color(theme::TEXT));
                        if sys.secure_boot == Some(true) {
                            ui.add_space(4.0);
                            ui.label(RichText::new("Secure Boot is on; the unsigned loader will be refused.").size(12.5).color(theme::WARN));
                        }
                    });
                }
            }
        });
        ui.add_space(14.0);
        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::h2("2. Choose the target disk"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Rescan").clicked() {
                        self.refresh();
                    }
                });
            });
            ui.label(theme::muted(if self.method == InstallMethod::InstallMedia {
                "Every byte on the selected disk will be replaced by the installer image. Use a USB stick or a disk you do not need."
            } else {
                "Choose the second disk that will receive LoOS. The running system disk is never offered. Interactive Setup asks again inside LoOS before writing."
            }));
            ui.add_space(10.0);
            let targets: Vec<Disk> = self.disks.clone();
            let mut shown = 0;
            for d in &targets {
                let safe = !d.system_disk && !d.read_only && d.size_bytes >= 2 * 1024 * 1024 * 1024;
                let mounted_ok = self.method == InstallMethod::LocalSetup || cfg!(windows) || d.mount_count() == 0;
                let enabled = safe && mounted_ok;
                if d.system_disk {
                    continue;
                }
                shown += 1;
                let selected = self.selected_disk.as_deref() == Some(d.id.as_str());
                let reason = if d.read_only {
                    Some("read-only")
                } else if d.size_bytes < 2 * 1024 * 1024 * 1024 {
                    Some("smaller than 2 GiB")
                } else if !mounted_ok {
                    Some("has mounted partitions — unmount it first")
                } else {
                    None
                };
                let resp = disk_card(ui, d, selected, enabled, reason);
                if resp.clicked() && enabled {
                    self.selected_disk = Some(d.id.clone());
                }
                ui.add_space(3.0);
            }
            if shown == 0 {
                theme::warning_box(
                    ui,
                    "No non-system disk found",
                    "Connect a USB drive (8 GB or larger) for installer media, or add a second internal disk. The app refuses to overwrite the disk it is running from.",
                );
            } else {
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(theme::small("Select:"));
                    for d in targets.iter().filter(|d| !d.system_disk) {
                        let enabled = !d.read_only && d.size_bytes >= 2 * 1024 * 1024 * 1024 && (self.method == InstallMethod::LocalSetup || cfg!(windows) || d.mount_count() == 0);
                        let selected = self.selected_disk.as_deref() == Some(d.id.as_str());
                        if ui.add_enabled(enabled, egui::Button::selectable(selected, format!("{} ({})", d.friendly_name(), human_bytes(d.size_bytes)))).clicked() {
                            self.selected_disk = Some(d.id.clone());
                        }
                    }
                });
            }
        });
        ui.add_space(14.0);
        if self.expert {
            self.install_power_options(ui);
        } else {
            ui.label(theme::small("Power-user options (firmware image, unattended setup, boot label) are in the mode switch on the left."));
        }
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.add(theme::secondary_button("← Back")).clicked() {
                self.page = Page::Home;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enabled = self.selected_disk.is_some();
                if ui.add_enabled(enabled, theme::primary_button("Review plan →")).clicked() {
                    self.go_review();
                }
            });
        });
    }

    fn install_power_options(&mut self, ui: &mut egui::Ui) {
        theme::card_alt().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::h2("Power options"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut self.unattended, "Unattended target install");
                });
            });
            ui.label(theme::small("These options are only used if you expand them. Defaults are safe."));
            ui.add_space(6.0);
            egui::CollapsingHeader::new("Exact deployment settings").default_open(false).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::muted("Firmware image:"));
                    ui.selectable_value(&mut self.firmware, Firmware::Uefi, "UEFI");
                    ui.selectable_value(&mut self.firmware, Firmware::Bios, "Legacy BIOS");
                    ui.checkbox(&mut self.firmware_override, "keep this choice");
                });
                ui.horizontal(|ui| {
                    ui.label(theme::muted("Boot entry label:"));
                    ui.add(egui::TextEdit::singleline(&mut self.boot_label).desired_width(220.0));
                });
                ui.checkbox(&mut self.verify_readback, "Verify every written byte by reading it back (recommended)");
                ui.checkbox(&mut self.reboot_when_done, "Reboot automatically when the operation finishes");
            });
            if self.unattended {
                ui.add_space(10.0);
                theme::warning_box(
                    ui,
                    "Unattended mode erases the target disk",
                    "After reboot, LoOS Setup uses a one-time account and installs without asking. The target must be a non-system disk. Use a temporary password: it is written to the ESP until Setup consumes it.",
                );
                ui.add_space(6.0);
                egui::Grid::new("unattended_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label(theme::muted("User name"));
                    ui.add(egui::TextEdit::singleline(&mut self.admin_user).desired_width(240.0));
                    ui.end_row();
                    ui.label(theme::muted("One-time password"));
                    ui.add(egui::TextEdit::singleline(&mut self.admin_password).password(true).desired_width(240.0));
                    ui.end_row();
                    ui.label(theme::muted("Computer name"));
                    ui.add(egui::TextEdit::singleline(&mut self.computer_name).desired_width(240.0));
                    ui.end_row();
                });
                if !self.admin_user.is_empty() && !valid_login_name(&self.admin_user) {
                    ui.label(theme::danger("User name must start with a-z; use a-z, 0-9, '-' or '_'."));
                }
                if !self.computer_name.is_empty() && !valid_hostname(&self.computer_name) {
                    ui.label(theme::danger("Computer name must be 1–31 letters, digits or '-'."));
                }
            }
        });
    }

    fn install_review(&mut self, ui: &mut egui::Ui) {
        let Some(plan) = self.plan.clone() else {
            self.install_stage = InstallStage::Choose;
            return;
        };
        let dangerous = plan.method == InstallMethod::InstallMedia || plan.unattended;
        theme::card().show(ui, |ui| {
            ui.label(theme::h2("Review the exact plan"));
            ui.add_space(8.0);
            let target = plan.target_disk.as_ref().map(|d| d.identity_line()).unwrap_or_default();
            kv(ui, "Method", match plan.method {
                InstallMethod::InstallMedia => "write verified installer media",
                InstallMethod::LocalSetup => "stage setup on the existing ESP (no repartitioning)",
            });
            kv(ui, "Firmware", plan.firmware.label());
            kv(ui, "Target", &target);
            kv(ui, "Writes", if dangerous { "the selected disk only" } else { "ESP files + one firmware boot entry" });
            kv(ui, "Running system disk", "protected — never a target");
            kv(ui, "Verification", if plan.verify_readback { "readback + SHA-256" } else { "disabled (not recommended)" });
            if plan.unattended {
                kv(ui, "After reboot", "automatic install to the selected disk");
                kv(ui, "One-time user", &plan.admin_user);
                kv(ui, "Target", &format!("{:.1} GiB exact-size match", plan.target_disk.as_ref().map(|d| d.size_gib()).unwrap_or(0.0)));
            } else if plan.method == InstallMethod::LocalSetup {
                kv(ui, "After reboot", "LoOS Setup asks for the target and account");
            }
        });
        ui.add_space(12.0);
        if dangerous {
            theme::danger_box(
                ui,
                "This is destructive",
                "Writing the installer image replaces the partition table and the first part of the selected disk. Everything on that disk will be gone. The disk Windows/Linux is running from is refused by the engine, and no other disk is touched.",
            );
        } else {
            theme::warning_box(
                ui,
                "This changes boot files (but not partitions)",
                "The app copies a LoOS loader to the existing ESP and adds a firmware boot entry. It does not repartition or erase. A firmware boot entry can still affect boot order; the existing Windows/Linux entry is left in place.",
            );
        }
        ui.add_space(12.0);
        theme::card_alt().show(ui, |ui| {
            ui.label(theme::h2("Before you continue"));
            if dangerous {
                ui.checkbox(&mut self.backup_ack, "I have a backup of anything important on the target disk.");
            } else {
                ui.checkbox(&mut self.backup_ack, "I understand this adds a firmware boot entry to this PC.");
            }
            ui.add_space(6.0);
            let phrase = if dangerous { "ERASE" } else { "STAGE" };
            ui.label(theme::muted(format!("Type {phrase} to confirm. This is the last screen before any writing starts.")));
            ui.add(egui::TextEdit::singleline(&mut self.phrase).hint_text(phrase).desired_width(220.0));
        });
        ui.add_space(12.0);
        if let Some(sys) = &self.system {
            if !sys.elevated {
                theme::warning_box(
                    ui,
                    "Administrator permission required",
                    "Disk and ESP operations need elevation. Use the button below; the app restarts as administrator. Re-open this screen and choose the same target. Nothing has been written yet.",
                );
                if ui.add(theme::primary_button("Restart as administrator")).clicked() {
                    if let Err(e) = platform::elevate_self() {
                        self.toast(e.to_string());
                    }
                }
                ui.add_space(8.0);
            }
        }
        let expected = if dangerous { "ERASE" } else { "STAGE" };
        let ready = self.phrase.trim().eq_ignore_ascii_case(expected) && self.backup_ack;
        ui.horizontal(|ui| {
            if ui.add(theme::secondary_button("← Back")).clicked() {
                self.install_stage = InstallStage::Choose;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let btn = if dangerous {
                    theme::danger_button(if plan.method == InstallMethod::InstallMedia { "Write installer" } else { "Stage + auto-install" })
                } else {
                    theme::primary_button("Stage LoOS Setup")
                };
                if ui.add_enabled(ready, btn).clicked() {
                    self.start_install();
                }
            });
        });
        if !ready {
            ui.label(theme::small("The action button unlocks after the confirmation text and acknowledgement."));
        }
    }

    fn install_running(&mut self, ui: &mut egui::Ui) {
        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::h2(if self.install_phase.is_empty() { "Working…" } else { &self.install_phase }));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.cancel_requested {
                        theme::pill(ui, "cancel requested", theme::WARN, theme::WARN_DIM);
                    } else {
                        theme::pill(ui, "cancelable", theme::SUCCESS, theme::SUCCESS_DIM);
                    }
                });
            });
            ui.add_space(10.0);
            let pct = (self.install_progress * 100.0).clamp(0.0, 100.0);
            ui.add(
                egui::ProgressBar::new(self.install_progress)
                    .desired_height(22.0)
                    .text(format!("{pct:.0}% · {}", self.install_detail)),
            );
            ui.add_space(10.0);
            ui.label(theme::muted(
                "Cancel is checked before every 4 MiB block. After cancel, no new writes begin; the operation reports exactly what was left incomplete.",
            ));
            if self.cancel_requested {
                ui.label(theme::small("Waiting for the current disk block to return…"));
            } else if ui.add(theme::danger_button("Cancel this operation")).clicked() {
                self.cancel_job();
            }
        });
        ui.add_space(12.0);
        self.log_panel(ui, true);
    }

    fn install_finished(&mut self, ui: &mut egui::Ui) {
        let result = self.install_done.clone();
        match result {
            Some(Ok(outcome)) => {
                theme::card().show(ui, |ui| {
                    ui.label(theme::title("Ready."));
                    ui.add_space(4.0);
                    ui.label(theme::muted(&outcome.summary));
                    ui.add_space(12.0);
                    if let Some(e) = &outcome.boot_entry {
                        theme::inset().show(ui, |ui| {
                            ui.label(theme::small("Boot entry / target"));
                            ui.label(RichText::new(e).size(13.0).color(theme::TEXT));
                        });
                    }
                    ui.add_space(12.0);
                    theme::success_box(ui, "Verified locally", "No network call was made. Every byte came from the signed-off payload inside this binary, and the readback hash matched.");
                    ui.add_space(14.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.add(theme::primary_button("Reboot now")).clicked() {
                            match platform::reboot() {
                                Ok(_) => self.toast("Reboot requested."),
                                Err(e) => self.toast(e.to_string()),
                            }
                        }
                        if ui.add(theme::secondary_button("Move my files")).clicked() {
                            self.page = Page::Migration;
                        }
                        if ui.add(theme::secondary_button("Back to start")).clicked() {
                            self.install_stage = InstallStage::Choose;
                            self.page = Page::Home;
                            self.install_done = None;
                        }
                        if ui.add(theme::secondary_button("Save log")).clicked() {
                            self.save_log();
                        }
                    });
                    ui.add_space(8.0);
                    ui.label(theme::small("Next boot: select 'LoOS Setup' in the firmware boot menu if this PC does not boot it automatically. The LoOS installer will ask for a target and local account."));
                });
            }
            Some(Err(e)) => {
                theme::card().show(ui, |ui| {
                    ui.label(theme::h1(if self.cancel_requested { "Cancelled" } else { "Deployment stopped" }));
                    ui.add_space(6.0);
                    if self.cancel_requested {
                        theme::warning_box(ui, "Cancelled safely", "No further blocks were written. If this was installer media, the target may contain a partial image and is not bootable. Re-run the write to finish, or discard the media.");
                    } else {
                        theme::danger_box(ui, "The operation did not complete", &e);
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_button("Try again")).clicked() {
                            self.install_stage = InstallStage::Choose;
                            self.install_done = None;
                            self.phrase.clear();
                            self.backup_ack = false;
                        }
                        if ui.add(theme::secondary_button("Save log")).clicked() {
                            self.save_log();
                        }
                    });
                });
            }
            None => {
                self.install_stage = InstallStage::Choose;
            }
        }
        ui.add_space(12.0);
        self.log_panel(ui, true);
    }

    fn log_panel(&mut self, ui: &mut egui::Ui, collapsed: bool) {
        egui::CollapsingHeader::new("Operation log (local only)")
            .default_open(!collapsed)
            .show(ui, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(7, 11, 17))
                    .stroke(Stroke::new(1.0_f32, theme::BORDER))
                    .corner_radius(8)
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(220.0).stick_to_bottom(true).show(ui, |ui| {
                            for l in self.logs.iter().rev().take(200).rev() {
                                ui.label(RichText::new(l).monospace().size(11.5).color(theme::MUTED));
                            }
                        });
                    });
                ui.add_space(6.0);
                if ui.button("Save log").clicked() {
                    self.save_log();
                }
            });
    }
}

// ------------------------------------------------------------------- migration

impl LoosSwitchApp {
    fn page_migration(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(theme::h1("Move my files"));
                ui.label(theme::muted("Copy-only. Pick folders, choose a destination, verify every file. Nothing is deleted from Windows or Linux."));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                theme::pill(ui, "copy-only", theme::ACCENT_HOVER, theme::ACCENT_DIM);
            });
        });
        ui.add_space(14.0);
        match self.migration_stage {
            MigrationStage::Choose => self.migration_choose(ui),
            MigrationStage::Running => self.migration_running(ui),
            MigrationStage::Finished => self.migration_finished(ui),
        }
    }

    fn migration_choose(&mut self, ui: &mut egui::Ui) {
        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::h2("1. What should come with you?"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("+ Add folder").clicked() {
                        self.picker_mode = PickerMode::Source;
                        self.folder_picker = Some(FolderPicker {
                            path: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
                            title: "Add a folder to migrate".into(),
                        });
                    }
                    if ui.small_button("Measure selected").clicked() {
                        self.scan_sources();
                    }
                });
            });
            ui.label(theme::small("Quick folders were discovered from your user profile. Hidden caches and system folders are excluded by default."));
            ui.add_space(8.0);
            if self.sources.is_empty() {
                theme::warning_box(ui, "No user folders found", "Use “Add folder” to choose any folder from this computer or a mounted drive.");
            }
            let mut remove: Option<usize> = None;
            for (i, s) in self.sources.iter_mut().enumerate() {
                Frame::new()
                    .fill(theme::CARD_ALT)
                    .stroke(Stroke::new(1.0_f32, theme::BORDER))
                    .corner_radius(9)
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut s.selected, "");
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&s.destination).size(14.0).color(theme::TEXT));
                                ui.label(theme::small(s.path.display().to_string()));
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Remove").clicked() {
                                    remove = Some(i);
                                }
                                let size = s.size_bytes.map(human_bytes).unwrap_or_else(|| "not measured".into());
                                let files = s.file_count.map(|n| format!(" · {n} files")).unwrap_or_default();
                                ui.label(theme::muted(format!("{size}{files}")));
                            });
                        });
                    });
                ui.add_space(2.0);
            }
            if let Some(i) = remove {
                self.sources.remove(i);
            }
        });
        ui.add_space(14.0);
        theme::card().show(ui, |ui| {
            ui.label(theme::h2("2. Where should the bundle go?"));
            ui.label(theme::muted("Choose a different disk or an external drive — ideally the backup drive you will keep after switching. A portable bundle is safest."));
            ui.add_space(6.0);
            ui.checkbox(&mut self.direct_home, "Destination is already a mounted LoOS home (copy directly, no bundle folder)");
            if self.direct_home {
                ui.label(theme::small("Power option: select a mounted /home/<user> from an installed LoOS system. Files are copied straight into it."));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(theme::secondary_button("Choose destination…")).clicked() {
                    self.picker_mode = PickerMode::Destination;
                    self.folder_picker = Some(FolderPicker {
                        path: self.destination.clone().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("/")),
                        title: "Choose a migration destination".into(),
                    });
                }
                match &self.destination {
                    Some(p) => {
                        ui.label(RichText::new(p.display().to_string()).size(13.5).color(theme::TEXT));
                        let free = fs2::available_space(p).unwrap_or(0);
                        self.migration_free = free;
                        ui.label(theme::small(format!("{} free", human_bytes(free))));
                    }
                    None => {
                        ui.label(theme::muted("No destination chosen yet."));
                    }
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(theme::muted("LoOS user name:"));
                ui.add(egui::TextEdit::singleline(&mut self.target_home).desired_width(200.0));
                ui.label(theme::small("Folders land under /home/<name>/ inside the bundle."));
            });
            if !valid_login_name(&self.target_home) {
                ui.label(theme::danger("Use a-z, 0-9, '-' or '_'; start with a-z; not root."));
            }
        });
        ui.add_space(14.0);
        egui::CollapsingHeader::new("Migration options")
            .default_open(false)
            .show(ui, |ui| {
                theme::card_alt().show(ui, |ui| {
                    ui.checkbox(&mut self.migration_options.include_hidden, "Include hidden files and folders");
                    ui.checkbox(&mut self.migration_options.verify_hashes, "SHA-256 verify every copied file (recommended)");
                    ui.checkbox(&mut self.migration_options.preserve_timestamps, "Preserve original modification times");
                    ui.checkbox(&mut self.migration_options.follow_symlinks, "Follow symbolic links (may copy outside the selected folder)");
                    ui.horizontal(|ui| {
                        ui.label(theme::muted("If a file exists:"));
                        egui::ComboBox::from_id_salt("conflict")
                            .selected_text(self.migration_options.conflict.label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.migration_options.conflict, ConflictPolicy::Skip, ConflictPolicy::Skip.label());
                                ui.selectable_value(&mut self.migration_options.conflict, ConflictPolicy::Overwrite, ConflictPolicy::Overwrite.label());
                                ui.selectable_value(&mut self.migration_options.conflict, ConflictPolicy::Rename, ConflictPolicy::Rename.label());
                            });
                    });
                    ui.label(theme::muted("Excluded folder names (comma-separated):"));
                    ui.add(egui::TextEdit::singleline(&mut self.exclusions_text).desired_width(f32::INFINITY));
                });
            });
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.add(theme::secondary_button("← Back")).clicked() {
                self.page = Page::Home;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let selected: Vec<_> = self.sources.iter().filter(|s| s.selected).collect();
                let total: u64 = selected.iter().filter_map(|s| s.size_bytes).sum();
                let can_start = !selected.is_empty() && self.destination.is_some() && valid_login_name(&self.target_home) && self.job.is_none();
                let label = if total > 0 { format!("Start copy · {}", human_bytes(total)) } else { "Start copy".into() };
                if ui.add_enabled(can_start, theme::primary_button(label)).clicked() {
                    self.migration_options.excluded_names = self
                        .exclusions_text
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    self.start_migration();
                }
            });
        });
        ui.add_space(6.0);
        ui.label(theme::small("If sizes are not measured yet, the copy still works but the progress estimate is less precise. “Measure selected” never copies files."));
    }

    fn migration_running(&mut self, ui: &mut egui::Ui) {
        let p = self.migration_progress.clone();
        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::h2("Copying and verifying"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.cancel_requested {
                        theme::pill(ui, "cancel requested", theme::WARN, theme::WARN_DIM);
                    } else {
                        theme::pill(ui, "copy-only", theme::SUCCESS, theme::SUCCESS_DIM);
                    }
                });
            });
            let frac = if p.bytes_total > 0 {
                (p.bytes_done as f64 / p.bytes_total as f64) as f32
            } else if p.files_total > 0 {
                (p.files_done as f64 / p.files_total as f64) as f32
            } else {
                0.0
            };
            ui.add_space(10.0);
            ui.add(
                egui::ProgressBar::new(frac.clamp(0.0, 1.0))
                    .desired_height(22.0)
                    .text(format!(
                        "{} / {} · {} files{}",
                        human_bytes(p.bytes_done),
                        human_bytes(p.bytes_total),
                        p.files_done,
                        if p.skipped > 0 { format!(" · {} skipped", p.skipped) } else { String::new() }
                    )),
            );
            if p.errors > 0 {
                ui.label(theme::danger(format!("{} file(s) reported an error; details are in the manifest.", p.errors)));
            }
            ui.add_space(6.0);
            ui.label(theme::small(if p.current.is_empty() { "Preparing…".into() } else { p.current.clone() }));
            ui.add_space(10.0);
            if !self.cancel_requested {
                if ui.add(theme::danger_button("Cancel copy")).clicked() {
                    self.cancel_job();
                }
            } else {
                ui.label(theme::small("Waiting for the current chunk to finish; completed files stay verified and journaled."));
            }
        });
        ui.add_space(12.0);
        self.log_panel(ui, true);
    }

    fn migration_finished(&mut self, ui: &mut egui::Ui) {
        let result = self.migration_done.clone();
        match result {
            Some(Ok(m)) => {
                theme::card().show(ui, |ui| {
                    ui.label(theme::title("Your files are packed."));
                    ui.add_space(4.0);
                    ui.label(theme::muted(format!(
                        "{} files · {} · {} skipped · {} errors. Source files were never modified.",
                        m.stats.files_copied,
                        human_bytes(m.stats.bytes_copied),
                        m.stats.files_skipped,
                        m.stats.errors.len()
                    )));
                    ui.add_space(12.0);
                    if let Some(p) = &self.migration_plan {
                        theme::inset().show(ui, |ui| {
                            ui.label(theme::small("Migration bundle"));
                            ui.label(RichText::new(p.destination_root.display().to_string()).size(13.0).color(theme::TEXT));
                            ui.label(theme::small(format!("manifest.json + RESTORE.txt · target /home/{}", m.target_home)));
                        });
                    }
                    ui.add_space(12.0);
                    if m.stats.errors.is_empty() {
                        theme::success_box(ui, "Verified", "Every copied file was read back and checked against its source SHA-256. The bundle is complete.");
                    } else {
                        theme::warning_box(ui, "Completed with warnings", &m.stats.errors.join("\n"));
                    }
                    ui.add_space(14.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.add(theme::primary_button("Install LoOS")).clicked() {
                            self.page = Page::Install;
                            self.install_stage = InstallStage::Choose;
                        }
                        if ui.add(theme::secondary_button("Back to start")).clicked() {
                            self.page = Page::Home;
                            self.migration_stage = MigrationStage::Choose;
                            self.migration_done = None;
                        }
                        if ui.add(theme::secondary_button("Save log")).clicked() {
                            self.save_log();
                        }
                    });
                });
            }
            Some(Err(e)) => {
                theme::card().show(ui, |ui| {
                    ui.label(theme::h1(if self.cancel_requested { "Copy cancelled" } else { "Copy stopped" }));
                    ui.add_space(6.0);
                    if self.cancel_requested {
                        theme::warning_box(ui, "Cancelled safely", "Completed files remain in the bundle. Run the same plan again to resume: same-size files with the same timestamp are skipped.");
                    } else {
                        theme::danger_box(ui, "Migration did not complete", &e);
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_button("Back")).clicked() {
                            self.migration_stage = MigrationStage::Choose;
                            self.migration_done = None;
                        }
                        if ui.add(theme::secondary_button("Save log")).clicked() {
                            self.save_log();
                        }
                    });
                });
            }
            None => self.migration_stage = MigrationStage::Choose,
        }
        ui.add_space(12.0);
        self.log_panel(ui, true);
    }
}

// ---------------------------------------------------------------- static pages

impl LoosSwitchApp {
    fn page_safety(&mut self, ui: &mut egui::Ui) {
        ui.label(theme::h1("Safety & recovery"));
        ui.label(theme::muted("The rules this app follows, in plain language."));
        ui.add_space(16.0);
        ui.columns(2, |cols| {
            theme::card().show(&mut cols[0], |ui| {
                ui.label(theme::h2("What this app will never do"));
                ui.add_space(6.0);
                for item in [
                    "Write to the disk that hosts the running Windows/Linux system.",
                    "Repartition a disk just because you opened the Install page.",
                    "Delete or move files during migration.",
                    "Send a log, hardware ID, ping or crash report anywhere.",
                    "Ask for a Microsoft, Google or LoOS cloud account.",
                    "Silently enable Secure Boot bypasses, network access or telemetry.",
                ] {
                    bullet(ui, item, theme::SUCCESS);
                }
            });
            theme::card().show(&mut cols[1], |ui| {
                ui.label(theme::h2("What it does do"));
                ui.add_space(6.0);
                for item in [
                    "Writes only to the exact whole disk you confirm.",
                    "Verifies the embedded payload hash before and after writing.",
                    "Checks cancel before every 4 MiB block.",
                    "Leaves the source untouched when migrating.",
                    "Journals migration progress so a cancelled copy can resume.",
                    "Keeps all diagnostics local until you choose to save them.",
                ] {
                    bullet(ui, item, theme::ACCENT_HOVER);
                }
            });
        });
        ui.add_space(14.0);
        theme::card().show(ui, |ui| {
            ui.label(theme::h2("Cancel semantics — the honest version"));
            ui.label(theme::muted(
                "A disk write is not a text edit. Cancel is checked before every 4 MiB block and before every file/chunk in migration. Once a block has started it is allowed to finish so the drive is not left mid-command; after that no new block begins. If you cancel while writing installer media, the target is left with a partial image and is not bootable. Re-run the write to finish, or discard the stick. The running system disk is never touched.",
            ));
            ui.add_space(8.0);
            ui.label(theme::h2("Replacing the disk LoOS runs from"));
            ui.label(theme::muted(
                "An operating system cannot safely repartition the disk it is booted from while its filesystems are mounted. For that job, use “Install media”: write a USB installer, boot it, and let LoOS Setup use the whole disk. Windows and Linux support this route identically. If LoOS should go on a second disk, UEFI users can stage setup on the existing ESP without touching partitions.",
            ));
        });
        ui.add_space(14.0);
        theme::card().show(ui, |ui| {
            ui.label(theme::h2("Before any destructive step"));
            for item in [
                "Copy important files with Move my files to a different physical drive.",
                "Check the target's model, size and serial in the review screen.",
                "Keep the Windows/Linux recovery media you already have.",
                "On BitLocker systems, suspend protection and keep the recovery key.",
                "On Secure Boot systems, either turn Secure Boot off or use a signed LoOS build.",
            ] {
                bullet(ui, item, theme::WARN);
            }
        });
        ui.add_space(14.0);
        theme::card().show(ui, |ui| {
            ui.label(theme::h2("Recovery if a boot entry goes wrong"));
            ui.label(theme::muted(
                "The existing Windows/Linux boot entry and boot order are left in place. To remove a staged LoOS setup entry, open firmware settings or use efibootmgr on Linux; Windows users can use firmware settings or a Linux live USB. Media writes are whole-disk operations by design — recovery means re-running your previous OS installer or restoring from the backup you made.",
            ));
        });
    }

    fn page_logs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(theme::h1("Activity log"));
                ui.label(theme::muted("Local, in-memory, and never transmitted. Save it only if you want to keep it."));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(theme::secondary_button("Save log")).clicked() {
                    self.save_log();
                }
            });
        });
        ui.add_space(14.0);
        theme::card().show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(520.0).stick_to_bottom(true).show(ui, |ui| {
                for l in &self.logs {
                    ui.label(RichText::new(l).monospace().size(12.0).color(theme::MUTED));
                }
                if self.logs.is_empty() {
                    ui.label(theme::muted("No activity yet."));
                }
            });
        });
    }

    fn capture_frame(&mut self, ctx: &egui::Context) {
        let Some(cap) = &mut self.screenshot else { return };
        self.capture_frames += 1;
        let mut image = None;
        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Screenshot { image: img, .. } = ev {
                    image = Some(img.clone());
                }
            }
        });
        if let Some(img) = image {
            let w = img.width() as u32;
            let h = img.height() as u32;
            let mut bytes = Vec::with_capacity((w * h * 4) as usize);
            for p in &img.pixels {
                bytes.extend_from_slice(&p.to_array());
            }
            match image::save_buffer(&cap.path, &bytes, w, h, image::ColorType::Rgba8) {
                Ok(_) => eprintln!("screenshot saved to {}", cap.path.display()),
                Err(e) => eprintln!("screenshot failed: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.capture_frames == 8 && !cap.requested {
            cap.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
    }

    fn page_about(&mut self, ui: &mut egui::Ui) {
        ui.label(theme::h1("About & privacy"));
        ui.label(theme::muted("LoOS Switch is a local installer and migration front end for LoOS — Local only OS."));
        ui.add_space(16.0);
        ui.columns(2, |cols| {
            theme::card().show(&mut cols[0], |ui| {
                ui.label(theme::h2("Zero telemetry, by construction"));
                for item in [
                    "No HTTP/TLS client is linked into the binary.",
                    "No analytics, crash-reporting or update SDK is present.",
                    "No device fingerprint, machine ID or account is collected.",
                    "No background network activity is started at launch.",
                    "The only data written is the deployment/migration output you ask for.",
                ] {
                    bullet(ui, item, theme::SUCCESS);
                }
                ui.add_space(8.0);
                ui.label(theme::small("Verify it: run the binary with --self-test, watch it with a network monitor, or read the source. Nothing is hidden behind an account."));
            });
            theme::card().show(&mut cols[1], |ui| {
                ui.label(theme::h2("Build details"));
                kv(ui, "Version", APP_VERSION);
                kv(ui, "Runtime", "Rust / egui / glow");
                kv(ui, "Payload date", payload_date());
                kv(ui, "Platform", std::env::consts::OS);
                kv(ui, "Arch", std::env::consts::ARCH);
                kv(ui, "Licence", "MIT (app code)");
                ui.add_space(8.0);
                ui.label(theme::small("The LoOS installer payload is built from the LoOS tree in this workspace. Third-party notices are in THIRD-PARTY.md."));
            });
        });
        ui.add_space(14.0);
        theme::card().show(ui, |ui| {
            ui.label(theme::h2("What “local only” means here"));
            ui.label(theme::muted(
                "The installer needs no network. It carries the boot payload inside the executable, verifies a SHA-256 digest of the compressed and raw image, and writes only after you type a confirmation. If the internet is disconnected, everything in this app still works.",
            ));
            ui.add_space(8.0);
            ui.label(theme::h2("Power-user entry points"));
            ui.label(theme::monospace("loos-switch --self-test"));
            ui.label(theme::monospace("loos-switch --list-disks"));
            ui.label(theme::monospace("loos-switch --make-media --device <disk> --firmware uefi --yes"));
            ui.label(theme::monospace("loos-switch --stage-uefi --target-disk <id>"));
            ui.label(theme::monospace("loos-switch --backup <folder> --to <destination>"));
        });
    }
}

// ---------------------------------------------------------------------- helpers

impl eframe::App for LoosSwitchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.top_bar(ctx);
        self.sidebar(ctx);
        self.central(ctx);
        self.folder_picker_window(ctx);

        if self.screenshot.is_some() {
            self.capture_frame(ctx);
        }

        if let Some((text, at)) = self.toast.clone() {
            if at.elapsed() < Duration::from_secs(5) {
                egui::Area::new("toast".into())
                    .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -24.0])
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        egui::Frame::new()
                            .fill(theme::CARD_HOVER)
                            .stroke(Stroke::new(1.0_f32, theme::ACCENT))
                            .corner_radius(10)
                            .inner_margin(Margin::symmetric(18, 10))
                            .show(ui, |ui| {
                                ui.label(RichText::new(text).size(13.5).color(theme::TEXT));
                            });
                    });
                ctx.request_repaint_after(Duration::from_millis(200));
            } else {
                self.toast = None;
            }
        }
    }
}

fn method_button(
    ui: &mut egui::Ui,
    selected: bool,
    title: &str,
    desc: &str,
    badge: &str,
    current: &mut InstallMethod,
    value: InstallMethod,
) {
    let fill = if selected { theme::ACCENT_DIM } else { theme::CARD_ALT };
    let stroke = if selected { theme::ACCENT } else { theme::BORDER };
    let resp = Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, stroke))
        .corner_radius(12)
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let _ = ui.radio(selected, "");
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(theme::h2(title));
                        theme::pill(ui, badge, theme::MUTED, theme::CARD);
                    });
                    ui.label(RichText::new(desc).size(13.0).color(theme::MUTED));
                });
            });
        })
        .response
        .interact(egui::Sense::click());
    if resp.clicked() {
        *current = value;
    }
}

fn disk_card(ui: &mut egui::Ui, d: &Disk, selected: bool, enabled: bool, reason: Option<&str>) -> egui::Response {
    let fill = if selected { theme::ACCENT_DIM } else if enabled { theme::CARD_ALT } else { Color32::from_rgb(15, 21, 31) };
    let stroke = if selected { theme::ACCENT } else { theme::BORDER };
    let resp = Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, stroke))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let _ = ui.radio(selected, "");
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        let color = if enabled { theme::TEXT } else { theme::DIM };
                        ui.label(RichText::new(d.friendly_name()).size(14.0).color(color));
                        if d.is_usb() {
                            theme::pill(ui, "USB", theme::ACCENT_HOVER, theme::ACCENT_DIM);
                        }
                        if d.removable {
                            theme::pill(ui, "removable", theme::MUTED, theme::CARD);
                        }
                        if d.system_disk {
                            theme::pill(ui, "SYSTEM", theme::DANGER, theme::DANGER_DIM);
                        }
                    });
                    ui.label(theme::small(d.identity_line()));
                    if let Some(r) = reason {
                        ui.label(RichText::new(format!("Not available: {r}")).size(12.0).color(theme::WARN));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(theme::small(if d.mount_count() > 0 { format!("{} mount(s)", d.mount_count()) } else { d.partition_style.clone() }));
                });
            });
        })
        .response;
    if enabled {
        resp.interact(egui::Sense::click())
    } else {
        resp
    }
}

fn steps(ui: &mut egui::Ui, stage: InstallStage) {
    let labels = ["1  Choose", "2  Review", "3  Deploy"];
    let active = match stage {
        InstallStage::Choose => 0,
        InstallStage::Review => 1,
        InstallStage::Running => 2,
        InstallStage::Finished => 2,
    };
    ui.horizontal(|ui| {
        for (i, l) in labels.iter().enumerate() {
            let done = i < active || (stage == InstallStage::Finished && i <= active);
            let fg = if i == active { theme::TEXT } else if done { theme::SUCCESS } else { theme::DIM };
            theme::pill(
                ui,
                &format!("{}{}", if done && i != active { "✓  " } else { "" }, l),
                fg,
                if i == active { theme::ACCENT_DIM } else { theme::CARD_ALT },
            );
            if i < labels.len() - 1 {
                ui.label(theme::small("—"));
            }
        }
    });
}

fn bullet(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("•").color(color).size(16.0));
        ui.label(RichText::new(text).size(13.5).color(theme::TEXT));
    });
}

fn sanitise_login(s: &str) -> String {
    let mut out: String = s
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-')
        .collect();
    out = out.trim_start_matches(|c: char| !c.is_ascii_lowercase()).to_string();
    if out.is_empty() { "user".into() } else { out.chars().take(31).collect() }
}

fn sanitise_host(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' })
        .take(31)
        .collect();
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "loos-pc".into() } else { out }
}

fn short_time() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        % 86400;
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

fn payload_date() -> &'static str {
    "2026-11-02"
}
