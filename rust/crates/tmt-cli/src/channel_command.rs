//! `tmt channel inspect|recover`: show one binding's message-channel enrollments,
//! and remove an abandoned one on the user's explicit request. Each driver owns
//! its records and the recovery rule (the channel contracts' "Recovery"
//! sections); this command resolves the binding and presents the outcome. It
//! never signals a process, sends, resends or pastes.

use crate::{
    invocation::{ChannelRequest, EnrollmentSelector, OutputMode},
    output::{Failure, after_cleanup},
    target,
};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    host::{CallerEnvironment, Host},
    runtime::{
        RuntimeRegistry,
        channel::{
            EnrollmentReport, EnrollmentState, EvidenceError, ProcessRole, ProcessState, Recovery,
            RecoveryError, inspect_command, recover_command, valid_enrollment_id,
        },
    },
    storage::Storage,
};
use tmt_cli_style::{Terminal, message};

/// One absolute bound for every observation a command makes.
const OBSERVATION_DEADLINE: Duration = Duration::from_secs(5);

pub fn execute(request: ChannelRequest, mode: OutputMode) -> io::Result<u8> {
    let outcome = validate(&request)
        .and_then(|()| binding(request.selector.clone()))
        .and_then(|(paths, binding_id)| {
            let directory = paths.channel_directory();
            let registry = RuntimeRegistry::first_party();
            let deadline = Instant::now() + OBSERVATION_DEADLINE;
            match &request.recover {
                None => inspect(&registry, &directory, &binding_id, deadline).map(|reports| {
                    Outcome::Inspected {
                        binding_id,
                        reports,
                    }
                }),
                Some(generation) => {
                    recover(&registry, &directory, &binding_id, generation, deadline)
                }
            }
        });
    match outcome {
        Ok(outcome) => publish(&outcome, mode),
        Err(error) => error.publish(mode),
    }
}

/// Binding IDs and generations are UUIDs, as talk and inspect print them; a
/// malformed one is a usage error before any lookup.
fn validate(request: &ChannelRequest) -> Result<(), Failure> {
    let values = [
        match &request.selector {
            EnrollmentSelector::Binding(binding_id) => Some(("--binding", binding_id)),
            EnrollmentSelector::Target(_) => None,
        },
        request
            .recover
            .as_ref()
            .map(|generation| ("--generation", generation)),
    ];
    for (option, value) in values.into_iter().flatten() {
        if !valid_enrollment_id(value) {
            return Err(Failure::new(
                "USAGE_ERROR",
                format!(
                    "{option} takes the UUID that talk or tmt channel inspect printed, not {value:?}."
                ),
                1,
            ));
        }
    }
    Ok(())
}

enum Outcome {
    Inspected {
        binding_id: String,
        reports: Vec<(String, EnrollmentReport)>,
    },
    Recovered {
        driver: String,
        report: Box<EnrollmentReport>,
        removed: Vec<PathBuf>,
        kept: Vec<PathBuf>,
    },
    /// No enrollment of the binding is on record: nothing to do.
    Absent {
        binding_id: String,
        generation: String,
    },
}

/// The binding to act on. A target goes through the shared resolver and must
/// name an active binding; `--binding` is taken as given, since a talk error
/// names a binding that observation may already have deleted.
fn binding(selector: EnrollmentSelector) -> Result<(ConfigPaths, String), Failure> {
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let target = match selector {
        EnrollmentSelector::Binding(binding_id) => return Ok((paths, binding_id)),
        EnrollmentSelector::Target(target) => target,
    };
    let host = Host::for_caller(&CallerEnvironment::current());
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "Nothing changed; retrying the identical command is safe.",
            "IDENTITY_ERROR",
            "Could not open identity storage.",
        )
    })?;
    let pending = target::resolve(&mut storage, &host, &target).and_then(|observed| {
        observed.binding.map(|binding| binding.id).ok_or_else(|| {
            Failure::new(
                "BINDING_NOT_FOUND",
                format!("'{target}' has no binding, so it names no channel enrollment."),
                3,
            )
            .suggestion("Use --binding with the binding ID a talk error printed.".to_owned())
        })
    });
    let binding_id = after_cleanup(pending, || storage.close()).map_err(|error| {
        if error.code == "NAME_NOT_FOUND" {
            error.suggestion(
                "An enrollment outlives its binding: use --binding with the ID a talk error printed."
                    .to_owned(),
            )
        } else {
            error
        }
    })?;
    Ok((paths, binding_id))
}

fn inspect(
    registry: &RuntimeRegistry,
    directory: &std::path::Path,
    binding_id: &str,
    deadline: Instant,
) -> Result<Vec<(String, EnrollmentReport)>, Failure> {
    let mut reports = Vec::new();
    for (harness, channel) in registry.channels() {
        match channel.inspect(directory, binding_id, deadline) {
            Ok(Some(report)) => reports.push((harness.as_str().to_owned(), report)),
            Ok(None) => {}
            Err(error) => return Err(invalid(&error)),
        }
    }
    Ok(reports)
}

