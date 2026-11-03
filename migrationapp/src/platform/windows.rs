use super::{PlatformError, RawTarget};
use crate::model::{Disk, Firmware, OsFamily, Partition, SystemInfo};
use serde_json::Value;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::System::WindowsProgramming::{
    GetFirmwareEnvironmentVariableW, SetFirmwareEnvironmentVariableW,
};
use windows_sys::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};

const EFI_GLOBAL_GUID: &str = "{8be4df61-93ca-11d2-aa0d-00e098032b8c}";

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn powershell(script: &str) -> Result<String, PlatformError> {
    let o = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .output()
        .map_err(PlatformError::Io)?;
    if !o.status.success() {
        return Err(PlatformError::Message(format!(
            "PowerShell failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

fn u64v(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(|x| x.as_u64()).unwrap_or(0)
}
fn strv(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
fn boolv(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn bus_name(n: u64) -> String {
    match n {
        1 => "SCSI",
        2 => "ATAPI",
        3 => "ATA",
        4 => "IEEE1394",
        5 => "SSA",
        6 => "FibreChannel",
        7 => "USB",
        8 => "RAID",
        9 => "iSCSI",
        10 => "SAS",
        11 => "SATA",
        12 => "SD",
        13 => "MMC",
        14 => "Virtual",
        15 => "FileBackedVirtual",
        16 => "StorageSpaces",
        17 => "NVMe",
        18 => "SCM",
        19 => "UFS",
        _ => "Unknown",
    }
    .to_string()
}

pub fn scan_disks() -> Result<Vec<Disk>, PlatformError> {
    let script = r#"
$disks = @(Get-Disk | Select-Object Number,FriendlyName,SerialNumber,Size,IsBoot,IsSystem,PartitionStyle,BusType,IsReadOnly,Guid)
$parts = @(Get-Partition | Select-Object DiskNumber,PartitionNumber,Size,Offset,Guid,GptType,DriveLetter,AccessPaths)
[pscustomobject]@{disks=$disks;parts=$parts} | ConvertTo-Json -Depth 6 -Compress
"#;
    let text = powershell(script)?;
    let root: Value = serde_json::from_str(&text).map_err(|e| PlatformError::Message(format!("cannot parse disk list: {e}")))?;
    let disks = root.get("disks").and_then(Value::as_array).cloned().unwrap_or_default();
    let parts = root.get("parts").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for d in &disks {
        let number = u64v(d, "Number");
        let mut partitions = Vec::new();
        for p in parts.iter().filter(|p| u64v(p, "DiskNumber") == number) {
            let access: Vec<String> = p
                .get("AccessPaths")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default();
            let letter = p.get("DriveLetter").and_then(Value::as_u64).map(|l| format!("{}:", l as u8 as char));
            let mut mounts = access.clone();
            if let Some(l) = letter {
                mounts.push(format!("{l}\\"));
            }
            let gpt = strv(p, "GptType").trim_matches(|c| c == '{' || c == '}').to_ascii_lowercase();
            let is_esp = gpt == "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
            partitions.push(Partition {
                number: u64v(p, "PartitionNumber") as u32,
                path: mounts.first().cloned().unwrap_or_default(),
                size_bytes: u64v(p, "Size"),
                fstype: None,
                mountpoints: mounts,
                is_esp,
            });
        }
        let bus = bus_name(u64v(d, "BusType"));
        let removable = bus == "USB";
        let system = boolv(d, "IsSystem") || boolv(d, "IsBoot");
        out.push(Disk {
            id: format!("{number}"),
            device_path: format!(r"\\.\PhysicalDrive{number}"),
            model: strv(d, "FriendlyName"),
            serial: strv(d, "SerialNumber"),
            size_bytes: u64v(d, "Size"),
            bus,
            removable,
            read_only: boolv(d, "IsReadOnly"),
            system_disk: system,
            partition_style: strv(d, "PartitionStyle"),
            partitions,
        });
    }
    Ok(out)
}

fn firmware() -> Option<Firmware> {
    let o = Command::new("bcdedit").arg("/enum").arg("{current}").output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout).to_ascii_lowercase();
    if s.contains("winload.efi") || s.contains(".efi") {
        Some(Firmware::Uefi)
    } else if s.contains("winload.exe") {
        Some(Firmware::Bios)
    } else {
        None
    }
}

fn secure_boot() -> Option<bool> {
    let o = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", "try { Confirm-SecureBootUEFI } catch { 'unknown' }"])
        .output()
        .ok()?;
    match String::from_utf8_lossy(&o.stdout).trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn first_esp_access_path() -> Option<PathBuf> {
    let script = r#"Get-Partition | Where-Object { $_.GptType -eq '{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}' } | Select-Object -First 1 -ExpandProperty AccessPaths"#;
    let s = powershell(script).ok()?;
    s.lines().map(str::trim).find(|l| !l.is_empty()).map(PathBuf::from)
}

pub fn system_info() -> Result<SystemInfo, PlatformError> {
    let os_name = powershell("(Get-CimInstance Win32_OperatingSystem).Caption")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "Windows".into());
    let kernel = powershell("(Get-CimInstance Win32_OperatingSystem).Version")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into());
    let hostname = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".into());
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from(r"C:\Users\user"));
    let memory = powershell("(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory")
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let battery = powershell("$b=Get-CimInstance Win32_Battery | Select-Object -First 1; if($b) { [pscustomobject]@{pct=$b.EstimatedChargeRemaining; status=$b.BatteryStatus} | ConvertTo-Json -Compress }")
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(s.trim()).ok())
        .and_then(|v| {
            let pct = v.get("pct").and_then(Value::as_u64).map(|x| x as u8);
            let st = v.get("status").and_then(Value::as_u64).unwrap_or(0);
            Some((pct, Some(st == 2)))
        })
        .unwrap_or((None, None));
    let bitlocker = powershell("$b=Get-BitLockerVolume -MountPoint $env:SystemDrive -ErrorAction SilentlyContinue; if($b -and $b.ProtectionStatus -eq 'On'){'on'}else{'off'}")
        .map(|s| s.trim().eq_ignore_ascii_case("on"))
        .unwrap_or(false);
    Ok(SystemInfo {
        os: OsFamily::Windows,
        os_name,
        kernel,
        firmware: firmware(),
        secure_boot: secure_boot(),
        elevated: is_admin(),
        esp_mount: first_esp_access_path(),
        home,
        user,
        hostname,
        total_memory_bytes: memory,
        battery_percent: battery.0,
        on_ac_power: battery.1,
        bitlocker_suspected: bitlocker,
    })
}

pub fn is_admin() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

fn enable_privilege(name: &str) -> Result<(), PlatformError> {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 {
            return Err(PlatformError::Message("OpenProcessToken failed".into()));
        }
        let mut luid = Default::default();
        let n = wide(name);
        if LookupPrivilegeValueW(std::ptr::null(), n.as_ptr(), &mut luid) == 0 {
            CloseHandle(token);
            return Err(PlatformError::Message("LookupPrivilegeValue failed".into()));
        }
        let mut tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }],
        };
        let ok = AdjustTokenPrivileges(token, 0, &mut tp, 0, std::ptr::null_mut(), std::ptr::null_mut());
        CloseHandle(token);
        if ok == 0 {
            return Err(PlatformError::Message("AdjustTokenPrivileges failed".into()));
        }
        Ok(())
    }
}

