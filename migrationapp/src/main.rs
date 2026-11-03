#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod app;
mod install;
mod migration;
mod model;
mod payload;
mod platform;
mod theme;

use install::InstallEvent;
use model::{Firmware, InstallMethod, InstallPlan, MigrationOptions, MigrationSource};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[cfg(target_os = "windows")]
fn attach_console_for_cli() {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::Console::{AttachConsole, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
    let path: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) != 0 {
            let h = CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            );
            if h != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
                SetStdHandle(STD_OUTPUT_HANDLE, h);
                SetStdHandle(STD_ERROR_HANDLE, h);
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(target_os = "windows")]
    if !args.is_empty() {
        attach_console_for_cli();
    }
    if args.is_empty() {
        launch_gui(None, None);
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--screenshot") {
        let path = args.get(i + 1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("loos-switch.png"));
        let page = args.iter().position(|a| a == "--screenshot-page").and_then(|j| args.get(j + 1)).map(String::as_str);
        launch_gui(Some(path), page);
        return;
    }
    if let Err(e) = run_cli(&args) {
        eprintln!("loos-switch: {e}");
        std::process::exit(1);
    }
}

fn launch_gui(screenshot: Option<PathBuf>, page: Option<&str>) {
    let page_owned = page.map(str::to_string);
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/loos-switch.png")).ok();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1240.0, 820.0])
        .with_min_inner_size([980.0, 700.0])
        .with_app_id("loos-switch")
        .with_title("LoOS Switch — local-only installer & migration");
    if let Some(icon) = icon {
        if screenshot.is_none() {
            viewport = viewport.with_icon(icon);
        }
    }
    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    if let Err(e) = eframe::run_native(
        "LoOS Switch",
        native_options,
        Box::new(move |cc| {
            let app = match &screenshot {
                Some(path) => app::LoosSwitchApp::new_capture(cc, path.clone(), page_owned.as_deref()),
                None => app::LoosSwitchApp::new(cc),
            };
            Ok(Box::new(app))
        }),
    ) {
        eprintln!("cannot start the GUI: {e}\nUse --help for the headless/power-user commands.");
        std::process::exit(1);
    }
}

const HELP: &str = r#"LoOS Switch — local-only LoOS installer and migration manager

GUI (default):
  loos-switch

Safe inspection:
  --help                       show this help
  --screenshot <path> [--screenshot-page home|install|migration|safety]
                               render one page to a PNG and exit (QA)
  --version                    print app and payload versions
  --self-test                  verify embedded payload + environment, write nothing
  --list-disks                 show disks with safety classification
  --extract-payload <uefi|bios> <path>
                               extract the raw installer image for inspection

Installer media (destroys the selected whole disk):
  --make-media --device <id|path> [--firmware uefi|bios] [--yes]
                               write verified LoOS installer media
  --no-verify                  skip readback verification (not recommended)

Local UEFI staging (does not repartition; adds a boot entry):
  --stage-uefi [--target-disk <id>] [--unattended --user <name>
               --password <temporary> --hostname <name>] [--yes]

Migration (copy-only, source is never modified):
  --backup <folder> [<folder> ...] --to <destination> [--user <name>]
  --no-verify                  skip SHA-256 verification
  --direct-home                destination is already a mounted LoOS home
  --dry-run                    measure and print the plan only

Global:
  --offline                    accepted for scripts; the app never goes online
"#;

