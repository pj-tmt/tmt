//! Provider-owned foreground command planning. Fresh remote launch leaves thread
//! creation to the TUI; exact resume supplies only its selected thread.

use crate::runtime::RuntimeCommand;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};
use tmt_core::binding::session::ProviderSessionId;

pub const TOKEN_ENV: &str = "TMT_CODEX_ENDPOINT_TOKEN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentError {
    UnsupportedArgument(OsString),
    MissingValue(OsString),
    InvalidEndpoint,
    InvalidWorkingDirectory,
    UnsupportedPermission(&'static str),
    UnsupportedConfig,
}

/// Arguments validated before any endpoint or provider process is created.
/// This first channel surface intentionally accepts options, not an existing
/// resume/fork subcommand or a positional prompt that could submit extra input.
#[derive(Debug)]
pub struct LaunchOptions {
    foreground: Vec<OsString>,
    server: Vec<OsString>,
    working_directory: PathBuf,
    sandbox: Option<String>,
    approval: Option<String>,
}

impl LaunchOptions {
    pub fn parse(command: &RuntimeCommand, cwd: &Path) -> Result<Self, AttachmentError> {
        if !cwd.is_absolute() {
            return Err(AttachmentError::InvalidWorkingDirectory);
        }
        let mut result = Self {
            foreground: Vec::new(),
            server: Vec::new(),
            working_directory: cwd.to_owned(),
            sandbox: None,
            approval: None,
        };
        let mut args = command.args.iter();
        while let Some(argument) = args.next() {
            let text = argument
                .to_str()
                .ok_or_else(|| AttachmentError::UnsupportedArgument(argument.clone()))?;
            let (name, inline) = text
                .split_once('=')
                .map_or((text, None), |(k, v)| (k, Some(v)));
            if matches!(name, "--strict-config" | "--no-alt-screen") && inline.is_none() {
                result.foreground.push(argument.clone());
                if name == "--strict-config" {
                    result.server.push(argument.clone());
                }
                continue;
            }
            if !matches!(
                name,
                "-c" | "--config"
                    | "--enable"
                    | "--disable"
                    | "-m"
                    | "--model"
                    | "-s"
                    | "--sandbox"
                    | "-a"
                    | "--ask-for-approval"
                    | "-C"
                    | "--cd"
            ) {
                return Err(AttachmentError::UnsupportedArgument(argument.clone()));
            }
            let value = match inline {
                Some(value) => OsString::from(value),
                None => args
                    .next()
                    .cloned()
                    .ok_or_else(|| AttachmentError::MissingValue(argument.clone()))?,
            };
            result.option(name, value, cwd)?;
        }
        Ok(result)
    }

    /// Only the launcher's typed exact-resume target authorizes the generated
    /// resume grammar. Arbitrary user resume/fork argv still fails `parse`.
    pub fn for_launch(
        command: &RuntimeCommand,
        cwd: &Path,
        resume: Option<&ProviderSessionId>,
    ) -> Result<Self, AttachmentError> {
        let Some(session) = resume else {
            return Self::parse(command, cwd);
        };
        let invalid = || AttachmentError::UnsupportedArgument("exact resume".into());
        uuid::Uuid::parse_str(session.as_str()).map_err(|_| invalid())?;
        let mut args = command.args.as_slice();
        if args.first().is_none_or(|arg| arg != "resume") {
            return Err(invalid());
        }
        args = &args[1..];
        let mut options = Vec::new();
        if args.first().is_some_and(|arg| arg == "-m") {
            if args.len() < 3 {
                return Err(invalid());
            }
            options.extend_from_slice(&args[..2]);
            args = &args[2..];
        }
        if args.first().is_none_or(|arg| arg != session.as_str()) {
            return Err(invalid());
        }
        args = &args[1..];
        if !args.is_empty() && args != [OsString::from("--no-daemon")] {
            return Err(invalid());
        }
        Self::parse(
            &RuntimeCommand {
                executable: command.executable.clone(),
                args: options,
            },
            cwd,
        )
    }

    pub fn thread_resume_params(&self, session: &ProviderSessionId) -> serde_json::Value {
        let mut params =
            serde_json::json!({"threadId": session.as_str(), "cwd": self.working_directory});
        // The generated resume grammar carries at most one explicit model.
        if let Some(model) = self
            .foreground
            .windows(2)
            .find(|pair| pair[0] == "-m" || pair[0] == "--model")
        {
            params["model"] = model[1].to_string_lossy().into_owned().into();
        }
        params
    }

