//! Lifecycle and private receipt for the optional loopback Office service.

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use crate::{bounded_file, config::ConfigPaths, file_lock};

const RECEIPT_LIMIT: usize = 4096;
const SERVICE_PROTOCOL: u32 = 1;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceReceipt {
    pub schema_version: u32,
    pub pid: u32,
    pub port: u16,
    pub nonce: String,
    pub browser_token: String,
    pub control_token: String,
    pub running_version: String,
}

impl ServiceReceipt {
    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn session_url(&self) -> String {
        format!("{}/local#token={}", self.endpoint(), self.browser_token)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ServiceStatus {
    pub running: bool,
    pub receipt: Option<ServiceReceipt>,
    pub restart_needed: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ServiceStart {
    pub receipt: ServiceReceipt,
    pub changed: bool,
    pub reused: bool,
}

#[derive(Debug)]
pub enum ServiceError {
    Unavailable(io::Error),
    PortUnavailable,
    Conflict,
    RestartRequired,
    Uncertain,
}

#[derive(Debug)]
pub enum PreviewError {
    NotRunning,
    RestartRequired,
    Limit,
    Invalid,
    Unavailable(io::Error),
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotRunning => "Start local Office before previewing an art pack.",
            Self::RestartRequired => {
                "Restart local Office with the current companion before previewing."
            }
            Self::Limit => "Local Office already has four live art previews.",
            Self::Invalid => "Local Office rejected the art preview.",
            Self::Unavailable(_) => "Local Office art preview is unavailable.",
        })
    }
}

impl std::error::Error for PreviewError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable(error) => Some(error),
            _ => None,
        }
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable(_) => "Local Office service is unavailable.",
            Self::PortUnavailable => "The requested local Office port is unavailable.",
            Self::Conflict => "Local Office is already running on another port.",
            Self::RestartRequired => "The installed Office changed; stop and restart Office.",
            Self::Uncertain => "Local Office process ownership could not be confirmed.",
        })
    }
}

impl std::error::Error for ServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable(error) => Some(error),
            _ => None,
        }
    }
}

pub fn status(paths: &ConfigPaths, installed_version: &str) -> Result<ServiceStatus, ServiceError> {
    let Some(receipt) = read_receipt(paths)? else {
        return Ok(ServiceStatus {
            running: false,
            receipt: None,
            restart_needed: false,
        });
    };
    if healthy(&receipt) {
        return Ok(ServiceStatus {
            running: true,
            restart_needed: receipt.running_version != installed_version,
            receipt: Some(receipt),
        });
    }
    if process_absent(receipt.pid) {
        return Ok(ServiceStatus {
            running: false,
            receipt: None,
            restart_needed: false,
        });
    }
    Err(ServiceError::Uncertain)
}

pub fn start(
    paths: &ConfigPaths,
    executable: &Path,
    installed_version: &str,
    requested_port: Option<u16>,
) -> Result<ServiceStart, ServiceError> {
    prepare_runtime(paths)?;
    let _lock = file_lock::exclusive(&runtime_directory(paths).join("start.lock"))
        .map_err(ServiceError::Unavailable)?;
    let current = status(paths, installed_version)?;
    if let Some(receipt) = current.receipt {
        if current.restart_needed {
            return Err(ServiceError::RestartRequired);
        }
        if requested_port.is_some_and(|port| port != receipt.port) {
            return Err(ServiceError::Conflict);
        }
        return Ok(ServiceStart {
            receipt,
            changed: false,
            reused: true,
        });
    }
    let mut command = Command::new(executable);
    command
        .args(["__tmt-office-service", "1", "serve"])
        .env(
            "TMT_OFFICE_SERVICE_PORT",
            requested_port.unwrap_or(0).to_string(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    // The service reaches core through the `tmt` that declared itself core, or
    // the one that launched the companion; never a PATH lookup.
    command.env(
        "TMT_EXECUTABLE",
        crate::core_executable::selected().map_err(ServiceError::Unavailable)?,
    );
    let mut child = command.spawn().map_err(ServiceError::Unavailable)?;
    let stdout = child.stdout.take().ok_or_else(|| {
        ServiceError::Unavailable(io::Error::other("Office readiness pipe is unavailable"))
    })?;
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    let ready = receiver.recv_timeout(Duration::from_secs(5));
    if matches!(ready, Ok(Ok(ref line)) if line == "TMT-OFFICE-SERVICE/1 PORT-UNAVAILABLE\n") {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ServiceError::PortUnavailable);
    }
    if !matches!(ready, Ok(Ok(line)) if line == "TMT-OFFICE-SERVICE/1 READY\n") {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ServiceError::Unavailable(io::Error::other(
            "Office readiness failed",
        )));
    }
    let receipt = match read_receipt(paths) {
        Ok(Some(receipt)) if healthy(&receipt) => receipt,
        Ok(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ServiceError::Unavailable(io::Error::other(
                "Office readiness was not authenticated",
            )));
        }
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    Ok(ServiceStart {
        receipt,
        changed: true,
        reused: false,
    })
}