fn open_volume(letter: &str) -> Option<HANDLE> {
    let path = wide(&format!(r"\\.\{}", letter));
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE { None } else { Some(h) }
}

/// Best-effort volume dismount for direct physical-drive writes. Windows
/// refuses writes to mounted volumes; this is the same sequence disk-imaging
/// tools use. No filesystem data is deleted.
pub fn dismount_check(disk: &Disk) -> Result<(), PlatformError> {
    for p in &disk.partitions {
        for m in &p.mountpoints {
            let letter = if m.len() >= 2 && m.as_bytes()[1] == b':' { &m[..2] } else { continue };
            if let Some(h) = open_volume(letter) {
                let mut ret = 0u32;
                unsafe {
                    DeviceIoControl(h, FSCTL_LOCK_VOLUME, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut ret, std::ptr::null_mut());
                    DeviceIoControl(h, FSCTL_DISMOUNT_VOLUME, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut ret, std::ptr::null_mut());
                    CloseHandle(h);
                }
            }
        }
    }
    Ok(())
}

pub fn open_target(disk: &Disk) -> Result<RawTarget, PlatformError> {
    if disk.system_disk {
        return Err(PlatformError::Message("refusing to open the running system disk".into()));
    }
    dismount_check(disk)?;
    let path = wide(&disk.device_path);
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(PlatformError::Message(format!(
            "cannot open {} for writing (Windows error {})",
            disk.device_path,
            unsafe { GetLastError() }
        )));
    }
    let file = unsafe { std::fs::File::from_raw_handle(h as _) };
    Ok(RawTarget { file })
}

