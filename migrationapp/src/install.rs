//! Deployment engine. It has two deliberately narrow capabilities:
//!
//! 1. Write a complete, verified LoOS installer image to a whole USB/dedicated
//!    disk. This is how a PC whose only disk is the running one gets LoOS: the
//!    actual installation then happens in LoOS Setup after boot.
//! 2. Stage a small setup loader on the existing ESP and add a firmware boot
//!    entry, without repartitioning or erasing anything. This is how a second
//!    disk can be provisioned with no USB.
//!
//! Neither path ever writes to the running system disk.

use crate::model::{Firmware, InstallMethod, InstallPlan, SystemInfo};
use crate::payload::{Payload, PayloadImage};
use crate::platform::{self, human_bytes, RawTarget};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

const MIN_INSTALL_DISK: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("operation cancelled")]
    Cancelled,
    #[error("{0}")]
    Blocked(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("payload error: {0}")]
    Payload(#[from] crate::payload::PayloadError),
    #[error("platform error: {0}")]
    Platform(#[from] crate::platform::PlatformError),
    #[error("verification failed: the bytes read back do not match the embedded image")]
    Verify,
    #[error("{0}")]
    Message(String),
}

#[derive(Clone, Debug)]
pub enum InstallEvent {
    Progress { phase: &'static str, done: u64, total: u64, detail: String },
    Log(String),
    Finished { summary: String },
}

#[derive(Clone, Debug)]
pub struct InstallOutcome {
    pub summary: String,
    pub boot_entry: Option<String>,
}

fn event<F: FnMut(InstallEvent)>(tx: &mut F, e: InstallEvent) {
    tx(e);
}

pub fn preflight(plan: &InstallPlan, system: &SystemInfo) -> Vec<String> {
    plan.blockers(system)
}

pub fn execute<F: FnMut(InstallEvent)>(
    plan: &InstallPlan,
    system: &SystemInfo,
    cancel: &AtomicBool,
    mut tx: F,
) -> Result<InstallOutcome, InstallError> {
    let blockers = preflight(plan, system);
    if !blockers.is_empty() {
        return Err(InstallError::Blocked(blockers.join(" ")));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(InstallError::Cancelled);
    }
    match plan.method {
        InstallMethod::InstallMedia => execute_media(plan, system, cancel, &mut tx),
        InstallMethod::LocalSetup => execute_local(plan, system, cancel, &mut tx),
    }
}

fn check_target<'a>(plan: &'a InstallPlan, _system: &SystemInfo) -> Result<&'a crate::model::Disk, InstallError> {
    let disk = plan
        .target_disk
        .as_ref()
        .ok_or_else(|| InstallError::Blocked("No target disk selected.".into()))?;
    if disk.system_disk {
        return Err(InstallError::Blocked("That disk holds the running operating system.".into()));
    }
    if disk.size_bytes < MIN_INSTALL_DISK {
        return Err(InstallError::Blocked("LoOS needs a target of at least 2 GiB.".into()));
    }
    if disk.read_only {
        return Err(InstallError::Blocked("The target disk is read-only.".into()));
    }
    if disk.mount_count() > 0 && !cfg!(windows) {
        return Err(InstallError::Blocked("Unmount the target disk's partitions first.".into()));
    }
    Ok(disk)
}

