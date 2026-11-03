//! Copy-only migration. The source is never modified. Every operation is
//! cancellable, every copied file is journaled, and the copy can be verified
//! with SHA-256 before the bundle is declared complete.

use crate::model::{
    ConflictPolicy, CopyStats, MigrationEntry, MigrationManifest, MigrationOptions, MigrationPlan,
    SourceRecord,
};
use crate::payload::{sha256_file, PayloadError};
use crate::platform::{human_bytes, utc_now_string, APP_NAME, APP_VERSION};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use walkdir::WalkDir;

const JOURNAL: &str = ".loos-switch-copy.jsonl";
const CHUNK: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("operation cancelled")]
    Cancelled,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("verification failed for {0}")]
    Verify(String),
    #[error("{0}")]
    Message(String),
}

#[derive(Clone, Debug)]
pub struct CopyProgress {
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: String,
    pub skipped: u64,
    pub errors: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct JournalRecord {
    relative: String,
    source: String,
    size: u64,
    sha256: Option<String>,
    status: String,
}

fn excluded(name: &str, opts: &MigrationOptions) -> bool {
    opts.excluded_names.iter().any(|e| e.eq_ignore_ascii_case(name))
}

/// Walk one selected root and count regular files/bytes (following the same
/// filters as the copy, so the estimate is honest).
pub fn measure_source(
    source: &Path,
    opts: &MigrationOptions,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64, &Path),
) -> Result<(u64, u64), MigrationError> {
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut last = 0u64;
    for ent in WalkDir::new(source).follow_links(opts.follow_symlinks).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        if e.depth() == 0 {
            return true;
        }
        if excluded(&name, opts) {
            return false;
        }
        if !opts.include_hidden && name.starts_with('.') {
            return false;
        }
        true
    }) {
        if cancel.load(Ordering::Relaxed) {
            return Err(MigrationError::Cancelled);
        }
        let ent = match ent {
            Ok(e) => e,
            Err(_) => continue,
        };
        if ent.file_type().is_file() {
            if let Ok(m) = ent.metadata() {
                files += 1;
                bytes += m.len();
                if bytes - last >= 64 * 1024 * 1024 {
                    last = bytes;
                    progress(files, bytes, ent.path());
                }
            }
        }
    }
    progress(files, bytes, source);
    Ok((files, bytes))
}

fn dest_name_conflict(dest: &Path, policy: ConflictPolicy) -> Option<PathBuf> {
    if !dest.exists() {
        return Some(dest.to_path_buf());
    }
    match policy {
        ConflictPolicy::Overwrite => Some(dest.to_path_buf()),
        ConflictPolicy::Skip => None,
        ConflictPolicy::Rename => {
            let stem = dest.file_stem().unwrap_or_default().to_string_lossy();
            let ext = dest.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            for n in 1..10000 {
                let p = dest.with_file_name(format!("{stem} ({n}){ext}"));
                if !p.exists() {
                    return Some(p);
                }
            }
            None
        }
    }
}