/// Every driver is asked; only the one whose record carries `generation` can
/// remove anything, and the others answer without a change.
fn recover(
    registry: &RuntimeRegistry,
    directory: &std::path::Path,
    binding_id: &str,
    generation: &str,
    deadline: Instant,
) -> Result<Outcome, Failure> {
    let mut other = None;
    let mut unreadable = None;
    for (harness, channel) in registry.channels() {
        let driver = harness.as_str();
        match channel.recover(directory, binding_id, generation, deadline) {
            Ok(Recovery::Recovered {
                report,
                removed,
                kept,
            }) => {
                return Ok(Outcome::Recovered {
                    driver: driver.to_owned(),
                    report: Box::new(report),
                    removed,
                    kept,
                });
            }
            Ok(Recovery::Absent) => {}
            Ok(Recovery::OtherGeneration(report)) => other = Some((driver.to_owned(), report)),
            Err(RecoveryError::Invalid(error)) => unreadable = Some(error),
            Err(error) => return Err(refusal(driver, &error)),
        }
    }
    if let Some(error) = unreadable {
        return Err(invalid(&error));
    }
    if let Some((driver, report)) = other {
        return Err(Failure::new(
            "CHANNEL_ENROLLMENT_CHANGED",
            format!(
                "Binding {binding_id}'s {driver} enrollment is generation {}, not {generation}. Nothing was removed.",
                report.generation
            ),
            1,
        )
        .suggestion(format!(
            "Check it with: {}",
            inspect_command(binding_id)
        )));
    }
    Ok(Outcome::Absent {
        binding_id: binding_id.to_owned(),
        generation: generation.to_owned(),
    })
}

fn invalid(error: &EvidenceError) -> Failure {
    Failure::new("CHANNEL_ENROLLMENT_INVALID", error.message(), 1)
}

fn refusal(driver: &str, error: &RecoveryError) -> Failure {
    let named = |report: &EnrollmentReport| {
        format!(
            "The {driver} enrollment {} of binding {}",
            report.generation, report.binding_id
        )
    };
    let process = |report: &EnrollmentReport, state| {
        report
            .processes
            .iter()
            .find(|process| process.state == state)
            .map(|process| format!("its {} (pid {})", role_label(process.role), process.pid))
            .unwrap_or_else(|| "a recorded process".to_owned())
    };
    match error {
        RecoveryError::Running(report) => Failure::new(
            "CHANNEL_ENROLLMENT_LIVE",
            format!(
                "{} is in use: {} is running. Nothing was removed.",
                named(report),
                process(report, ProcessState::Present)
            ),
            1,
        )
        .suggestion(
            "Recovery never stops a process; it applies only after that launch has ended."
                .to_owned(),
        ),
        RecoveryError::Unverifiable(report) => Failure::new(
            "CHANNEL_ENROLLMENT_UNVERIFIABLE",
            format!(
                "{}: {} cannot be observed, so it cannot be proven gone. Nothing was removed.",
                named(report),
                process(report, ProcessState::Unobservable)
            ),
            1,
        )
        .suggestion(
            "Retry; recovery refuses until every recorded process can be observed.".to_owned(),
        ),
        RecoveryError::Changed(report) => Failure::new(
            "CHANNEL_ENROLLMENT_CHANGED",
            format!(
                "{} changed while it was checked. Nothing was removed.",
                named(report)
            ),
            1,
        )
        .suggestion(format!(
            "Check it again with: {}",
            inspect_command(&report.binding_id)
        )),
        RecoveryError::Invalid(error) => invalid(error),
        RecoveryError::Failed { path } => Failure::new(
            "CHANNEL_RECOVERY_FAILED",
            format!("Recovery could not finish at {}.", path.display()),
            1,
        )
        .suggestion("Running the same command again is safe.".to_owned()),
    }
}

fn role_label(role: ProcessRole) -> &'static str {
    match role {
        ProcessRole::LaunchOwner => "launch owner",
        ProcessRole::Foreground => "foreground",
        ProcessRole::Provider => "provider",
        ProcessRole::Endpoint => "endpoint",
    }
}

fn role_key(role: ProcessRole) -> &'static str {
    match role {
        ProcessRole::LaunchOwner => "launchOwner",
        ProcessRole::Foreground => "foreground",
        ProcessRole::Provider => "provider",
        ProcessRole::Endpoint => "endpoint",
    }
}

fn process_state(state: ProcessState) -> &'static str {
    match state {
        ProcessState::Present => "running",
        ProcessState::Gone => "gone",
        ProcessState::Unobservable => "unobservable",
    }
}

fn enrollment_state(state: EnrollmentState) -> &'static str {
    match state {
        EnrollmentState::Running => "running",
        EnrollmentState::Unverifiable => "unverifiable",
        EnrollmentState::Unconfirmed => "unconfirmed",
        EnrollmentState::Ended => "ended",
    }
}

fn paths(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect()
}

/// The exact recovery, only for a state recovery accepts.
fn next_step(report: &EnrollmentReport) -> Option<String> {
    matches!(
        report.state(),
        EnrollmentState::Unconfirmed | EnrollmentState::Ended
    )
    .then(|| recover_command(&report.binding_id, &report.generation))
}

