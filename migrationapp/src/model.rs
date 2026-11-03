//! Shared, serializable models. The GUI and the headless CLI use exactly the
//! same plan types, so a plan shown on screen is the plan that gets executed.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Firmware {
    Uefi,
    Bios,
}

impl Firmware {
    pub fn label(self) -> &'static str {
        match self {
            Firmware::Uefi => "UEFI",
            Firmware::Bios => "Legacy BIOS",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsFamily {
    Linux,
    Windows,
}

impl OsFamily {
    pub fn label(self) -> &'static str {
        match self {
            OsFamily::Linux => "Linux",
            OsFamily::Windows => "Windows",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Partition {
    pub number: u32,
    pub path: String,
    pub size_bytes: u64,
    pub fstype: Option<String>,
    pub mountpoints: Vec<String>,
    pub is_esp: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Disk {
    /// Stable across a session; `\\.\PhysicalDriveN` or `/dev/sdX`.
    pub id: String,
    pub device_path: String,
    pub model: String,
    pub serial: String,
    pub size_bytes: u64,
    pub bus: String,
    pub removable: bool,
    pub read_only: bool,
    pub system_disk: bool,
    pub partition_style: String,
    pub partitions: Vec<Partition>,
}

impl Disk {
    pub fn friendly_name(&self) -> String {
        if self.model.trim().is_empty() {
            self.device_path.clone()
        } else {
            self.model.clone()
        }
    }

    pub fn size_gib(&self) -> f64 {
        self.size_bytes as f64 / 1024.0 / 1024.0 / 1024.0
    }

    pub fn mount_count(&self) -> usize {
        self.partitions.iter().map(|p| p.mountpoints.len()).sum()
    }

    pub fn is_usb(&self) -> bool {
        self.removable || self.bus.eq_ignore_ascii_case("usb")
    }

    /// Conservative guard: never write the running OS disk. On Linux we also
    /// require the target to be unmounted; on Windows the engine locks and
    /// dismounts volume handles as part of the explicit write.
    pub fn is_safe_write_target(&self) -> bool {
        if self.system_disk || self.read_only {
            return false;
        }
        #[cfg(target_os = "linux")]
        {
            self.mount_count() == 0
        }
        #[cfg(target_os = "windows")]
        {
            true
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            false
        }
    }

    pub fn identity_line(&self) -> String {
        let serial = if self.serial.is_empty() { "no serial".into() } else { self.serial.clone() };
        format!("{} · {:.1} GiB · {} · {}", self.friendly_name(), self.size_gib(), self.device_path, serial)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemInfo {
    pub os: OsFamily,
    pub os_name: String,
    pub kernel: String,
    pub firmware: Option<Firmware>,
    pub secure_boot: Option<bool>,
    pub elevated: bool,
    pub esp_mount: Option<PathBuf>,
    pub home: PathBuf,
    pub user: String,
    pub hostname: String,
    pub total_memory_bytes: Option<u64>,
    pub battery_percent: Option<u8>,
    pub on_ac_power: Option<bool>,
    pub bitlocker_suspected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallMethod {
    /// Write a verified LoOS installer image to a whole USB/disk.
    InstallMedia,
    /// Put a small, non-destructive LoOS setup loader on the existing ESP and
    /// add a one-shot firmware boot entry. Never erases or repartitions.
    LocalSetup,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallPlan {
    pub method: InstallMethod,
    pub firmware: Firmware,
    pub target_disk: Option<Disk>,
    pub esp_mount: Option<PathBuf>,
    pub boot_label: String,
    pub keep_boot_order: bool,
    pub reboot_when_done: bool,
    pub unattended: bool,
    pub verify_readback: bool,
    /// User-account details are only used by the local unattended path. The
    /// normal LoOS Setup wizard collects them itself.
    pub admin_user: String,
    pub admin_password: String,
    pub computer_name: String,
    pub migration_bundle: Option<PathBuf>,
}

impl InstallPlan {
    pub fn fresh(firmware: Firmware, method: InstallMethod) -> Self {
        Self {
            method,
            firmware,
            target_disk: None,
            esp_mount: None,
            boot_label: "LoOS Setup".into(),
            keep_boot_order: true,
            reboot_when_done: false,
            unattended: false,
            verify_readback: true,
            admin_user: String::new(),
            admin_password: String::new(),
            computer_name: "loos-pc".into(),
            migration_bundle: None,
        }
    }

    /// Reasons this exact plan cannot be executed. Empty means ready.
    pub fn blockers(&self, system: &SystemInfo) -> Vec<String> {
        let mut out = Vec::new();
        if !system.elevated {
            out.push("Administrator/root permission is required for disk operations.".into());
        }
        match self.method {
            InstallMethod::InstallMedia => {
                let Some(d) = &self.target_disk else {
                    out.push("Choose a USB drive or dedicated disk.".into());
                    return out;
                };
                if d.read_only {
                    out.push("The selected target is read-only.".into());
                }
                if d.system_disk {
                    out.push("The selected disk contains the running operating system. Refusing to overwrite it.".into());
                }
                #[cfg(target_os = "linux")]
                if d.mount_count() > 0 {
                    out.push("The selected disk has mounted partitions. Unmount it first.".into());
                }
                if d.size_bytes < 2 * 1024 * 1024 * 1024 {
                    out.push("LoOS needs a target of at least 2 GiB.".into());
                }
            }
            InstallMethod::LocalSetup => {
                if self.firmware != Firmware::Uefi {
                    out.push("Local setup staging currently requires UEFI. Use install media for BIOS machines.".into());
                }
                if system.secure_boot == Some(true) {
                    out.push("Secure Boot is on and this LoOS build is unsigned. Turn Secure Boot off or use a signed build.".into());
                }
                let Some(esp) = &self.esp_mount else {
                    out.push("No mounted EFI System Partition was found.".into());
                    return out;
                };
                if !esp.exists() {
                    out.push("The EFI System Partition mount point does not exist.".into());
                }
                if !self.unattended {
                    // interactive setup is fine with no extra fields
                    return out;
                }
                if self.admin_user.trim().is_empty() {
                    out.push("Unattended setup needs a user name.".into());
                }
                if !valid_login_name(&self.admin_user) {
                    out.push("User name must start with a-z and contain only a-z, 0-9, '-' or '_'.".into());
                }
                if self.admin_password.len() < 8 {
                    out.push("Unattended password must be at least 8 characters.".into());
                }
                if !valid_hostname(&self.computer_name) {
                    out.push("Computer name must be 1–31 letters, digits or '-'.".into());
                }
            }
        }
        out
    }
}

pub fn valid_login_name(s: &str) -> bool {
    let bytes = s.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 31
        && bytes[0].is_ascii_lowercase()
        && bytes.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-')
        && s != "root"
}

pub fn valid_hostname(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 31
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.as_bytes().iter().all(|b| b.is_ascii_alphanumeric() || *b == b'-')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    Skip,
    Overwrite,
    Rename,
}

impl ConflictPolicy {
    pub fn label(self) -> &'static str {
        match self {
            ConflictPolicy::Skip => "Skip existing files",
            ConflictPolicy::Overwrite => "Replace existing files",
            ConflictPolicy::Rename => "Keep both (add a suffix)",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigrationOptions {
    pub include_hidden: bool,
    pub follow_symlinks: bool,
    pub verify_hashes: bool,
    pub preserve_timestamps: bool,
    pub conflict: ConflictPolicy,
    /// Directory names skipped anywhere in a tree (case-insensitive).
    pub excluded_names: Vec<String>,
}

impl Default for MigrationOptions {
    fn default() -> Self {
        Self {
            include_hidden: true,
            follow_symlinks: false,
            verify_hashes: true,
            preserve_timestamps: true,
            conflict: ConflictPolicy::Skip,
            excluded_names: vec![
                "node_modules".into(),
                "__pycache__".into(),
                ".cache".into(),
                "Cache".into(),
                "cache2".into(),
                "Code Cache".into(),
                "GPUCache".into(),
                "Crashpad".into(),
                "tmp".into(),
                "Temp".into(),
                "$Recycle.Bin".into(),
                "System Volume Information".into(),
            ],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigrationSource {
    pub path: PathBuf,
    /// Where this folder lands inside the LoOS home. Example: `Documents`.
    pub destination: String,
    pub selected: bool,
    pub size_bytes: Option<u64>,
    pub file_count: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub sources: Vec<MigrationSource>,
    pub destination_root: PathBuf,
    pub target_home: String,
    pub options: MigrationOptions,
    pub free_bytes: u64,
    pub total_bytes: u64,
    /// When true, `destination_root` is already a mounted LoOS home and files
    /// are copied straight into it instead of a portable bundle tree.
    #[serde(default)]
    pub direct_to_home: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CopyStats {
    pub files_copied: u64,
    pub files_skipped: u64,
    pub bytes_copied: u64,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigrationManifest {
    pub format: u32,
    pub created_utc: String,
    pub app: String,
    pub app_version: String,
    pub target_home: String,
    pub options: MigrationOptions,
    pub sources: Vec<SourceRecord>,
    pub stats: CopyStats,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRecord {
    pub source: PathBuf,
    pub destination: String,
    pub bytes: u64,
    pub files: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigrationEntry {
    pub source: PathBuf,
    pub relative: PathBuf,
    pub destination: PathBuf,
    pub size: u64,
    pub modified_unix: Option<u64>,
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_and_hostname_rules() {
        assert!(valid_login_name("silvan"));
        assert!(valid_login_name("a_b-9"));
        assert!(!valid_login_name("Bob"));
        assert!(!valid_login_name("root"));
        assert!(!valid_login_name("a b"));
        assert!(!valid_login_name("../x"));
        assert!(valid_hostname("loos-pc"));
        assert!(!valid_hostname("-bad"));
        assert!(!valid_hostname("bad-"));
        assert!(!valid_hostname("has space"));
    }
}
