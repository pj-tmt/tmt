//! Bounded, credential-free Git remote category resolution.

use crate::process::{CommandRequest, CommandRunner, UnixCommandRunner};
use std::{
    ffi::{OsStr, OsString},
    io,
    path::Path,
    time::{Duration, Instant},
};

pub fn resolve_remote(directory: &Path, name: &str) -> io::Result<String> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
    {
        return Err(invalid());
    }
    let args = [
        OsString::from("-C"),
        directory.as_os_str().to_owned(),
        OsString::from("remote"),
        OsString::from("get-url"),
        OsString::from("--"),
        OsString::from(name),
    ];
    let output = UnixCommandRunner
        .execute(CommandRequest {
            program: OsStr::new("git"),
            args: &args,
            input: &[],
            deadline: Instant::now() + Duration::from_secs(3),
            max_output_bytes: 4096,
        })
        .map_err(io::Error::other)?;
    if !output.stderr.is_empty() {
        return Err(invalid());
    }
    let raw = std::str::from_utf8(&output.stdout)
        .map_err(|_| invalid())?
        .strip_suffix('\n')
        .ok_or_else(invalid)?;
    canonicalize(raw)
}

pub fn canonicalize(raw: &str) -> io::Result<String> {
    if raw.is_empty() || raw.len() > 2048 || raw.contains(['?', '#', '\\', '\0', '%']) {
        return Err(invalid());
    }
    let (scheme, authority, path) = if let Some(rest) = raw.strip_prefix("https://") {
        let (a, p) = rest.split_once('/').ok_or_else(invalid)?;
        ("https", a, p)
    } else if let Some(rest) = raw.strip_prefix("ssh://") {
        let (a, p) = rest.split_once('/').ok_or_else(invalid)?;
        ("ssh", a, p)
    } else if !raw.contains("://") {
        let (a, p) = raw.split_once(':').ok_or_else(invalid)?;
        ("ssh", a, p)
    } else {
        return Err(invalid());
    };
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if host_port.is_empty()
        || host_port.starts_with(['.', '-'])
        || host_port.ends_with(['.', '-'])
        || host_port.contains("..")
        || !host_port
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
    {
        return Err(invalid());
    }
    if raw.as_bytes().get(1) == Some(&b':')
        && raw
            .as_bytes()
            .get(2)
            .is_some_and(|byte| *byte == b'/' || *byte == b'\\')
    {
        return Err(invalid());
    }
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => (h, Some(p)),
        _ if host_port.matches(':').count() == 0 => (host_port, None),
        _ => return Err(invalid()),
    };
    if let Some(port) = port
        && port
            .parse::<u16>()
            .ok()
            .filter(|value| *value > 0)
            .is_none()
    {
        return Err(invalid());
    }
    let path = path.trim_matches('/');
    if host.is_empty()
        || path.is_empty()
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(invalid());
    }
    let mut path = path;
    if let Some(value) = path.strip_suffix(".git") {
        path = value;
    }
    if path.is_empty() {
        return Err(invalid());
    }
    let port = port.filter(|port| {
        !((*port == "443" && scheme == "https") || (*port == "22" && scheme == "ssh"))
    });
    let canonical = format!(
        "{}{}/{}",
        host.to_ascii_lowercase(),
        port.map_or(String::new(), |p| format!(":{p}")),
        path
    );
    if !tmt_core::repository_id::valid_repository_id(&canonical) {
        return Err(invalid());
    }
    Ok(canonical)
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "Repository remote is invalid.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_remote_forms_are_credential_free() {
        for value in [
            "git@GitHub.COM:Org/Repo.git",
            "ssh://token@github.com:22/Org/Repo.git/",
            "https://user:secret@github.com:443/Org/Repo",
        ] {
            assert_eq!(canonicalize(value).unwrap(), "github.com/Org/Repo");
        }
    }

    #[test]
    fn local_ambiguous_and_traversal_remotes_are_rejected() {
        for value in [
            "../repo",
            "file:///tmp/repo",
            "https://github.com/a/../b",
            "https://github.com/a/%2e%2e/b",
            "https://github.com/a/b?token=secret",
            "https://github.com/a/b#secret",
            "C:/repo",
        ] {
            assert!(canonicalize(value).is_err(), "accepted {value}");
        }
    }
}
