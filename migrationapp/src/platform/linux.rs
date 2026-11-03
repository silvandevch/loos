use super::{PlatformError, RawTarget};
use crate::model::{Disk, Firmware, OsFamily, Partition, SystemInfo};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::process::Command;

fn out(cmd: &mut Command) -> Result<String, PlatformError> {
    let o = cmd.output().map_err(PlatformError::Io)?;
    if !o.status.success() {
        return Err(PlatformError::Message(format!(
            "{} failed: {}",
            cmd.get_program().to_string_lossy(),
            String::from_utf8_lossy(&o.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

fn ps(name: &str) -> String {
    if name.is_empty() { "Unknown".into() } else { name.to_string() }
}

fn lsblk_json() -> Result<Value, PlatformError> {
    let text = out(Command::new("lsblk").args([
        "--json",
        "-b",
        "-o",
        "NAME,PATH,TYPE,SIZE,MODEL,SERIAL,TRAN,RM,RO,MOUNTPOINTS,FSTYPE,PTTYPE,PARTTYPE",
    ]))?;
    serde_json::from_str(&text).map_err(|e| PlatformError::Message(format!("cannot parse lsblk: {e}")))
}

fn strval(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

fn boolval(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn mounts(v: &Value) -> Vec<String> {
    v.get("mountpoints")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.is_empty() && !s.starts_with('['))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn part_is_esp(v: &Value, fstype: &str, mps: &[String]) -> bool {
    let pt = strval(v, "parttype").to_ascii_lowercase();
    pt == "c12a7328-f81f-11d2-ba4b-00a0c93ec93b"
        || mps.iter().any(|m| m == "/boot/efi" || m == "/efi")
        || (fstype == "vfat" && mps.iter().any(|m| m.starts_with("/boot")))
}

pub fn scan_disks() -> Result<Vec<Disk>, PlatformError> {
    let root = lsblk_json()?;
    let Some(devs) = root.get("blockdevices").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for d in devs {
        let ty = strval(d, "type");
        if ty != "disk" {
            continue;
        }
        let name = strval(d, "name");
        if name.starts_with("zram") || name.starts_with("loop") || name.starts_with("ram") || name.starts_with("sr") {
            continue;
        }
        let mut partitions = Vec::new();
        let mut system = false;
        let mut mounts_any = Vec::new();
        if let Some(kids) = d.get("children").and_then(Value::as_array) {
            for (idx, p) in kids.iter().enumerate() {
                let ptype = strval(p, "type");
                if ptype != "part" {
                    continue;
                }
                let mps = mounts(p);
                let fstype = strval(p, "fstype");
                if mps
                    .iter()
                    .any(|m| m == "/" || m == "/boot" || m == "/boot/efi" || m == "/usr" || m == "/var" || m == "/home")
                {
                    system = true;
                }
                mounts_any.extend(mps.clone());
                partitions.push(Partition {
                    number: (idx + 1) as u32,
                    path: strval(p, "path"),
                    size_bytes: p.get("size").and_then(Value::as_u64).unwrap_or(0),
                    fstype: if fstype.is_empty() { None } else { Some(fstype) },
                    mountpoints: mps,
                    is_esp: p.get("parttype").and_then(Value::as_str).map(|s| s.eq_ignore_ascii_case("c12a7328-f81f-11d2-ba4b-00a0c93ec93b")).unwrap_or(false)
                        || part_is_esp(p, &strval(p, "fstype"), &mounts(p)),
                });
            }
        }
        // A mounted filesystem on a non-partitioned disk is also unsafe.
        if mounts(d).iter().any(|m| m == "/" || m == "/boot") {
            system = true;
        }
        let tran = strval(d, "tran");
        let removable = boolval(d, "rm") || tran.eq_ignore_ascii_case("usb");
        out.push(Disk {
            id: name.clone(),
            device_path: strval(d, "path"),
            model: ps(&strval(d, "model")).replace("Unknown", ""),
            serial: strval(d, "serial"),
            size_bytes: d.get("size").and_then(Value::as_u64).unwrap_or(0),
            bus: tran,
            removable,
            read_only: boolval(d, "ro"),
            system_disk: system,
            partition_style: strval(d, "pttype"),
            partitions,
        });
    }
    Ok(out)
}

fn read_efivar_secure_boot() -> Option<bool> {
    let p = Path::new("/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c");
    let data = fs::read(p).ok()?;
    data.get(4).map(|b| *b != 0)
}

fn os_name() -> String {
    fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("PRETTY_NAME="))
                .map(|l| l.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "Linux".into())
}

fn kernel() -> String {
    fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

fn meminfo() -> Option<u64> {
    let s = fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = s.lines().find(|l| l.starts_with("MemTotal:"))?.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

fn battery() -> (Option<u8>, Option<bool>) {
    for i in 0..8 {
        let base = format!("/sys/class/power_supply/BAT{i}");
        if !Path::new(&base).exists() {
            continue;
        }
        let pct = fs::read_to_string(format!("{base}/capacity")).ok().and_then(|s| s.trim().parse().ok());
        let status = fs::read_to_string(format!("{base}/status")).ok().unwrap_or_default();
        let ac = fs::read_to_string("/sys/class/power_supply/AC/online")
            .or_else(|_| fs::read_to_string("/sys/class/power_supply/ACAD/online"))
            .ok()
            .map(|s| s.trim() == "1");
        return (pct, ac.or(Some(status.trim() == "Charging" || status.trim() == "Full")));
    }
    (None, None)
}

fn esp_mount_from_mounts() -> Option<std::path::PathBuf> {
    let s = fs::read_to_string("/proc/mounts").ok()?;
    for line in s.lines() {
        let mut it = line.split_whitespace();
        let _dev = it.next()?;
        let mp = it.next()?;
        if mp == "/boot/efi" || mp == "/efi" {
            return Some(mp.into());
        }
    }
    None
}

pub fn system_info() -> Result<SystemInfo, PlatformError> {
    let uefi = Path::new("/sys/firmware/efi").exists();
    let disks = scan_disks().unwrap_or_default();
    let esp = esp_mount_from_mounts().or_else(|| super::find_esp_mount(&disks));
    let (battery_percent, on_ac_power) = battery();
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    let hostname = fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "localhost".into());
    Ok(SystemInfo {
        os: OsFamily::Linux,
        os_name: os_name(),
        kernel: kernel(),
        firmware: Some(if uefi { Firmware::Uefi } else { Firmware::Bios }),
        secure_boot: if uefi { read_efivar_secure_boot() } else { None },
        elevated: super::is_elevated(),
        esp_mount: esp,
        home: dirs::home_dir().unwrap_or_else(|| Path::new("/").into()),
        user,
        hostname,
        total_memory_bytes: meminfo(),
        battery_percent,
        on_ac_power,
        bitlocker_suspected: false,
    })
}

pub fn open_target(disk: &Disk) -> Result<RawTarget, PlatformError> {
    if disk.system_disk {
        return Err(PlatformError::Message("refusing to open the running system disk".into()));
    }
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&disk.device_path)
        .map_err(|e| PlatformError::Message(format!("cannot open {} for writing: {e}", disk.device_path)))?;
    Ok(RawTarget { file: f })
}

pub fn dismount_check(disk: &Disk) -> Result<(), PlatformError> {
    if disk.mount_count() > 0 {
        return Err(PlatformError::Message(format!(
            "{} has mounted partitions. Unmount them first.",
            disk.device_path
        )));
    }
    Ok(())
}

pub fn reboot() -> Result<(), PlatformError> {
    let _ = Command::new("sync").status();
    Command::new("systemctl")
        .arg("reboot")
        .status()
        .map_err(|e| PlatformError::Message(format!("could not request reboot: {e}")))?;
    Ok(())
}

pub fn elevate_self() -> Result<(), PlatformError> {
    let exe = std::env::current_exe().map_err(PlatformError::Io)?;
    let display = std::env::var("DISPLAY").unwrap_or_default();
    let wayland = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    let xauth = std::env::var("XAUTHORITY").unwrap_or_default();
    let mut c = Command::new("pkexec");
    c.arg("env").arg(format!("DISPLAY={display}")).arg(format!("WAYLAND_DISPLAY={wayland}"));
    if !xauth.is_empty() {
        c.arg(format!("XAUTHORITY={xauth}"));
    }
    c.arg(exe);
    let st = c.status().map_err(|e| PlatformError::Message(format!("cannot start pkexec: {e}")))?;
    if st.success() {
        std::process::exit(0);
    }
    Err(PlatformError::Message("administrator elevation was cancelled or failed".into()))
}

/// Locate the block device and partition number backing a mountpoint.
fn mount_source(mountpoint: &Path) -> Option<(String, u32)> {
    let text = out(Command::new("findmnt").args(["-n", "-o", "SOURCE", "--target", &mountpoint.to_string_lossy()])).ok()?;
    let dev = text.trim().to_string();
    if dev.is_empty() {
        return None;
    }
    let name = Path::new(&dev).file_name()?.to_string_lossy().to_string();
    let part = fs::read_to_string(format!("/sys/class/block/{name}/partition")).ok()?.trim().parse().ok()?;
    // Strip the partition suffix to find the parent disk.
    let mut base = name.clone();
    if let Some(pos) = name.rfind('p') {
        if name[pos + 1..].chars().all(|c| c.is_ascii_digit()) {
            base = name[..pos].to_string();
        }
    } else {
        base = name.trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
    }
    let disk = format!("/dev/{base}");
    Some((disk, part))
}

pub fn stage_uefi(esp: &Path, loader: &[u8], kernel: &[u8], boot_cfg: &str, label: &str) -> Result<String, PlatformError> {
    let efi_dir = esp.join("EFI/LoOS");
    let loos_dir = esp.join("LOOS");
    fs::create_dir_all(&efi_dir)?;
    fs::create_dir_all(&loos_dir)?;
    fs::write(efi_dir.join("BOOTX64.EFI"), loader)?;
    fs::write(loos_dir.join("KERNEL.ELF"), kernel)?;
    fs::write(loos_dir.join("BOOT.CFG"), boot_cfg)?;
    let _ = Command::new("sync").status();
    let (disk, part) = mount_source(esp).ok_or_else(|| PlatformError::Message("cannot identify the ESP's parent disk".into()))?;
    // Replace an older entry with the same label instead of piling up entries.
    if let Ok(text) = out(&mut Command::new("efibootmgr")) {
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("Boot") {
                if let Some((num, desc)) = rest.split_once('*') {
                    let desc = desc.trim();
                    if desc.starts_with(label) {
                        let _ = Command::new("efibootmgr").args(["-b", num.trim(), "-B"]).status();
                    }
                }
            }
        }
    }
    let st = Command::new("efibootmgr")
        .args([
            "-c",
            "-d",
            &disk,
            "-p",
            &part.to_string(),
            "-L",
            label,
            "-l",
            r"\EFI\LoOS\BOOTX64.EFI",
        ])
        .status()
        .map_err(|e| PlatformError::Message(format!("efibootmgr not available: {e}")))?;
    if !st.success() {
        return Err(PlatformError::Message("efibootmgr could not create the boot entry. Check firmware NVRAM permissions.".into()));
    }
    Ok(format!("UEFI boot entry '{label}' created for {disk} partition {part}."))
}
