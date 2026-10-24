//! Typed re-entry into the ordinary managed launcher on a new member PTY.
//! Only the supervisor is detached; run_bound keeps foreground ownership inside it.

use super::{RunRequest, diagnostic};
use crate::{invocation::ChannelMode, output::Failure};
use std::ffi::OsString;
use tmt_adapters::config::ConfigPaths;
use tmt_core::host::HostKind;

fn arguments(request: RunRequest<'_>) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec![
        if request.resume.is_some() {
            "resume".into()
        } else {
            "run".into()
        },
        "--host".into(),
        HostKind::pty().as_str().into(),
    ];
    match request.channel {
        ChannelMode::Disabled => argv.push("--no-channel".into()),
        ChannelMode::Required => argv.push("--channel".into()),
        ChannelMode::Default => {}
    }
    if let Some(resume) = request.resume {
        if resume.move_running {
            argv.push("--move".into());
        }
        if resume.retry {
            argv.push("--retry".into());
        }
        if resume.fresh {
            argv.push("--new".into());
        }
        if let Some(driver) = resume.driver {
            argv.extend(["--driver".into(), driver.into()]);
        }
        if let Some(model) = &request.options.model {
            argv.extend(["--model".into(), model.into()]);
        }
        if let Some(effort) = &request.options.effort {
            argv.extend(["--effort".into(), effort.into()]);
        }
    } else if request.save {
        argv.push("--save".into());
    }
    argv.extend(["--".into(), request.name.into()]);
    argv.extend_from_slice(request.command);
    argv
}

pub(super) fn start(request: RunRequest<'_>, paths: &ConfigPaths) -> Result<u8, Failure> {
    let executable = std::env::current_exe().map_err(|error| {
        Failure::new(
            "PTY_LAUNCH_FAILED",
            "No member was launched: the Core executable could not be located.",
            1,
        )
        .caused_by(error)
    })?;
    let started = tmt_adapters::pty::start_member(executable.as_os_str(), &arguments(request), paths)
        .map_err(|error| {
            let failure = match &error {
                tmt_adapters::pty::LaunchError::NotStarted(_) => Failure::new(
                    "PTY_LAUNCH_FAILED", "No member was launched: built-in host startup failed.", 1),
                tmt_adapters::pty::LaunchError::Unconfirmed {pane, ..} => Failure::new(
                    "PTY_LAUNCH_FAILED", "The built-in host launch was not confirmed; a member may have started. Do not blindly rerun.", 1)
                    .suggestion(format!("Inspect the pane first: tmt check -- {}", crate::output::shell_word(pane))),
            };
            failure.caused_by(error)
        })?;
    if let Some(warning) = &started.cleanup_warning {
        diagnostic(warning);
    }
    diagnostic(&format!(
        "started {} on host {} ({})",
        started.name,
        HostKind::pty().as_str(),
        started.pane
    ));
    Ok(0)
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] =
    &[crate::cli_style_tests::HintSpec::core(
        "Inspect the pane first: tmt check -- {}",
        &[""],
        &[],
    )];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{invocation::ChannelMode, run_command::Resume};
    use tmt_adapters::runtime::launch_preset::LaunchSettings;

    #[test]
    fn reentry_preserves_opaque_command_words_and_launch_policy() {
        let options = LaunchSettings::default();
        let command = vec![
            "fake-provider".into(),
            "a b".into(),
            "--host".into(),
            "literal".into(),
        ];
        let argv = arguments(RunRequest {
            name: "member",
            options: &options,
            command: &command,
            resume: None,
            save: true,
            channel: ChannelMode::Disabled,
        });
        assert_eq!(
            argv,
            vec![
                "run",
                "--host",
                HostKind::pty().as_str(),
                "--no-channel",
                "--save",
                "--",
                "member",
                "fake-provider",
                "a b",
                "--host",
                "literal"
            ]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
        );
    }
    #[test]
    fn a_fresh_resume_reenters_with_new() {
        let argv = arguments(RunRequest {
            name: "member",
            options: &LaunchSettings::default(),
            command: &[],
            resume: Some(Resume {
                driver: None,
                retry: false,
                move_running: false,
                fresh: true,
            }),
            save: false,
            channel: ChannelMode::Disabled,
        });
        assert_eq!(
            argv,
            [
                "resume",
                "--host",
                HostKind::pty().as_str(),
                "--no-channel",
                "--new",
                "--",
                "member"
            ]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
        );
    }
    #[test]
    fn resume_reentry_keeps_driver_move_retry_and_model_preferences() {
        let options = LaunchSettings {
            model: Some("model".into()),
            effort: Some("high".into()),
        };
        let argv = arguments(RunRequest {
            name: "member",
            options: &options,
            command: &[],
            resume: Some(Resume {
                driver: Some("driver"),
                retry: true,
                move_running: true,
                fresh: false,
            }),
            save: false,
            channel: ChannelMode::Default,
        });
        assert_eq!(
            argv,
            vec![
                "resume",
                "--host",
                HostKind::pty().as_str(),
                "--move",
                "--retry",
                "--driver",
                "driver",
                "--model",
                "model",
                "--effort",
                "high",
                "--",
                "member"
            ]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
        );
    }
}