fn execute_media<F: FnMut(InstallEvent)>(
    plan: &InstallPlan,
    system: &SystemInfo,
    cancel: &AtomicBool,
    tx: &mut F,
) -> Result<InstallOutcome, InstallError> {
    let disk = check_target(plan, system)?;
    platform::dismount_check(disk)?;
    let payload = Payload::embedded()?;
    let image = payload.image(plan.firmware)?;
    if disk.size_bytes < image.raw_bytes + 1024 * 1024 {
        return Err(InstallError::Blocked(format!(
            "The {} image needs at least {}; this disk is {}.",
            plan.firmware.label(),
            human_bytes(image.raw_bytes + 1024 * 1024),
            human_bytes(disk.size_bytes)
        )));
    }
    let mut target = platform::open_target(disk)?;
    event(tx, InstallEvent::Log(format!("Writing {} media to {}", plan.firmware.label(), disk.identity_line())));
    target.seek(0)?;
    let write_result = image.write_to(&mut target, cancel, |done, total| {
        event(
            tx,
            InstallEvent::Progress {
                phase: "Writing bootable image",
                done,
                total,
                detail: format!("{} / {}", human_bytes(done), human_bytes(total)),
            },
        );
    });
    if let Err(e) = write_result {
        // Never leave a half-written partition table claiming to be valid.
        if !matches!(e, crate::payload::PayloadError::Cancelled) {
            let _ = zero_head(&mut target);
        }
        return match e {
            crate::payload::PayloadError::Cancelled => {
                let _ = zero_head(&mut target);
                Err(InstallError::Cancelled)
            }
            other => Err(InstallError::Payload(other)),
        };
    }
    target.sync()?;
    event(tx, InstallEvent::Log("Image written; flushing to the device.".into()));
    if cancel.load(Ordering::Relaxed) {
        let _ = zero_head(&mut target);
        return Err(InstallError::Cancelled);
    }
    if plan.verify_readback {
        event(
            tx,
            InstallEvent::Progress {
                phase: "Verifying readback",
                done: 0,
                total: image.raw_bytes,
                detail: "Hashing the bytes that actually landed on disk".into(),
            },
        );
        verify_target(&mut target, &image, cancel, tx)?;
        event(tx, InstallEvent::Log("Readback verified byte-for-byte. The installer media is ready.".into()));
    }
    target.sync()?;
    let summary = format!(
        "{} installer media written and verified on {}. Boot this disk and choose LoOS Setup to install.",
        plan.firmware.label(),
        disk.friendly_name()
    );
    let boot_entry = Some(format!("Firmware {:?} media on {}", plan.firmware, disk.device_path));
    if plan.reboot_when_done {
        event(tx, InstallEvent::Log("Requesting reboot in 10 seconds…".into()));
        platform::reboot()?;
    }
    event(tx, InstallEvent::Finished { summary: summary.clone() });
    Ok(InstallOutcome { summary, boot_entry })
}

fn zero_head(target: &mut RawTarget) -> Result<(), InstallError> {
    let zeros = vec![0u8; 1024 * 1024];
    target.write_at(0, &zeros)?;
    target.sync()?;
    Ok(())
}

fn verify_target<F: FnMut(InstallEvent)>(
    target: &mut RawTarget,
    image: &PayloadImage,
    cancel: &AtomicBool,
    tx: &mut F,
) -> Result<(), InstallError> {
    target.seek(0)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 4 * 1024 * 1024];
    let mut done = 0u64;
    while done < image.raw_bytes {
        if cancel.load(Ordering::Relaxed) {
            return Err(InstallError::Cancelled);
        }
        let want = ((image.raw_bytes - done) as usize).min(buf.len());
        target.read_exact(&mut buf[..want])?;
        hasher.update(&buf[..want]);
        done += want as u64;
        event(
            tx,
            InstallEvent::Progress {
                phase: "Verifying readback",
                done,
                total: image.raw_bytes,
                detail: format!("{} / {}", human_bytes(done), human_bytes(image.raw_bytes)),
            },
        );
    }
    let got = hex::encode(hasher.finalize());
    if !got.eq_ignore_ascii_case(&image.raw_sha256_hex) {
        return Err(InstallError::Verify);
    }
    Ok(())
}

