//! Background startup uses the existing detached-process owner, then the same
//! supervisor socket for a managed-launch receipt. It never edits provider state.

use super::{Connection, socket_path};
use crate::{config::ConfigPaths, private_file::ensure_private_directory, process::detached};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    io,
    os::unix::fs::OpenOptionsExt,
    time::Duration,
};

pub struct StartedMember {
    pub pane: String,
    pub name: String,
    pub socket: String,
    pub cleanup_warning: Option<String>,
}

/// Preparation refusal is distinct from a detached startup whose result could
/// not be confirmed. Neither case gives authority to replay a possible launch.
#[derive(Debug)]
pub enum LaunchError {
    NotStarted(io::Error),
    Unconfirmed { pane: String, cause: io::Error },
}
impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotStarted(error) => write!(f, "no member was launched: {error}"),
            Self::Unconfirmed { pane, cause } => {
                write!(f, "launch in {pane} is not confirmed: {cause}")
            }
        }
    }
}
impl std::error::Error for LaunchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::NotStarted(error) => error,
            Self::Unconfirmed { cause, .. } => cause,
        })
    }
}
impl From<io::Error> for LaunchError {
    fn from(value: io::Error) -> Self {
        Self::NotStarted(value)
    }
}

pub fn start_member(
    executable: &OsStr,
    arguments: &[OsString],
    paths: &ConfigPaths,
) -> Result<StartedMember, LaunchError> {
    let directory = paths.pty_directory();
    ensure_private_directory(&directory)?;
    let pane = format!("pty_{}", uuid::Uuid::new_v4().simple());
    super::validate(&directory, &pane)?;
    let log_path = directory.join(format!("{pane}.startup.log"));
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&log_path)?;
    let mut argv: Vec<OsString> = vec![
        "__pty-serve".into(),
        directory.as_os_str().into(),
        pane.clone().into(),
        "80".into(),
        "24".into(),
        executable.into(),
    ];
    argv.extend_from_slice(arguments);
    let uncertain = |cause| LaunchError::Unconfirmed {
        pane: pane.clone(),
        cause,
    };
    let acknowledgment =
        detached::start_line(executable, &argv, log, b"ready ").map_err(|error| {
            uncertain(io::Error::other(format!(
                "supervisor startup is not confirmed: {error}; diagnostics: {}",
                log_path.display()
            )))
        })?;
    let line =
        std::str::from_utf8(&acknowledgment).map_err(|error| uncertain(io::Error::other(error)))?;
    let pid = line
        .strip_prefix("ready ")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|pid| *pid > 0)
        .ok_or_else(|| {
            uncertain(io::Error::other(
                "the supervisor returned invalid readiness",
            ))
        })?;
    let mut client = Connection::connect(&directory, &pane).map_err(|error| {
        uncertain(io::Error::other(format!(
            "supervisor started for PID {pid}, but launch is not confirmed: {error}"
        )))
    })?;
    let marker = client
        .await_launch(Duration::from_secs(60))
        .map_err(|error| {
            uncertain(io::Error::other(format!(
                "supervisor started for PID {pid}, but managed launch is not confirmed: {error}"
            )))
        })?;
    drop(client);
    let marker: tmt_driver_protocol::Marker =
        serde_json::from_str(&marker).map_err(|error| uncertain(io::Error::other(error)))?;
    let cleanup_warning = fs::remove_file(&log_path).err().map(|error| {
        format!(
            "could not remove startup diagnostics {}: {error}",
            log_path.display()
        )
    });
    Ok(StartedMember {
        name: marker.name,
        cleanup_warning,
        socket: socket_path(&directory, &pane)
            .to_string_lossy()
            .into_owned(),
        pane,
    })
}
