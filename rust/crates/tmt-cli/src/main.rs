mod answer_command;
mod api_command;
mod appearance;
mod binding_command;
mod binding_error;
mod caller_context;
mod caller_session_command;
mod channel_command;
mod channel_server_command;
mod check_command;
mod completion;
mod completion_command;
mod config_command;
mod consent;
mod context_command;
mod focus_command;
mod resume_command;
use tmt_adapters::delivery;
mod diagnostics;
mod driver_command;
#[cfg(test)]
mod driver_registry_tests;
mod driver_style;
mod exchange_command;
mod extension_command;
mod extension_hooks_command;
mod extension_install_command;
mod grammar;
mod guidance_command;
mod help_output;
mod identity_command;
mod identity_context;
mod init_command;
mod install_command;
mod invocation;
mod mcp_command;
mod native_install_command;
mod native_upgrade_command;
mod notes_command;
mod office_facade;
mod output;
use tmt_adapters::pane_badge;
mod consumption_sample_command;
mod focus_hook_command;
mod parser;
mod profile_command;
mod provider_hook_command;
mod reply_notice_command;
mod request_observer_command;
mod response_command;
mod room_command;
mod run_command;
mod setup_command;
mod skill_refresh_command;
mod skill_reminder;
mod talk_command;
mod target;
mod uninstall_command;
mod workspace_command;
mod workspace_hook;

#[cfg(test)]
mod cli_style_tests;

use std::io::{self, Write};
use std::process::ExitCode;

use invocation::{Invocation, OutputMode};

fn main() -> ExitCode {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let result = match parser::parse(&args) {
        Ok(parsed) => execute(parsed),
        Err(error) => failure(error.mode, error.code, &error.message),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            // Output-stream failures cannot reliably produce another document.
            let mut stderr = tmt_cli_style::stream::stderr();
            let terminal = stderr.terminal();
            let _ = tmt_cli_style::message::error(
                &mut stderr,
                terminal,
                &format!("Could not write CLI output: {error}"),
                None,
            );
            ExitCode::FAILURE
        }
    }
}

fn execute(parsed: invocation::Parsed) -> io::Result<u8> {
    // A host driver gets everything it needs in its request; a tmt it runs
    // does nothing, so a driver can neither recurse nor change TMT's state.
    if std::env::var_os(tmt_adapters::host::external::CALL_ENV).is_some()
        && !matches!(parsed.invocation, Invocation::Help(_) | Invocation::Version)
    {
        return failure(
            parsed.mode,
            "DRIVER_CALL_REFUSED",
            "tmt runs no command for a host driver; a driver gets what it needs in its request.",
        );
    }
    // A driver's pane IDs and targets are known once its syntax is registered.
    tmt_adapters::host::external::register_approved();
    // Companions reach core through this executable, whatever name it has.
    tmt_adapters::core_executable::declare_core();
    appearance::configure(parsed.mode.json);
    let inspect_drift = skill_reminder::eligible_for_drift(&parsed);
    let driver_hint = skill_reminder::eligible_for_driver_hint(&parsed);
    let mode = parsed.mode;
    let refresh_workspace = workspace_command::eligible(&parsed.invocation);
    // This process may observe lifecycle changes for enabled extension hooks;
    // delivery runs after the command's own effects and output.
    tmt_adapters::extension_hooks::allow_capture();
    let learn_session = !matches!(
        parsed.invocation,
        Invocation::Help(_)
            | Invocation::Version
            | Invocation::ProviderHook { .. }
            | Invocation::FocusHook { .. }
            | Invocation::ReplyNoticeWorker { .. }
            | Invocation::RequestObserver { .. }
            | Invocation::Mcp { .. }
            | Invocation::Api
    );
    let code = dispatch(parsed);
    tmt_adapters::extension_hooks::deliver_pending();
    let code = code?;
    if learn_session {
        // Flush the user's result before any optional native/provider I/O.
        let _ = tmt_cli_style::stream::stdout(mode.json).flush();
        caller_session_command::observe();
    }
    if refresh_workspace {
        workspace_command::refresh(&mut tmt_cli_style::stream::stdout(mode.json));
    }
    // A Herdr pane without its driver: whatever the result, say how to
    // approve one. It replaces the passive drift line.
    if driver_hint && skill_reminder::present_driver_hint() {
        return Ok(code);
    }
    if code == 0 && inspect_drift {
        skill_reminder::present(skill_reminder::Outcome::None, mode, true);
    }
    Ok(code)
}