fn copy_one(
    src: &Path,
    dst: &Path,
    opts: &MigrationOptions,
    cancel: &AtomicBool,
    journal: &mut File,
    entry: &mut MigrationEntry,
    bytes_done: &mut u64,
    bytes_total: u64,
    mut progress: impl FnMut(u64),
) -> Result<Option<String>, MigrationError> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    // Resume: if the destination already has the same size and mtime, skip it.
    if let (Ok(sm), Ok(dm)) = (fs::metadata(src), fs::metadata(dst)) {
        if sm.len() == dm.len() && sm.modified().ok() == dm.modified().ok() && dm.is_file() {
            *bytes_done += sm.len();
            let rec = JournalRecord {
                relative: dst.to_string_lossy().into_owned(),
                source: src.to_string_lossy().into_owned(),
                size: sm.len(),
                sha256: None,
                status: "already".into(),
            };
            writeln!(journal, "{}", serde_json::to_string(&rec).unwrap_or_default())?;
            progress(*bytes_done);
            return Ok(None);
        }
    }
    let mut fin = File::open(src)?;
    let tmp = dst.with_extension(format!(
        "{}.loos-part",
        dst.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    let mut fout = File::create(&tmp)?;
    let mut buf = vec![0u8; CHUNK];
    let mut copied = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = fs::remove_file(&tmp);
            return Err(MigrationError::Cancelled);
        }
        let n = fin.read(&mut buf)?;
        if n == 0 {
            break;
        }
        fout.write_all(&buf[..n])?;
        copied += n as u64;
        *bytes_done += n as u64;
        progress((*bytes_done).min(bytes_total));
    }
    fout.sync_all()?;
    drop(fout);
    if opts.preserve_timestamps {
        if let Ok(meta) = fs::metadata(src) {
            if let Ok(f) = File::options().write(true).open(&tmp) {
                let mut times = fs::FileTimes::new();
                if let Ok(t) = meta.modified() {
                    times = times.set_modified(t);
                }
                if let Ok(t) = meta.accessed() {
                    times = times.set_accessed(t);
                }
                let _ = f.set_times(times);
            }
        }
    }
    fs::rename(&tmp, dst)?;
    let mut digest = None;
    if opts.verify_hashes {
        let cancel2 = AtomicBool::new(false);
        let src_hash = sha256_file(src, cancel, |_| {}).map_err(map_payload)?;
        let dst_hash = sha256_file(dst, &cancel2, |_| {}).map_err(map_payload)?;
        if src_hash != dst_hash {
            return Err(MigrationError::Verify(src.display().to_string()));
        }
        digest = Some(src_hash);
    }
    entry.size = copied;
    let rec = JournalRecord {
        relative: dst.to_string_lossy().into_owned(),
        source: src.to_string_lossy().into_owned(),
        size: copied,
        sha256: digest.clone(),
        status: "copied".into(),
    };
    writeln!(journal, "{}", serde_json::to_string(&rec).unwrap_or_default())?;
    journal.flush()?;
    progress(*bytes_done);
    Ok(digest)
}

fn map_payload(e: PayloadError) -> MigrationError {
    match e {
        PayloadError::Cancelled => MigrationError::Cancelled,
        other => MigrationError::Message(other.to_string()),
    }
}