    fn option(&mut self, name: &str, value: OsString, cwd: &Path) -> Result<(), AttachmentError> {
        if matches!(name, "-C" | "--cd") {
            let path = PathBuf::from(value);
            self.working_directory = if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            };
            return Ok(());
        }
        if matches!(name, "-s" | "--sandbox" | "-a" | "--ask-for-approval") {
            let text = value.to_str().unwrap_or("");
            let (field, allowed, option): (&mut Option<String>, &[&str], &'static str) =
                if matches!(name, "-s" | "--sandbox") {
                    (
                        &mut self.sandbox,
                        &["read-only", "workspace-write", "danger-full-access"],
                        "--sandbox",
                    )
                } else {
                    (
                        &mut self.approval,
                        &["untrusted", "on-request", "never"],
                        "--ask-for-approval",
                    )
                };
            if !allowed.contains(&text) {
                return Err(AttachmentError::UnsupportedPermission(option));
            }
            *field = Some(text.to_owned());
        } else {
            if matches!(name, "-c" | "--config") {
                // The upstream remote-resume detector treats these root keys
                // as permission overrides. Refuse their generic forms instead
                // of silently changing policy; typed flags are supported.
                // Only bare dotted keys are accepted so quoting/escapes cannot
                // conceal one of these roots from the boundary check.
                let key = value
                    .to_str()
                    .and_then(|text| text.split_once('='))
                    .map(|(key, _)| key.trim())
                    .ok_or(AttachmentError::UnsupportedConfig)?;
                if key.split('.').any(|part| {
                    part.is_empty()
                        || !part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                }) || matches!(
                    key.split('.').next(),
                    Some(
                        "approval_policy"
                            | "approvals_reviewer"
                            | "sandbox_mode"
                            | "default_permissions"
                            | "permissions"
                            | "network"
                            | "sandbox_workspace_write"
                    )
                ) {
                    return Err(AttachmentError::UnsupportedConfig);
                }
            }
            self.foreground.extend([name.into(), value.clone()]);
        }
        match name {
            "-c" | "--config" | "--enable" | "--disable" => {
                self.server.extend([name.into(), value])
            }
            _ => {
                let key = match name {
                    "-m" | "--model" => "model",
                    "-s" | "--sandbox" => "sandbox_mode",
                    "-a" | "--ask-for-approval" => "approval_policy",
                    _ => unreachable!("option names are validated by parse"),
                };
                let value = value
                    .to_str()
                    .ok_or_else(|| AttachmentError::UnsupportedArgument(value.clone()))?;
                self.server.extend([
                    "-c".into(),
                    format!(
                        "{key}={}",
                        serde_json::to_string(value).expect("string serializes")
                    )
                    .into(),
                ]);
            }
        }
        Ok(())
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }
    pub fn server_arguments(&self) -> &[OsString] {
        &self.server
    }

    pub fn foreground(
        &self,
        selected: &RuntimeCommand,
        endpoint: &str,
        thread: &ProviderSessionId,
    ) -> Result<RuntimeCommand, AttachmentError> {
        self.remote_foreground(selected, endpoint, Some(thread))
    }

    /// Let the remote TUI create its initial thread without a resume bootstrap.
    pub fn fresh_foreground(
        &self,
        selected: &RuntimeCommand,
        endpoint: &str,
    ) -> Result<RuntimeCommand, AttachmentError> {
        self.remote_foreground(selected, endpoint, None)
    }

    fn remote_foreground(
        &self,
        selected: &RuntimeCommand,
        endpoint: &str,
        thread: Option<&ProviderSessionId>,
    ) -> Result<RuntimeCommand, AttachmentError> {
        // Local transport ownership is verified by the enrollment owner. This
        // check excludes implicit shared sockets and remote/network discovery.
        if !owned_endpoint_shape(endpoint) {
            return Err(AttachmentError::InvalidEndpoint);
        }
        let mut args = Vec::new();
        if thread.is_some() {
            args.push("resume".into());
        }
        args.extend([
            "--remote".into(),
            endpoint.into(),
            "--remote-auth-token-env".into(),
            TOKEN_ENV.into(),
        ]);
        // The launcher may use its original cwd, while the owned server uses
        // the resolved cwd. An absolute -C makes both interpretations identical.
        args.extend(["-C".into(), self.working_directory.as_os_str().to_owned()]);
        args.extend(self.foreground.iter().cloned());
        if let Some(thread) = thread {
            args.push(thread.as_str().into());
        } else {
            // Fresh thread creation belongs to the TUI. Resume cannot accept
            // these overrides, so only the fresh command carries typed flags.
            for (name, value) in [
                ("--sandbox", &self.sandbox),
                ("--ask-for-approval", &self.approval),
            ] {
                if let Some(value) = value {
                    args.extend([name.into(), value.into()]);
                }
            }
        }
        Ok(RuntimeCommand {
            executable: selected.executable.clone(),
            args,
        })
    }
}

fn owned_endpoint_shape(endpoint: &str) -> bool {
    if let Some(path) = endpoint.strip_prefix("unix://") {
        return Path::new(path).is_absolute() && !path.chars().any(char::is_control);
    }
    endpoint
        .strip_prefix("ws://127.0.0.1:")
        .and_then(|port| port.parse::<u16>().ok())
        .is_some_and(|port| port != 0)
}

#[cfg(test)]
mod tests;
