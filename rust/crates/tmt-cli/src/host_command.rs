//! `tmt host`: what each terminal host offers (#4580). `templates` reads every host's default
//! launch template (tmux's own, then each approved driver's) and creates nothing. The rules for
//! which driver offers one belong to `tmt_adapters::host::launch`; this is its front end.

use crate::driver_command::running_tmt;
use crate::invocation::{HostRequest, OutputMode};
use crate::output::{Failure, shell_word};
use serde_json::json;
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    driver_protocol::registry,
    host::{
        launch::{HostLaunch, HostSource, LaunchTemplate},
        launch_templates,
    },
    process::UnixCommandRunner,
};
use tmt_cli_style::{detail, palette::Terminal};

fn read() -> Result<Vec<HostLaunch>, Failure> {
    let paths = ConfigPaths::discover()?;
    let tmt = running_tmt()?;
    let records = registry::read(&paths.global_dir)
        .map_err(|error| Failure::new(error.code(), error.to_string(), 1))?;
    let drivers: Vec<_> = records
        .into_iter()
        .map(|record| {
            let (state, _) = registry::state(&paths.global_dir, &record, &tmt, &UnixCommandRunner);
            (record, state)
        })
        .collect();
    Ok(launch_templates(&drivers))
}

fn document(hosts: &[HostLaunch]) -> serde_json::Value {
    json!({
        "version": 1,
        "hosts": hosts.iter().map(|host| json!({
            "host": host.host,
            "state": match host.source {
                HostSource::BuiltIn => "builtin",
                HostSource::Driver(state) => state.as_str(),
            },
            "launch": host.launch.as_ref().map(|launch| json!({
                "argv": launch.argv(),
                "placeholders": LaunchTemplate::PLACEHOLDERS,
            })),
        })).collect::<Vec<_>>(),
    })
}

fn source_text(source: HostSource) -> String {
    match source {
        HostSource::BuiltIn => "built in".into(),
        HostSource::Driver(state) => format!("driver ({})", state.as_str()),
    }
}

/// The command line as typed, each placeholder left bare so it reads as one.
fn command_line(launch: &LaunchTemplate) -> String {
    launch
        .argv()
        .iter()
        .map(|word| {
            if LaunchTemplate::is_placeholder(word) {
                word.clone()
            } else {
                shell_word(word)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn write_hosts(
    output: &mut impl Write,
    terminal: Terminal,
    hosts: &[HostLaunch],
) -> io::Result<()> {
    for (index, host) in hosts.iter().enumerate() {
        if index > 0 {
            writeln!(output)?;
        }
        let template = host
            .launch
            .as_ref()
            .map_or_else(|| "none".to_owned(), command_line);
        detail::write(
            output,
            terminal,
            &format!("Host {}", host.host),
            &[
                ("source", source_text(host.source)),
                ("opens a pane with", template),
            ],
        )?;
    }
    Ok(())
}

pub fn execute(request: HostRequest, mode: OutputMode) -> io::Result<u8> {
    let HostRequest::Templates = request;
    match read() {
        Ok(hosts) => {
            let mut output = tmt_cli_style::stream::stdout(mode.json);
            let terminal = output.terminal();
            if mode.json {
                writeln!(output, "{}", document(&hosts))?;
            } else {
                write_hosts(&mut output, terminal, &hosts)?;
            }
            Ok(0)
        }
        Err(error) => error.publish(mode),
    }
}