/// Bytes that shells and scripts read exactly: the version, completion
/// candidates and completion scripts. Never styled.
fn write_exact(
    write: impl FnOnce(&mut io::StdoutLock<'static>) -> io::Result<()>,
) -> io::Result<()> {
    write(&mut io::stdout().lock())
}

fn dispatch(parsed: invocation::Parsed) -> io::Result<u8> {
    match parsed.invocation {
        Invocation::Mcp { identity } => return mcp_command::execute(&identity),
        Invocation::Api => {
            return api_command::execute();
        }
        Invocation::Extension {
            name,
            args,
            help,
            prefix,
        } => {
            return extension_command::execute(&name, &args, help, &prefix);
        }
        // The parser has already resolved and validated this public path.
        Invocation::Help(path) => {
            let discovered = if path.is_empty() {
                extension_command::discover()?
            } else {
                extension_command::Discovered::default()
            };
            let mut stdout = tmt_cli_style::stream::stdout(false);
            let terminal = stdout.terminal();
            help_output::write(&path, &discovered, terminal, &mut stdout)?;
            if path.is_empty()
                && !parsed.mode.json
                && tmt_cli_style::Interaction::detect(parsed.mode.json).stdout
                && completion_command::needs_setup()
            {
                writeln!(
                    stdout,
                    "Tip: shell completion is not set up; run tmt completion to see the line to add."
                )?;
            }
        }
        Invocation::Version => {
            write_exact(|stdout| writeln!(stdout, "{}", env!("CARGO_PKG_VERSION")))?
        }
        Invocation::Complete(words) => write_exact(|stdout| completion::query(&words, stdout))?,
        Invocation::Completion { shell } => {
            return completion_command::execute(shell.as_deref(), parsed.mode);
        }
        Invocation::CompletionScript(shell) => {
            write_exact(|stdout| grammar::completion::generate(&shell, stdout))?;
        }
        Invocation::Config(request) => {
            return config_command::execute(request, parsed.mode);
        }
        Invocation::ExtensionInstall(request) => {
            return extension_install_command::execute(request, parsed.mode);
        }
        Invocation::ExtensionHooks(request) => {
            return extension_hooks_command::execute(request, parsed.mode);
        }
        Invocation::Driver(request) => {
            return driver_command::execute(request, parsed.mode);
        }
        Invocation::Init => {
            return init_command::execute(parsed.mode);
        }
        Invocation::Run {
            name,
            command,
            resume,
            save,
            channel,
        } => {
            let resume = resume.then(run_command::Resume::default);
            return run_command::execute(&name, &command, resume, save, channel);
        }
        Invocation::ChannelServer {
            harness,
            binding_id,
            generation,
            directory,
        } => {
            return channel_server_command::execute(&harness, &binding_id, &generation, &directory);
        }
        Invocation::Channel(request) => {
            return channel_command::execute(request, parsed.mode);
        }
        Invocation::Resume {
            name,
            forget,
            retry,
            channel,
        } => {
            if forget {
                return resume_command::forget(&name);
            }
            return run_command::execute(
                &name,
                &[],
                Some(run_command::Resume { retry }),
                false,
                channel,
            );
        }
        Invocation::Learn { skill } => {
            return guidance_command::execute(skill.as_deref());
        }
        Invocation::Install {
            target,
            directory,
            force,
        } => {
            return install_command::execute(target, directory, force, parsed.mode);
        }
        Invocation::Setup {
            provider,
            status,
            remove,
            usage,
            yes,
        } => {
            return setup_command::execute(provider, status, remove, usage, yes, parsed.mode);
        }
        Invocation::ConsumptionSample => return consumption_sample_command::execute(),
        Invocation::FocusHook {
            provider,
            launch,
            worker,
            work_budget_ms,
        } => {
            return focus_hook_command::execute(
                &provider,
                launch.as_deref(),
                worker,
                work_budget_ms,
            );
        }
        Invocation::ProviderHook {
            caller_session,
            activity_only,
            provider,
            worker,
            work_budget_ms,
        } => {
            if caller_session {
                return Ok(caller_session_command::worker(&provider, work_budget_ms));
            }
            return provider_hook_command::execute(
                &provider,
                worker,
                work_budget_ms,
                activity_only,
            );
        }
        Invocation::ReplyNoticeWorker { batch_id, log_id } => {
            return reply_notice_command::execute(&batch_id, &log_id);
        }
        Invocation::RequestObserver { request_id } => {
            return request_observer_command::execute(&request_id);
        }
        Invocation::Identity(request) => {
            return identity_command::execute(request, parsed.mode);
        }
        Invocation::Room(request) => {
            return room_command::execute(request, parsed.mode);
        }
        Invocation::NotesPath { identity } => {
            return notes_command::execute(identity, parsed.mode);
        }
        Invocation::Exchange {
            identity,
            operation,
        } => {
            return exchange_command::execute(identity, operation, parsed.mode);
        }
        request @ Invocation::Talk { .. } => {
            return talk_command::execute(request, parsed.mode);
        }
        request @ (Invocation::Role { .. } | Invocation::Preamble(_)) => {
            return profile_command::execute(request, parsed.mode);
        }
        Invocation::Check { target, lines } => {
            return check_command::execute(target, lines, parsed.mode);
        }
        Invocation::Focus { target } => {
            return focus_command::execute(target, parsed.mode);
        }
        Invocation::FocusClient => {
            return focus_command::client(parsed.mode);
        }
        request @ (Invocation::Reply { .. } | Invocation::Result { .. }) => {
            return response_command::execute(request, parsed.mode);
        }
        request @ (Invocation::Inbox { .. } | Invocation::Answer { .. }) => {
            return answer_command::execute(request, parsed.mode);
        }
        Invocation::WhoamiContext => {
            return context_command::execute(parsed.mode);
        }
        request @ (Invocation::Bind { .. }
        | Invocation::BindMarked { .. }
        | Invocation::Whoami
        | Invocation::Unbind
        | Invocation::Remove { .. }
        | Invocation::Rename { .. }
        | Invocation::List { .. }) => {
            return binding_command::execute(request, parsed.mode);
        }
        Invocation::Upgrade {
            channel,
            exact,
            unpin,
            yes,
            allow_schema_ahead,
        } => {
            return native_upgrade_command::execute_with_schema_consent(
                channel,
                exact.as_deref(),
                unpin,
                yes,
                allow_schema_ahead,
                parsed.mode,
            );
        }
        Invocation::NativeUpgradeExtensions { plan } => {
            return extension_install_command::upgrade_all::execute(plan, parsed.mode);
        }
        Invocation::NativeRefreshSkills { managed } => {
            return skill_refresh_command::execute(managed, parsed.mode);
        }
        Invocation::Uninstall { purge, yes, prefix } => {
            return uninstall_command::execute(purge, yes, prefix.as_deref(), parsed.mode);
        }
        Invocation::Office { prefix, operation } => {
            return office_facade::execute(prefix, operation, parsed.mode);
        }
        Invocation::NativeInstallHandoff { probe, version } => {
            return native_install_command::handoff(version, probe, parsed.mode);
        }
        Invocation::NativeSchema { source_sha } => {
            return native_install_command::schema(&source_sha, parsed.mode);
        }
        Invocation::NativeInstall {
            product,
            archive,
            manifest,
            prefix,
            channel,
            pin,
        } => {
            return native_install_command::execute(
                product,
                &archive,
                &manifest,
                &prefix,
                channel,
                pin,
                parsed.mode,
            );
        }
    }
    Ok(0)
}

fn failure(mode: OutputMode, code: &'static str, message: &str) -> io::Result<u8> {
    // Parse and existing configuration failures retain their exit-1 contract.
    output::Failure::new(code, message, 1).publish(mode)
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "Tip: shell completion is not set up; run tmt completion to see the line to add.",
        &[" to see the line to add."],
        &[],
    ),
    crate::cli_style_tests::HintSpec::skipped(
        "tmt runs no command for a host driver; a driver gets what it needs in its request.",
        "Executable or option reference in prose, not a full command suggestion.",
    ),
];
