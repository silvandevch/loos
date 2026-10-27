//! Shared helpers: run host tools, skip when absent, scratch files.
#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn have(tool: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {}", tool)])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("loos-fs-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub fn run(cmd: &str, args: &[&str]) -> (bool, String) {
    let out = Command::new(cmd).args(args).output().unwrap_or_else(|e| panic!("{}: {}", cmd, e));
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), s)
}

pub fn must(cmd: &str, args: &[&str]) -> String {
    let (ok, s) = run(cmd, args);
    assert!(ok, "{} {:?} failed:\n{}", cmd, args, s);
    s
}

pub fn blank(path: &Path, bytes: u64) {
    let f = std::fs::File::create(path).unwrap();
    f.set_len(bytes).unwrap();
}

/// Deterministic pseudo-random payload.
pub fn payload(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9e3779b97f4a7c15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}