fn run_cli(args: &[String]) -> Result<(), String> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{HELP}");
        return Ok(());
    }
    if args.iter().any(|a| a == "--version") {
        let p = payload::Payload::embedded().map_err(|e| e.to_string())?;
        println!("LoOS Switch {} (payload {}; UEFI {} bytes, BIOS {} bytes)", platform::APP_VERSION, p.manifest.built_utc, p.manifest.uefi.raw_bytes, p.manifest.bios.raw_bytes);
        return Ok(());
    }
    if args.iter().any(|a| a == "--list-disks") {
        let sys = platform::system_info().map_err(|e| e.to_string())?;
        println!("{} · {} · {} · {} elevation", sys.os_name, sys.firmware.map(|f| f.label()).unwrap_or("firmware unknown"), if sys.elevated { "admin" } else { "standard user" }, if sys.secure_boot == Some(true) { "Secure Boot ON" } else { "Secure Boot off/unknown" });
        for d in platform::scan_disks().map_err(|e| e.to_string())? {
            println!(
                "{}  id={}  {:.1} GiB  {}  removable={} system={} mounts={} safe={}",
                d.device_path,
                d.id,
                d.size_gib(),
                d.friendly_name(),
                d.is_usb(),
                d.system_disk,
                d.mount_count(),
                d.is_safe_write_target()
            );
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--self-test") {
        let sys = platform::system_info().map_err(|e| e.to_string())?;
        println!("system: {} ({}), firmware {:?}, elevated {}", sys.os_name, sys.kernel, sys.firmware, sys.elevated);
        println!("telemetry: none (no network client linked)");
        let cancel = AtomicBool::new(false);
        for line in install::payload_self_test(&cancel).map_err(|e| e.to_string())? {
            println!("payload: {line}");
        }
        println!("payload self-test: OK");
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--extract-payload") {
        let fw = match args.get(i + 1).map(String::as_str) {
            Some("uefi") => Firmware::Uefi,
            Some("bios") => Firmware::Bios,
            _ => return Err("--extract-payload needs uefi or bios".into()),
        };
        let out = args.get(i + 2).ok_or("--extract-payload needs an output path")?;
        payload::extract_payload_to(std::path::Path::new(out), fw).map_err(|e| e.to_string())?;
        println!("wrote {}", out);
        return Ok(());
    }
    if args.iter().any(|a| a == "--make-media") {
        return cli_make_media(args);
    }
    if args.iter().any(|a| a == "--stage-uefi") {
        return cli_stage_uefi(args);
    }
    if args.iter().any(|a| a == "--backup") {
        return cli_backup(args);
    }
    Err("unknown command. Run --help.".into())
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}
fn value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn cli_make_media(args: &[String]) -> Result<(), String> {
    let sys = platform::system_info().map_err(|e| e.to_string())?;
    let disks = platform::scan_disks().map_err(|e| e.to_string())?;
    let wanted = value(args, "--device").ok_or("--make-media needs --device <id|path>")?;
    let disk = disks
        .iter()
        .find(|d| d.id == wanted || d.device_path == wanted || d.device_path.ends_with(wanted))
        .cloned()
        .ok_or_else(|| format!("no disk matches {wanted}"))?;
    if !disk.is_safe_write_target() {
        return Err(format!("{} is not a safe target (system={}, mounted partitions={}, ro={})", disk.device_path, disk.system_disk, disk.mount_count(), disk.read_only));
    }
    let fw = match value(args, "--firmware") {
        Some("bios") => Firmware::Bios,
        Some("uefi") => Firmware::Uefi,
        None => sys.firmware.unwrap_or(Firmware::Uefi),
        Some(x) => return Err(format!("unknown firmware {x}")),
    };
    if !flag(args, "--yes") {
        return Err(format!("refusing to overwrite {} without --yes (target: {})", disk.device_path, disk.identity_line()));
    }
    let mut plan = InstallPlan::fresh(fw, InstallMethod::InstallMedia);
    plan.target_disk = Some(disk);
    plan.verify_readback = !flag(args, "--no-verify");
    plan.reboot_when_done = flag(args, "--reboot");
    if !sys.elevated {
        return Err("root/administrator permission is required for raw disk writes".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let mut emit = |e: InstallEvent| match e {
        InstallEvent::Progress { phase, done, total, detail } => {
            if total > 0 {
                eprint!("\r{phase}: {:5.1}% {detail}          ", done as f64 * 100.0 / total as f64);
            }
        }
        InstallEvent::Log(l) => eprintln!("\n{l}"),
        InstallEvent::Finished { summary } => eprintln!("\n{summary}"),
    };
    let outcome = install::execute(&plan, &sys, &cancel, &mut emit).map_err(|e| e.to_string())?;
    println!("\n{}", outcome.summary);
    Ok(())
}

fn cli_stage_uefi(args: &[String]) -> Result<(), String> {
    let sys = platform::system_info().map_err(|e| e.to_string())?;
    let disks = platform::scan_disks().map_err(|e| e.to_string())?;
    let target = value(args, "--target-disk")
        .and_then(|id| disks.iter().find(|d| d.id == id || d.device_path == id).cloned());
    let mut plan = InstallPlan::fresh(Firmware::Uefi, InstallMethod::LocalSetup);
    plan.target_disk = target;
    plan.esp_mount = sys.esp_mount.clone();
    plan.unattended = flag(args, "--unattended");
    plan.admin_user = value(args, "--user").unwrap_or("user").into();
    plan.admin_password = value(args, "--password").unwrap_or("").into();
    plan.computer_name = value(args, "--hostname").unwrap_or("loos-pc").into();
    plan.boot_label = value(args, "--label").unwrap_or("LoOS Setup").into();
    if plan.unattended {
        let target = plan.target_disk.as_ref().ok_or("--unattended needs --target-disk <id>")?;
        if target.system_disk {
            return Err("unattended target is the running system disk".into());
        }
        if !flag(args, "--yes") {
            return Err("refusing unattended install without --yes".into());
        }
    }
    if !sys.elevated {
        return Err("root/administrator permission is required to write the ESP".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let mut emit = |e: InstallEvent| {
        if let InstallEvent::Log(l) = e {
            eprintln!("{l}");
        }
    };
    let outcome = install::execute(&plan, &sys, &cancel, &mut emit).map_err(|e| e.to_string())?;
    println!("{}", outcome.summary);
    Ok(())
}

fn cli_backup(args: &[String]) -> Result<(), String> {
    let mut sources = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--backup" {
            i += 1;
            while i < args.len() && !args[i].starts_with("--") {
                sources.push(PathBuf::from(&args[i]));
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    if sources.is_empty() {
        return Err("--backup needs at least one folder".into());
    }
    let dest = PathBuf::from(value(args, "--to").ok_or("--backup needs --to <destination>")?);
    let user = value(args, "--user").unwrap_or("user").to_string();
    if !model::valid_login_name(&user) {
        return Err("--user must be a valid LoOS login name".into());
    }
    let opts = MigrationOptions { verify_hashes: !flag(args, "--no-verify"), include_hidden: !flag(args, "--no-hidden"), ..Default::default() };
    let cancel = AtomicBool::new(false);
    let mut ms = Vec::new();
    let mut total = 0u64;
    for s in &sources {
        let (files, bytes) = migration::measure_source(s, &opts, &cancel, |_, _, _| {}).map_err(|e| e.to_string())?;
        total += bytes;
        ms.push(MigrationSource { path: s.clone(), destination: s.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".into()), selected: true, size_bytes: Some(bytes), file_count: Some(files) });
    }
    let free = std::fs::create_dir_all(&dest).ok().and_then(|_| fs2::available_space(&dest).ok()).unwrap_or(0);
    println!("Migration plan: {} source(s), {} files total, {}; destination {} ({} free)", sources.len(), ms.iter().filter_map(|s| s.file_count).sum::<u64>(), platform::human_bytes(total), dest.display(), platform::human_bytes(free));
    if flag(args, "--dry-run") {
        return Ok(());
    }
    if total > free && free > 0 {
        return Err("not enough free space at the destination".into());
    }
    let plan = model::MigrationPlan { sources: ms, destination_root: dest, target_home: user, options: opts, free_bytes: free, total_bytes: total, direct_to_home: flag(args, "--direct-home") };
    let manifest = migration::execute(&plan, &cancel, |p| eprintln!("\r{}/{} · {} files    ", platform::human_bytes(p.bytes_done), platform::human_bytes(p.bytes_total), p.files_done)).map_err(|e| e.to_string())?;
    println!("\nMigration complete: {} files, {} ({} errors)", manifest.stats.files_copied, platform::human_bytes(manifest.stats.bytes_copied), manifest.stats.errors.len());
    Ok(())
}
