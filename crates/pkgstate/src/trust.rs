//! Download/install trust policy. Default is deny: https only, host must be allowlisted,
//! SHA-256 must be present, and (by default) the package must be signed by an allowed publisher.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// What a front-end wants to fetch/install.
#[derive(Clone, Debug)]
pub struct Candidate<'a> {
    /// Download URL.
    pub url: &'a str,
    /// Expected SHA-256 hex from the (trusted) manifest.
    pub sha256: &'a str,
    /// Publisher from a *verified* signature, if any.
    pub signed_publisher: Option<&'a str>,
}

/// Why a candidate was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Denial {
    /// Not https.
    NotHttps,
    /// URL has credentials, a non-default port outside the allowlist, or is malformed.
    BadUrl,
    /// Host not allowlisted.
    HostDenied(String),
    /// Missing/malformed hash.
    NoHash,
    /// Publisher required but absent or not allowlisted.
    PublisherDenied,
}

/// The policy (all lists are admin-configured; nothing is built in).
#[derive(Clone, Debug)]
pub struct Policy {
    /// Allowed hosts; `*.example.com` matches subdomains only.
    pub hosts: Vec<String>,
    /// Allowed signed publishers (exact, case-insensitive).
    pub publishers: Vec<String>,
    /// Allow packages without a verified signature (hash still required).
    pub allow_unsigned: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { hosts: Vec::new(), publishers: Vec::new(), allow_unsigned: false }
    }
}

/// Extract the lower-case host of an https URL; rejects userinfo and explicit ports other than 443.
pub fn https_host(url: &str) -> Result<String, Denial> {
    let rest = url.strip_prefix("https://").ok_or(Denial::NotHttps)?;
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    if auth.is_empty() || auth.contains('@') || auth.contains(char::is_whitespace) {
        return Err(Denial::BadUrl);
    }
    let host = match auth.rsplit_once(':') {
        Some((h, "443")) => h,
        Some(_) => return Err(Denial::BadUrl),
        None => auth,
    };
    if host.is_empty() || host.starts_with('.') || host.ends_with('.') || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') {
        return Err(Denial::BadUrl);
    }
    Ok(host.to_ascii_lowercase())
}

fn host_matches(pattern: &str, host: &str) -> bool {
    let p = pattern.to_ascii_lowercase();
    match p.strip_prefix("*.") {
        Some(suffix) => host.len() > suffix.len() + 1 && host.ends_with(suffix) && host.as_bytes()[host.len() - suffix.len() - 1] == b'.',
        None => p == host,
    }
}

impl Policy {
    /// Evaluate a candidate.
    pub fn check(&self, c: &Candidate) -> Result<(), Denial> {
        let host = https_host(c.url)?;
        if !self.hosts.iter().any(|p| host_matches(p, &host)) {
            return Err(Denial::HostDenied(host));
        }
        if c.sha256.len() != 64 || !c.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Denial::NoHash);
        }
        match c.signed_publisher {
            Some(p) if self.publishers.iter().any(|a| a.eq_ignore_ascii_case(p)) => Ok(()),
            Some(_) => Err(Denial::PublisherDenied),
            None if self.allow_unsigned => Ok(()),
            None => Err(Denial::PublisherDenied),
        }
    }

    /// Convenience: add a host.
    pub fn allow_host(&mut self, h: &str) -> &mut Self {
        self.hosts.push(h.to_string());
        self
    }
}