pub fn stop(paths: &ConfigPaths) -> Result<bool, ServiceError> {
    match fs::symlink_metadata(runtime_directory(paths)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(ServiceError::Unavailable(error)),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(ServiceError::Uncertain);
        }
        Ok(_) => {}
    }
    prepare_runtime(paths)?;
    let _lock = file_lock::exclusive(&runtime_directory(paths).join("start.lock"))
        .map_err(ServiceError::Unavailable)?;
    let Some(receipt) = read_receipt(paths)? else {
        return Ok(false);
    };
    if !control(&receipt, "/control/v1/stop") {
        if process_absent(receipt.pid) {
            remove_matching_receipt(paths, &receipt)?;
            return Ok(false);
        }
        return Err(ServiceError::Uncertain);
    }
    for _ in 0..50 {
        if process_absent(receipt.pid) {
            remove_matching_receipt(paths, &receipt)?;
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(ServiceError::Uncertain)
}

pub fn preview(
    paths: &ConfigPaths,
    installed_version: &str,
    pack: &tmt_office_model::codec::office_prop::ValidatedPropPack,
) -> Result<serde_json::Value, PreviewError> {
    preview_pack(
        paths,
        installed_version,
        pack.bytes(),
        pack.digest(),
        PreviewKind::Prop,
    )
}

pub fn preview_avatar(
    paths: &ConfigPaths,
    installed_version: &str,
    pack: &tmt_office_model::codec::office_avatar::ValidatedAvatarPack,
) -> Result<serde_json::Value, PreviewError> {
    preview_pack(
        paths,
        installed_version,
        pack.bytes(),
        pack.digest(),
        PreviewKind::Avatar,
    )
}

#[derive(Clone, Copy)]
enum PreviewKind {
    Prop,
    Avatar,
}

impl PreviewKind {
    fn control_path(self) -> &'static str {
        match self {
            Self::Prop => "/control/v1/prop-previews",
            Self::Avatar => "/control/v1/avatar-previews",
        }
    }
    fn limit_code(self) -> &'static str {
        match self {
            Self::Prop => "OFFICE_PROP_PREVIEW_LIMIT",
            Self::Avatar => "OFFICE_AVATAR_PREVIEW_LIMIT",
        }
    }
    fn invalid_code(self) -> &'static str {
        match self {
            Self::Prop => "OFFICE_PROP_INVALID",
            Self::Avatar => "OFFICE_AVATAR_INVALID",
        }
    }
    fn ui_path(self) -> &'static str {
        match self {
            Self::Prop => "props",
            Self::Avatar => "avatars",
        }
    }
}