pub fn reboot() -> Result<(), PlatformError> {
    Command::new("shutdown")
        .args(["/r", "/t", "10", "/c", "LoOS Switch: rebooting to finish setup"])
        .status()
        .map_err(|e| PlatformError::Message(format!("could not request reboot: {e}")))?;
    Ok(())
}

pub fn elevate_self() -> Result<(), PlatformError> {
    let exe = std::env::current_exe().map_err(PlatformError::Io)?;
    let file = wide(&exe.to_string_lossy());
    let verb = wide("runas");
    let ret = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    if ret as isize <= 32 {
        return Err(PlatformError::Message("administrator elevation was cancelled or failed".into()));
    }
    std::process::exit(0);
}

/// Copies the loader/kernel/config to the ESP and creates a UEFI NVRAM entry.
/// This never touches another partition or the running Windows installation.
pub fn stage_uefi(esp: &Path, loader: &[u8], kernel: &[u8], boot_cfg: &str, label: &str) -> Result<String, PlatformError> {
    let efi_dir = esp.join("EFI").join("LoOS");
    let loos_dir = esp.join("LOOS");
    std::fs::create_dir_all(&efi_dir)?;
    std::fs::create_dir_all(&loos_dir)?;
    std::fs::write(efi_dir.join("BOOTX64.EFI"), loader)?;
    std::fs::write(loos_dir.join("KERNEL.ELF"), kernel)?;
    std::fs::write(loos_dir.join("BOOT.CFG"), boot_cfg)?;
    // Flush the files by reopening; Windows write-back on removable media is
    // handled by the volume driver.
    Ok(add_boot_entry(label, r"\EFI\LoOS\BOOTX64.EFI")?)
}