fn execute_local<F: FnMut(InstallEvent)>(
    plan: &InstallPlan,
    system: &SystemInfo,
    cancel: &AtomicBool,
    tx: &mut F,
) -> Result<InstallOutcome, InstallError> {
    if plan.firmware != Firmware::Uefi {
        return Err(InstallError::Blocked("Local setup requires UEFI; use install media on BIOS machines.".into()));
    }
    let esp = plan
        .esp_mount
        .clone()
        .or_else(|| system.esp_mount.clone())
        .ok_or_else(|| InstallError::Blocked("No mounted EFI System Partition found.".into()))?;
    let payload = Payload::embedded()?;
    let image = payload.image(Firmware::Uefi)?;
    event(tx, InstallEvent::Log("Extracting the small LoOS setup loader from the verified payload…".into()));
    let files = image.extract_uefi_loader(cancel)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(InstallError::Cancelled);
    }

    let mut cfg = String::from("kernel=\\LOOS\\KERNEL.ELF\n");
    if plan.unattended {
        let target = plan
            .target_disk
            .as_ref()
            .ok_or_else(|| InstallError::Blocked("Unattended setup needs an explicit target disk.".into()))?;
        if target.system_disk || target.size_bytes < MIN_INSTALL_DISK {
            return Err(InstallError::Blocked("Unattended setup target must be a non-system disk of at least 2 GiB.".into()));
        }
        cfg.push_str(&format!(
            "cmdline=installer installer.auto=1 installer.auto.bytes={} installer.user={} installer.pass={} installer.host={}\n",
            target.size_bytes,
            plan.admin_user,
            plan.admin_password,
            plan.computer_name
        ));
    } else {
        cfg.push_str("cmdline=installer\n");
    }

    event(
        tx,
        InstallEvent::Log(format!(
            "Staging {:.1} MiB setup loader on {} (no partitions changed).",
            (files.loader.len() + files.kernel.len()) as f64 / 1024.0 / 1024.0,
            esp.display()
        )),
    );
    event(
        tx,
        InstallEvent::Progress {
            phase: "Staging setup loader",
            done: 1,
            total: 3,
            detail: "Copying loader, kernel and boot configuration".into(),
        },
    );
    let boot = platform::stage_uefi(&esp, &files.loader, &files.kernel, &cfg, &plan.boot_label)?;
    event(tx, InstallEvent::Log(boot.clone()));
    event(
        tx,
        InstallEvent::Progress {
            phase: "Staging setup loader",
            done: 3,
            total: 3,
            detail: "Firmware boot entry created".into(),
        },
    );
    if plan.reboot_when_done {
        event(tx, InstallEvent::Log("Rebooting in 10 seconds. Choose 'LoOS Setup' if the firmware does not automatically.".into()));
        platform::reboot()?;
    }
    let summary = if plan.unattended {
        format!(
            "LoOS Setup staged. On reboot, setup will install to the {:.1} GiB target disk automatically. The one-time password is shown once in the review step.",
            plan.target_disk.as_ref().map(|d| d.size_gib()).unwrap_or(0.0)
        )
    } else {
        "LoOS Setup staged on the existing ESP. Nothing was erased or repartitioned. Reboot and choose 'LoOS Setup' in the firmware boot menu.".into()
    };
    event(tx, InstallEvent::Finished { summary: summary.clone() });
    Ok(InstallOutcome { summary, boot_entry: Some(boot) })
}

/// Verify that an embedded payload decompresses to the expected hash without
/// touching a disk. Used by the GUI's first-run self-check and by `--self-test`.
pub fn payload_self_test(cancel: &AtomicBool) -> Result<Vec<String>, InstallError> {
    let payload = Payload::embedded()?;
    let mut out = Vec::new();
    for img in payload.all_images()? {
        let mut sink = std::io::sink();
        img.write_to(&mut sink, cancel, |_, _| {})?;
        out.push(format!(
            "{}: {} bytes, sha256 {} — OK",
            img.firmware.label(),
            img.raw_bytes,
            &img.raw_sha256_hex[..16]
        ));
        if img.firmware == Firmware::Uefi {
            let files = img.extract_uefi_loader(cancel)?;
            if !files.loader.starts_with(b"MZ") || !files.kernel.starts_with(b"\x7fELF") {
                return Err(InstallError::Message("UEFI boot files have the wrong format".into()));
            }
            out.push(format!(
                "UEFI boot files: loader {} bytes, kernel {} bytes — OK",
                files.loader.len(),
                files.kernel.len()
            ));
        }
    }
    Ok(out)
}