fn report_document(driver: &str, report: &EnrollmentReport) -> Value {
    json!({
        "driver": driver,
        "record": report.record.display().to_string(),
        "identityId": report.identity_id,
        "generation": report.generation,
        "state": enrollment_state(report.state()),
        "pane": report.pane.as_ref().map(|pane| json!({
            "host": pane.host,
            "serverId": pane.server_id,
            "socketPath": pane.socket_path,
            "paneId": pane.pane_id,
            "panePid": pane.pane_pid,
        })),
        "processes": report.processes.iter().map(|process| json!({
            "role": role_key(process.role),
            "pid": process.pid,
            "start": process.start,
            "state": process_state(process.state),
        })).collect::<Vec<_>>(),
        "foregroundRecorded": report.foreground_recorded,
        "verification": report.verification,
        "removes": paths(&report.removes),
        "keeps": paths(&report.keeps),
        "recover": next_step(report),
    })
}

fn publish(outcome: &Outcome, mode: OutputMode) -> io::Result<u8> {
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        let document = match outcome {
            Outcome::Inspected {
                binding_id,
                reports,
            } => json!({
                "bindingId": binding_id,
                "enrollments": reports
                    .iter()
                    .map(|(driver, report)| report_document(driver, report))
                    .collect::<Vec<_>>(),
            }),
            Outcome::Recovered {
                driver,
                report,
                removed,
                kept,
            } => json!({
                "bindingId": report.binding_id,
                "generation": report.generation,
                "driver": driver,
                "recovered": true,
                "removed": paths(removed),
                "kept": paths(kept),
            }),
            Outcome::Absent {
                binding_id,
                generation,
            } => json!({
                "bindingId": binding_id,
                "generation": generation,
                "recovered": false,
            }),
        };
        writeln!(stdout, "{document}")?;
        return Ok(0);
    }
    let terminal = stdout.terminal();
    match outcome {
        Outcome::Inspected {
            binding_id,
            reports,
        } => {
            if reports.is_empty() {
                writeln!(
                    stdout,
                    "No channel enrollment is recorded for binding {binding_id}."
                )?;
            }
            for (driver, report) in reports {
                write_report(&mut stdout, terminal, driver, report)?;
            }
        }
        Outcome::Recovered {
            driver,
            report,
            removed,
            kept,
        } => {
            message::success(
                &mut stdout,
                terminal,
                &format!(
                    "Recovered the {driver} channel enrollment {} of binding {}",
                    report.generation, report.binding_id
                ),
            )?;
            for path in removed {
                writeln!(stdout, "  removed  {}", path.display())?;
            }
            if !kept.is_empty() {
                message::warning(
                    &mut stdout,
                    terminal,
                    &format!("Left in place: {}", paths(kept).join(", ")),
                    Some("Nothing proves these unused; remove them yourself only after checking."),
                )?;
            }
        }
        Outcome::Absent { binding_id, .. } => {
            writeln!(
                stdout,
                "No channel enrollment is recorded for binding {binding_id}; nothing to recover."
            )?;
        }
    }
    Ok(0)
}

fn write_report(
    output: &mut impl Write,
    terminal: Terminal,
    driver: &str,
    report: &EnrollmentReport,
) -> io::Result<()> {
    let mut fields = vec![
        ("binding", report.binding_id.clone()),
        (
            "identity",
            report.identity_id.clone().unwrap_or_else(|| "-".into()),
        ),
        ("generation", report.generation.clone()),
        ("state", enrollment_state(report.state()).to_owned()),
        (
            "pane",
            report.pane.as_ref().map_or_else(
                || "not recorded".to_owned(),
                |pane| {
                    format!(
                        "{} on {} (pane process {})",
                        pane.pane_id, pane.socket_path, pane.pane_pid
                    )
                },
            ),
        ),
    ];
    for process in &report.processes {
        fields.push((
            role_label(process.role),
            format!(
                "pid {}, started {}: {}",
                process.pid,
                process.start,
                process_state(process.state)
            ),
        ));
    }
    if !report.foreground_recorded {
        fields.push(("foreground", "not recorded".to_owned()));
    }
    fields.push(("record", report.record.display().to_string()));
    if report.removes.len() > 1 {
        fields.push(("also removes", paths(&report.removes[1..]).join(", ")));
    }
    if !report.keeps.is_empty() {
        fields.push(("keeps", paths(&report.keeps).join(", ")));
    }
    fields.push(("verify", report.verification.clone()));
    tmt_cli_style::detail::write(
        output,
        terminal,
        &format!("{driver} channel enrollment"),
        &fields,
    )?;
    let hint = match report.state() {
        EnrollmentState::Running => "It is in use, so there is nothing to recover.".to_owned(),
        EnrollmentState::Unverifiable => {
            "Retry when its processes can be observed; recovery refuses until then.".to_owned()
        }
        EnrollmentState::Unconfirmed => format!(
            "After checking the pane: {}",
            recover_command(&report.binding_id, &report.generation)
        ),
        EnrollmentState::Ended => format!(
            "Recover it with: {}",
            recover_command(&report.binding_id, &report.generation)
        ),
    };
    message::hint(output, terminal, &hint)
}