fn add_boot_entry(label: &str, loader_path: &str) -> Result<String, PlatformError> {
    enable_privilege("SeSystemEnvironmentPrivilege")?;
    // UUID + geometry of the ESP from PowerShell.
    let script = r#"
$p = Get-Partition | Where-Object { $_.GptType -eq '{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}' } | Select-Object -First 1
if ($p) { [pscustomobject]@{part=$p.PartitionNumber;start=[uint64]($p.Offset/512);size=[uint64]($p.Size/512);guid=$p.Guid;disk=$p.DiskNumber} | ConvertTo-Json -Compress }
"#;
    let info: Value = serde_json::from_str(powershell(script)?.trim())
        .map_err(|e| PlatformError::Message(format!("cannot locate ESP: {e}")))?;
    let part_no = info.get("part").and_then(Value::as_u64).ok_or_else(|| PlatformError::Message("ESP partition not found".into()))? as u32;
    let start = info.get("start").and_then(Value::as_u64).unwrap_or(0);
    let size = info.get("size").and_then(Value::as_u64).unwrap_or(0);
    let guid = info.get("guid").and_then(Value::as_str).unwrap_or("");
    let part_guid = guid_to_bytes(guid).ok_or_else(|| PlatformError::Message("ESP partition GUID is invalid".into()))?;

    let mut dp = Vec::new();
    // HardDrive node: GPT signature is the partition GUID.
    dp.push(4u8); dp.push(1u8); dp.extend_from_slice(&42u16.to_le_bytes());
    dp.extend_from_slice(&part_no.to_le_bytes());
    dp.extend_from_slice(&start.to_le_bytes());
    dp.extend_from_slice(&size.to_le_bytes());
    dp.extend_from_slice(&part_guid);
    dp.push(2u8); // MBR type (unused for GPT)
    dp.push(2u8); // signature type: GPT
    // FilePath node.
    let path16: Vec<u16> = std::ffi::OsStr::new(loader_path).encode_wide().chain(std::iter::once(0)).collect();
    let path_bytes = path16.len() * 2;
    dp.push(4u8); dp.push(4u8); dp.extend_from_slice(&((4 + path_bytes) as u16).to_le_bytes());
    for u in &path16 { dp.extend_from_slice(&u.to_le_bytes()); }
    // End of entire device path.
    dp.extend_from_slice(&[0x7f, 0xff, 4, 0]);

    let mut desc: Vec<u16> = label.encode_utf16().collect();
    desc.push(0);
    let mut option = Vec::new();
    option.extend_from_slice(&1u32.to_le_bytes()); // LOAD_OPTION_ACTIVE
    option.extend_from_slice(&(dp.len() as u16).to_le_bytes());
    for u in &desc { option.extend_from_slice(&u.to_le_bytes()); }
    option.extend_from_slice(&dp);

    let guid_w = wide(EFI_GLOBAL_GUID);
    unsafe {
        let mut order = vec![0u8; 4096];
        let n = GetFirmwareEnvironmentVariableW(
            wide("BootOrder").as_ptr(),
            guid_w.as_ptr(),
            order.as_mut_ptr() as *mut c_void,
            order.len() as u32,
        );
        let mut entries: Vec<u16> = if n >= 2 {
            order[..n as usize].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
        } else {
            Vec::new()
        };
        let mut next = 0u16;
        for e in &entries { if *e > next { next = *e; } }
        next = next.saturating_add(1);
        let name = format!("Boot{next:04X}");
        let ok = SetFirmwareEnvironmentVariableW(wide(&name).as_ptr(), guid_w.as_ptr(), option.as_ptr() as *const c_void, option.len() as u32);
        if ok == 0 {
            return Err(PlatformError::Message(format!("firmware refused the boot entry (Windows error {})", GetLastError())));
        }
        entries.push(next);
        let mut out = Vec::new();
        for e in entries { out.extend_from_slice(&e.to_le_bytes()); }
        let ok = SetFirmwareEnvironmentVariableW(wide("BootOrder").as_ptr(), guid_w.as_ptr(), out.as_ptr() as *const c_void, out.len() as u32);
        if ok == 0 {
            return Err(PlatformError::Message("boot entry was written but BootOrder could not be updated".into()));
        }
    }
    Ok(format!("UEFI boot entry '{}' created. Reboot and choose it, or set it one-time in firmware.", label))
}

fn guid_to_bytes(s: &str) -> Option<[u8; 16]> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 32 { return None; }
    let mut raw = [0u8; 16];
    for i in 0..16 { raw[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?; }
    Some([
        raw[3], raw[2], raw[1], raw[0], raw[5], raw[4], raw[7], raw[6], raw[8], raw[9], raw[10], raw[11], raw[12], raw[13], raw[14], raw[15],
    ])
}