fn preview_pack(
    paths: &ConfigPaths,
    installed_version: &str,
    bytes: &[u8],
    digest: &str,
    kind: PreviewKind,
) -> Result<serde_json::Value, PreviewError> {
    let status = status(paths, installed_version).map_err(|error| match error {
        ServiceError::RestartRequired => PreviewError::RestartRequired,
        ServiceError::Unavailable(error) => PreviewError::Unavailable(error),
        _ => PreviewError::NotRunning,
    })?;
    let receipt = status.receipt.ok_or(PreviewError::NotRunning)?;
    if status.restart_needed {
        return Err(PreviewError::RestartRequired);
    }
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), receipt.port);
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(PreviewError::Unavailable)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(PreviewError::Unavailable)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(PreviewError::Unavailable)?;
    let headers = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nX-TMT-Office-Nonce: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        kind.control_path(),
        receipt.port,
        receipt.control_token,
        receipt.nonce,
        bytes.len()
    );
    stream
        .write_all(headers.as_bytes())
        .and_then(|()| stream.write_all(bytes))
        .map_err(PreviewError::Unavailable)?;
    let mut response = Vec::new();
    let length = stream
        .take(16_385)
        .read_to_end(&mut response)
        .map_err(PreviewError::Unavailable)?;
    if length >= 16_385 {
        return Err(PreviewError::Unavailable(io::Error::other(
            "Office preview response exceeds its bound",
        )));
    }
    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            PreviewError::Unavailable(io::Error::other("Invalid Office preview response"))
        })?;
    let (headers, body) = response.split_at(separator + 4);
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|error| PreviewError::Unavailable(io::Error::other(error)))?;
    if headers.starts_with(b"HTTP/1.1 429 ") && value["error"] == kind.limit_code() {
        return Err(PreviewError::Limit);
    }
    if headers.starts_with(b"HTTP/1.1 400 ") && value["error"] == kind.invalid_code() {
        return Err(PreviewError::Invalid);
    }
    if !headers.starts_with(b"HTTP/1.1 200 ") {
        return Err(PreviewError::Unavailable(io::Error::other(
            "Local Office rejected the preview request",
        )));
    }
    if !valid_preview_reply(&value, &receipt, digest, kind) {
        return Err(PreviewError::Invalid);
    }
    Ok(value)
}

fn valid_preview_reply(
    value: &serde_json::Value,
    receipt: &ServiceReceipt,
    digest: &str,
    kind: PreviewKind,
) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let Some(preview_id) = value["previewId"].as_str().filter(|id| valid_secret(id)) else {
        return false;
    };
    let expected_url = format!(
        "{}/local/{}/preview/{preview_id}#token={}",
        receipt.endpoint(),
        kind.ui_path(),
        receipt.browser_token
    );
    object.len() == 4
        && value["digest"].as_str() == Some(digest)
        && value["expiresAtMs"].as_u64().is_some()
        && value["url"].as_str() == Some(expected_url.as_str())
}

pub fn write_receipt(paths: &ConfigPaths, receipt: &ServiceReceipt) -> io::Result<()> {
    prepare_runtime(paths).map_err(|error| io::Error::other(error.to_string()))?;
    let bytes = serde_json::to_vec(receipt)?;
    if bytes.len() > RECEIPT_LIMIT {
        return Err(io::Error::other("Office receipt is too large"));
    }
    let directory = runtime_directory(paths);
    let staging = directory.join(format!(".service-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&staging)?;
    let result = file
        .write_all(&bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&staging, receipt_path(paths)))
        .and_then(|()| File::open(directory)?.sync_all());
    if result.is_err() && staging.exists() {
        // This UUID path belongs only to this write attempt.
        fs::remove_file(&staging)?;
    }
    result
}

pub fn remove_matching_receipt(
    paths: &ConfigPaths,
    receipt: &ServiceReceipt,
) -> Result<(), ServiceError> {
    if read_receipt(paths)?.as_ref() == Some(receipt) {
        match fs::remove_file(receipt_path(paths)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(ServiceError::Unavailable(error)),
        }
    }
    Ok(())
}

pub fn service_lock(paths: &ConfigPaths) -> Result<impl Drop, io::Error> {
    prepare_runtime(paths).map_err(|error| io::Error::other(error.to_string()))?;
    file_lock::exclusive(&runtime_directory(paths).join("service.lock"))
}

fn read_receipt(paths: &ConfigPaths) -> Result<Option<ServiceReceipt>, ServiceError> {
    let bytes = match bounded_file::read_no_follow(&receipt_path(paths), RECEIPT_LIMIT) {
        Ok(bytes) => bytes,
        Err(bounded_file::FileReadError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(ServiceError::Unavailable(io::Error::other(error))),
    };
    let receipt: ServiceReceipt = serde_json::from_slice(&bytes)
        .map_err(|error| ServiceError::Unavailable(io::Error::other(error)))?;
    if receipt.schema_version != SERVICE_PROTOCOL
        || receipt.port == 0
        || !valid_secret(&receipt.nonce)
        || !valid_secret(&receipt.browser_token)
        || !valid_secret(&receipt.control_token)
    {
        return Err(ServiceError::Uncertain);
    }
    Ok(Some(receipt))
}

fn healthy(receipt: &ServiceReceipt) -> bool {
    control(receipt, "/control/v1/health")
}

fn control(receipt: &ServiceReceipt, path: &str) -> bool {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), receipt.port);
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(500)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nX-TMT-Office-Nonce: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        receipt.port, receipt.control_token, receipt.nonce
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = Vec::new();
    let Ok(length) = stream.take(4096).read_to_end(&mut response) else {
        return false;
    };
    length < 4096 && response.starts_with(b"HTTP/1.1 200 ")
}

