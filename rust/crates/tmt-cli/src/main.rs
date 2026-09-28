mod binding_command;
mod binding_error;
mod caller_context;
mod check_command;
mod completion;
mod config_command;
mod context_command;
use tmt_adapters::delivery;
mod diagnostics;
mod exchange_command;
mod extension_command;
mod grammar;
mod guidance_command;
mod identity_command;
mod identity_context;
mod init_command;
mod install_command;
mod invocation;
mod native_install_command;
mod native_upgrade_command;
mod notes_command;
mod office_avatar_command;
mod office_block_command;
mod office_board_command;
mod office_command;
mod office_extension_command;
mod office_layout_command;
mod office_pairing_command;
mod office_profile_command;
mod office_prop_command;
mod office_whiteboard_command;
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
            let _ = writeln!(io::stderr().lock(), "Could not write CLI output: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute(parsed: invocation::Parsed) -> io::Result<u8> {
    let inspect_drift = skill_reminder::eligible_for_drift(&parsed);
    let mode = parsed.mode;
    let code = dispatch(parsed)?;
    if code == 0 && inspect_drift {
        skill_reminder::present(skill_reminder::Outcome::None, mode, true);
    }
    Ok(code)
}

fn dispatch(parsed: invocation::Parsed) -> io::Result<u8> {
    let mut stdout = io::stdout().lock();
    match parsed.invocation {
        Invocation::Extension {
            name,
            args,
            help,
            prefix,
        } => {
            drop(stdout);
            return extension_command::execute(&name, &args, help, &prefix);
        }
        Invocation::Help(path) => {
            if path.is_empty() {
                writeln!(
                    stdout,
                    "TMT native alpha — collaborate with terminal agents through durable exchanges.\nRun tmt install to set up agent skills; managed installations use tmt upgrade.\n"
                )?;
            }
            // The parser has already resolved and validated this public path.
            grammar::help_command(&path)
                .map_err(io::Error::other)?
                .write_help(&mut stdout)?;
            writeln!(stdout)?;
            if path.is_empty() {
                extension_command::write_discovered(&mut stdout)?;
            }
        }
        Invocation::Version => writeln!(stdout, "{}", env!("CARGO_PKG_VERSION"))?,
        Invocation::Complete(words) => completion::query(&words, &mut stdout)?,
        Invocation::Completion(shell) => {
            grammar::completion::generate(shell.as_deref().unwrap_or(""), &mut stdout)?;
        }
        Invocation::Config(request) => {
            drop(stdout);
            return config_command::execute(request, parsed.mode);
        }
        Invocation::Init => {
            drop(stdout);
            return init_command::execute(parsed.mode);
        }
        Invocation::Run {
            name,
            command,
            resume,
            save,
        } => {
            drop(stdout);
            return run_command::execute(&name, &command, resume, save);
        }
        Invocation::Learn { skill } => {
            drop(stdout);
            return guidance_command::execute(skill.as_deref());
        }
        Invocation::Install {
            target,
            directory,
            force,
        } => {
            drop(stdout);
            return install_command::execute(target, directory, force, parsed.mode);
        }
        Invocation::Setup {
            provider,
            remove,
            yes,
        } => {
            drop(stdout);
            return setup_command::execute(provider, remove, yes, parsed.mode);
        }
        Invocation::ProviderHook { provider, worker } => {
            drop(stdout);
            return provider_hook_command::execute(&provider, worker);
        }
        Invocation::RequestObserver { request_id } => {
            drop(stdout);
            return request_observer_command::execute(&request_id);
        }
        Invocation::Identity(request) => {
            drop(stdout);
            return identity_command::execute(request, parsed.mode);
        }
        Invocation::Room(request) => {
            drop(stdout);
            return room_command::execute(request, parsed.mode);
        }
        Invocation::NotesPath { identity } => {
            drop(stdout);
            return notes_command::execute(identity, parsed.mode);
        }
        Invocation::Exchange {
            identity,
            operation,
        } => {
            drop(stdout);
            return exchange_command::execute(identity, operation, parsed.mode);
        }
        request @ Invocation::Talk { .. } => {
            drop(stdout);
            return talk_command::execute(request, parsed.mode);
        }
        request @ (Invocation::Role { .. } | Invocation::Preamble(_)) => {
            drop(stdout);
            return profile_command::execute(request, parsed.mode);
        }
        Invocation::Check { target, lines } => {
            drop(stdout);
            return check_command::execute(target, lines, parsed.mode);
        }
        request @ (Invocation::Reply { .. } | Invocation::Result { .. }) => {
            drop(stdout);
            return response_command::execute(request, parsed.mode);
        }
        Invocation::WhoamiContext => {
            drop(stdout);
            return context_command::execute(parsed.mode);
        }
        request @ (Invocation::Bind { .. }
        | Invocation::BindMarked { .. }
        | Invocation::Whoami
        | Invocation::Unbind
        | Invocation::Remove { .. }
        | Invocation::List { .. }) => {
            drop(stdout);
            return binding_command::execute(request, parsed.mode);
        }
        Invocation::Upgrade {
            channel,
            exact,
            unpin,
        } => {
            drop(stdout);
            return native_upgrade_command::execute(channel, exact.as_deref(), unpin, parsed.mode);
        }
        Invocation::NativeRefreshSkills => {
            drop(stdout);
            return skill_refresh_command::execute(parsed.mode);
        }
        Invocation::Office { prefix, operation } => {
            drop(stdout);
            return office_command::execute(prefix, operation, parsed.mode);
        }
        Invocation::NativeInstall {
            product,
            archive,
            manifest,
            prefix,
            channel,
            pin,
        } => {
            drop(stdout);
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