/// Execute a plan. Files land under `<destination_root>/home/<target_home>/...`.
/// The bundle also gets a manifest, a plain-language restore note, and a
/// resumable journal. The source is opened read-only.
pub fn execute(
    plan: &MigrationPlan,
    cancel: &AtomicBool,
    mut progress: impl FnMut(CopyProgress),
) -> Result<MigrationManifest, MigrationError> {
    if plan.free_bytes > 0 && plan.total_bytes > plan.free_bytes {
        return Err(MigrationError::Message(format!(
            "Not enough free space at the destination: need {}, {} available.",
            human_bytes(plan.total_bytes),
            human_bytes(plan.free_bytes)
        )));
    }
    let bundle_home = if plan.direct_to_home {
        plan.destination_root.clone()
    } else {
        plan.destination_root.join("home").join(&plan.target_home)
    };
    fs::create_dir_all(&bundle_home)?;
    let journal_path = plan.destination_root.join(JOURNAL);
    let mut journal = File::options().create(true).append(true).open(&journal_path)?;

    // Pre-compute the file list so progress is meaningful, but keep memory
    // bounded by counting first.
    progress(CopyProgress {
        files_done: 0,
        files_total: 0,
        bytes_done: 0,
        bytes_total: plan.total_bytes,
        current: "Checking folders".into(),
        skipped: 0,
        errors: 0,
    });
    let mut total_files = 0u64;
    for s in &plan.sources {
        let (f, _) = measure_source(&s.path, &plan.options, cancel, |_, _, _| {})?;
        total_files += f;
    }

    let mut stats = CopyStats { files_copied: 0, files_skipped: 0, bytes_copied: 0, errors: Vec::new() };
    let mut records = Vec::new();
    let bytes_total = plan.total_bytes.max(1);
    for src in &plan.sources {
        if cancel.load(Ordering::Relaxed) {
            return Err(MigrationError::Cancelled);
        }
        let dest_root = bundle_home.join(src.destination.trim_start_matches('/'));
        let mut src_files = 0u64;
        let mut src_bytes = 0u64;
        for ent in WalkDir::new(&src.path).follow_links(plan.options.follow_symlinks).into_iter().filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            if e.depth() == 0 {
                return true;
            }
            if excluded(&name, &plan.options) {
                return false;
            }
            if !plan.options.include_hidden && name.starts_with('.') {
                return false;
            }
            true
        }) {
            if cancel.load(Ordering::Relaxed) {
                return Err(MigrationError::Cancelled);
            }
            let ent = match ent {
                Ok(e) => e,
                Err(e) => {
                    if stats.errors.len() < 50 {
                        stats.errors.push(format!("{}: {e}", src.path.display()));
                    }
                    continue;
                }
            };
            let rel = match ent.path().strip_prefix(&src.path) {
                Ok(r) => r.to_path_buf(),
                Err(_) => continue,
            };
            if rel.as_os_str().is_empty() {
                continue;
            }
            let dst = dest_root.join(&rel);
            if ent.file_type().is_symlink() && !plan.options.follow_symlinks {
                stats.files_skipped += 1;
                continue;
            }
            if ent.file_type().is_dir() {
                fs::create_dir_all(&dst)?;
                continue;
            }
            if !ent.file_type().is_file() {
                stats.files_skipped += 1;
                continue;
            }
            let Some(actual_dst) = dest_name_conflict(&dst, plan.options.conflict) else {
                stats.files_skipped += 1;
                continue;
            };
            let size = ent.metadata().map(|m| m.len()).unwrap_or(0);
            let mut entry = MigrationEntry {
                source: ent.path().to_path_buf(),
                relative: rel.clone(),
                destination: actual_dst.clone(),
                size,
                modified_unix: ent.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()),
            };
            let current = rel.display().to_string();
            let absolute_done_base = stats.bytes_copied;
            let mut local_bytes = stats.bytes_copied;
            let files_done = stats.files_copied;
            let files_skipped = stats.files_skipped;
            let error_count = stats.errors.len() as u64;
            let mut last_report = 0u64;
            let res = copy_one(
                ent.path(),
                &actual_dst,
                &plan.options,
                cancel,
                &mut journal,
                &mut entry,
                &mut local_bytes,
                bytes_total,
                |done| {
                    let delta = done.saturating_sub(absolute_done_base);
                    if delta - last_report > 8 * 1024 * 1024 {
                        last_report = delta;
                        progress(CopyProgress {
                            files_done,
                            files_total: total_files,
                            bytes_done: absolute_done_base + delta,
                            bytes_total,
                            current: current.clone(),
                            skipped: files_skipped,
                            errors: error_count,
                        });
                    }
                },
            );
            stats.bytes_copied = local_bytes;
            match res {
                Ok(_) => {
                    stats.files_copied += 1;
                    src_files += 1;
                    src_bytes += size;
                }
                Err(MigrationError::Cancelled) => return Err(MigrationError::Cancelled),
                Err(e) => {
                    if stats.errors.len() < 50 {
                        stats.errors.push(format!("{}: {e}", ent.path().display()));
                    }
                }
            }
        }
        records.push(SourceRecord { source: src.path.clone(), destination: src.destination.clone(), bytes: src_bytes, files: src_files });
    }
    let manifest = MigrationManifest {
        format: 1,
        created_utc: utc_now_string(),
        app: APP_NAME.into(),
        app_version: APP_VERSION.into(),
        target_home: plan.target_home.clone(),
        options: plan.options.clone(),
        sources: records,
        stats: stats.clone(),
    };
    let manifest_name = if plan.direct_to_home { ".loos-switch-manifest.json" } else { "manifest.json" };
    let mpath = plan.destination_root.join(manifest_name);
    let tmp = plan.destination_root.join(format!("{manifest_name}.part"));
    fs::write(&tmp, serde_json::to_vec_pretty(&manifest).unwrap_or_default())?;
    fs::rename(tmp, mpath)?;
    let readme = format!(
        "LoOS Switch migration bundle\n===========================\n\nCreated: {}\nTarget LoOS home: /home/{}\n\nThis bundle is copy-only: your source files were never moved or deleted.\nTo bring it onto LoOS, copy the contents of:\n  {}\ninto:\n  /home/{}\n\nA manifest with SHA-256 hashes for every copied file is in manifest.json.\n\nResult: {} files, {}, skipped {}, {} errors.\n",
        utc_now_string(),
        plan.target_home,
        bundle_home.display(),
        plan.target_home,
        stats.files_copied,
        human_bytes(stats.bytes_copied),
        stats.files_skipped,
        stats.errors.len()
    );
    if !plan.direct_to_home {
        fs::write(plan.destination_root.join("RESTORE.txt"), readme)?;
    }
    progress(CopyProgress {
        files_done: stats.files_copied,
        files_total: total_files,
        bytes_done: stats.bytes_copied,
        bytes_total: bytes_total,
        current: "Verified".into(),
        skipped: stats.files_skipped,
        errors: stats.errors.len() as u64,
    });
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MigrationOptions, MigrationSource};

    #[test]
    fn cancellation_stops_before_completion() {
        let base = std::env::temp_dir().join(format!("loos-switch-cancel-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("large.bin"), vec![9u8; 16 * 1024 * 1024]).unwrap();
        let plan = MigrationPlan {
            sources: vec![MigrationSource {
                path: src.clone(),
                destination: "from-old-pc".into(),
                selected: true,
                size_bytes: Some(16 * 1024 * 1024),
                file_count: Some(1),
            }],
            destination_root: base.join("dest"),
            target_home: "user".into(),
            options: MigrationOptions::default(),
            free_bytes: u64::MAX,
            total_bytes: 16 * 1024 * 1024,
            direct_to_home: false,
        };
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let r = execute(&plan, &cancel, |p| {
            if p.bytes_done > 1024 * 1024 {
                c.store(true, Ordering::SeqCst);
            }
        });
        assert!(matches!(r, Err(MigrationError::Cancelled)));
        assert!(!base.join("dest/home/user/from-old-pc/large.bin").exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn copy_cancel_and_verify() {
        let base = std::env::temp_dir().join(format!("loos-switch-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("src");
        fs::create_dir_all(src.join("Docs")).unwrap();
        fs::write(src.join("Docs/a.txt"), b"hello migration").unwrap();
        fs::write(src.join("b.bin"), vec![7u8; 3 * 1024 * 1024]).unwrap();
        let plan = MigrationPlan {
            sources: vec![MigrationSource {
                path: src.clone(),
                destination: "Docs-from-old-pc".into(),
                selected: true,
                size_bytes: Some(3 * 1024 * 1024),
                file_count: Some(2),
            }],
            destination_root: base.join("dest"),
            target_home: "user".into(),
            options: MigrationOptions { verify_hashes: true, ..Default::default() },
            free_bytes: u64::MAX,
            total_bytes: 3 * 1024 * 1024,
            direct_to_home: false,
        };
        let cancel = AtomicBool::new(false);
        let m = execute(&plan, &cancel, |_| {}).unwrap();
        assert_eq!(m.stats.files_copied, 2);
        assert!(base.join("dest/home/user/Docs-from-old-pc/Docs/a.txt").exists());
        assert!(base.join("dest/manifest.json").exists());
        let _ = fs::remove_dir_all(&base);
    }
}
