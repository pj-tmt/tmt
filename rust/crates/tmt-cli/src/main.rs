mod answer_command;
mod api_command;
mod binding_command;
mod binding_error;
mod caller_context;
mod check_command;
mod completion;
mod config_command;
mod consent;
mod context_command;
mod focus_command;
mod resume_command;
use tmt_adapters::delivery;
mod diagnostics;
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
mod native_install_command;
mod native_upgrade_command;
mod notes_command;
mod office_facade;
mod output;
use tmt_adapters::pane_badge;
mod parser;
mod profile_command;
mod provider_hook_command;
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
    // Companions reach core through this executable, whatever name it has.
    tmt_adapters::core_executable::declare_core();
    let inspect_drift = skill_reminder::eligible_for_drift(&parsed);
    let mode = parsed.mode;
    // This process may observe lifecycle changes for enabled extension hooks;
    // delivery runs after the command's own effects and output.
    tmt_adapters::extension_hooks::allow_capture();
    let code = dispatch(parsed);
    tmt_adapters::extension_hooks::deliver_pending();
    let code = code?;
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
        }
        Invocation::Version => {
            write_exact(|stdout| writeln!(stdout, "{}", env!("CARGO_PKG_VERSION")))?
        }
        Invocation::Complete(words) => write_exact(|stdout| completion::query(&words, stdout))?,
        Invocation::Completion(shell) => {
            write_exact(|stdout| {
                grammar::completion::generate(shell.as_deref().unwrap_or(""), stdout)
            })?;
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
        Invocation::Init => {
            return init_command::execute(parsed.mode);
        }
        Invocation::Run {
            name,
            command,
            resume,
            save,
        } => {
            let resume = resume.then(run_command::Resume::default);
            return run_command::execute(&name, &command, resume, save);
        }
        Invocation::Resume {
            name,
            forget,
            retry,
        } => {
            if forget {
                return resume_command::forget(&name);
            }
            return run_command::execute(&name, &[], Some(run_command::Resume { retry }), false);
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
            remove,
            usage,
            yes,
        } => {
            return setup_command::execute(provider, remove, usage, yes, parsed.mode);
        }
        Invocation::ProviderHook { provider, worker } => {
            return provider_hook_command::execute(&provider, worker);
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
        } => {
            return native_upgrade_command::execute(channel, exact.as_deref(), unpin, parsed.mode);
        }
        Invocation::NativeRefreshSkills => {
            return skill_refresh_command::execute(parsed.mode);
        }
        Invocation::Uninstall { purge, yes, prefix } => {
            return uninstall_command::execute(purge, yes, prefix.as_deref(), parsed.mode);
        }
        Invocation::Office { prefix, operation } => {
            return office_facade::execute(prefix, operation, parsed.mode);
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
