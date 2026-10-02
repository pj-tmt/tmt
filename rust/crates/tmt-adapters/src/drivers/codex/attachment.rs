//! Provider-owned foreground command planning. Remote attachment resumes one
//! exact thread and does not supply an initial prompt, fork, or default socket.

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

    pub fn thread_start_params(&self) -> serde_json::Value {
        let mut params = serde_json::json!({
            "cwd": self.working_directory,
            "allowProviderModelFallback": false
        });
        if let Some(sandbox) = &self.sandbox {
            params["sandbox"] = sandbox.clone().into();
        }
        if let Some(approval) = &self.approval {
            params["approvalPolicy"] = approval.clone().into();
        }
        params
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
        // Local transport ownership is verified by the enrollment owner. This
        // check excludes implicit shared sockets and remote/network discovery.
        if !owned_endpoint_shape(endpoint) {
            return Err(AttachmentError::InvalidEndpoint);
        }
        let mut args = vec![
            "resume".into(),
            "--remote".into(),
            endpoint.into(),
            "--remote-auth-token-env".into(),
            TOKEN_ENV.into(),
        ];
        // The launcher may use its original cwd, while the owned server uses
        // the resolved cwd. An absolute -C makes both interpretations identical.
        args.extend(["-C".into(), self.working_directory.as_os_str().to_owned()]);
        args.extend(self.foreground.iter().cloned());
        args.push(thread.as_str().into());
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