fn prepare_runtime(paths: &ConfigPaths) -> Result<(), ServiceError> {
    let directory = runtime_directory(paths);
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder
        .create(&directory)
        .map_err(ServiceError::Unavailable)?;
    let metadata = fs::symlink_metadata(&directory).map_err(ServiceError::Unavailable)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ServiceError::Uncertain);
    }
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .map_err(ServiceError::Unavailable)
}

fn runtime_directory(paths: &ConfigPaths) -> PathBuf {
    paths.office_directory().join("runtime")
}
fn receipt_path(paths: &ConfigPaths) -> PathBuf {
    runtime_directory(paths).join("service-v1.json")
}
fn valid_secret(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn process_absent(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    matches!(
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            Option::<nix::sys::signal::Signal>::None
        ),
        Err(nix::errno::Errno::ESRCH)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt() -> ServiceReceipt {
        ServiceReceipt {
            schema_version: SERVICE_PROTOCOL,
            pid: 1,
            port: 4321,
            nonce: "n".repeat(43),
            browser_token: "b".repeat(43),
            control_token: "c".repeat(43),
            running_version: "test".into(),
        }
    }

    #[test]
    fn preview_reply_must_match_candidate_and_exact_private_target() {
        let receipt = receipt();
        let pack = tmt_office_model::codec::office_prop::builtin_pack();
        let preview_id = "p".repeat(43);
        let valid = serde_json::json!({
            "digest": pack.digest(),
            "previewId": preview_id,
            "expiresAtMs": 1,
            "url": format!(
                "{}/local/props/preview/{}#token={}",
                receipt.endpoint(), preview_id, receipt.browser_token
            )
        });
        assert!(valid_preview_reply(
            &valid,
            &receipt,
            pack.digest(),
            PreviewKind::Prop
        ));
        for invalid in [
            {
                let mut value = valid.clone();
                value["digest"] = serde_json::json!(format!("sha256:{}", "0".repeat(64)));
                value
            },
            {
                let mut value = valid.clone();
                value["previewId"] = serde_json::json!("short");
                value
            },
            {
                let mut value = valid.clone();
                value["url"] = serde_json::json!(format!(
                    "{}/local/props/preview/{}-other#token={}",
                    receipt.endpoint(),
                    preview_id,
                    receipt.browser_token
                ));
                value
            },
            {
                let mut value = valid.clone();
                value["url"] = serde_json::json!(format!(
                    "{}/local/props/preview/{}#token=wrong",
                    receipt.endpoint(),
                    preview_id
                ));
                value
            },
        ] {
            assert!(!valid_preview_reply(
                &invalid,
                &receipt,
                pack.digest(),
                PreviewKind::Prop
            ));
        }
        let directional = tmt_office_model::codec::office_prop::validate_pack(include_bytes!(
            "../../../../extensions/tmt-office/contracts/prop-pack-v2-sample.tmtprop.json"
        ))
        .unwrap();
        let mut directional_reply = valid.clone();
        directional_reply["digest"] = serde_json::json!(directional.digest());
        assert!(valid_preview_reply(
            &directional_reply,
            &receipt,
            directional.digest(),
            PreviewKind::Prop
        ));
        assert!(!valid_preview_reply(
            &valid,
            &receipt,
            directional.digest(),
            PreviewKind::Prop
        ));
        let avatar_digest = tmt_office_model::codec::office_avatar::framed_digest(pack.bytes(), 1);
        let avatar = serde_json::json!({
            "digest": avatar_digest,
            "previewId": preview_id,
            "expiresAtMs": 1,
            "url": format!(
                "{}/local/avatars/preview/{}#token={}",
                receipt.endpoint(), preview_id, receipt.browser_token
            )
        });
        assert!(valid_preview_reply(
            &avatar,
            &receipt,
            &avatar_digest,
            PreviewKind::Avatar
        ));
    }
}
