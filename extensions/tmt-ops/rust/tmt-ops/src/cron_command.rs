//! Thin grammar/output adapter; all admission, mutation and notices belong to the service.
use crate::{
    config::Config,
    core::{Core, SquadError},
    cron_service::{self as service, Change, JobKey, Mutation},
    specs,
    squad::Squad,
};
use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command};
use serde_json::{Value, json};
use tmt_cli_style::{
    Terminal, Token, detail,
    list::Section,
    message,
    table::{Cell, Column, Table},
};
use tmt_squad::cron::{Schedule, ScheduleInput};

fn selected(command: Command) -> Command {
    command
        .arg(Arg::new("squad").required(true).help("Squad name"))
        .arg(Arg::new("id").required(true).help("Job id, such as c1"))
}
fn schedules(command: Command, required: bool) -> Command {
    command
        .arg(
            Arg::new("every")
                .long("every")
                .help("Elapsed interval: positive whole s, m, h or d"),
        )
        .arg(
            Arg::new("from")
                .long("from")
                .requires("every")
                .help("Anchor an interval at HH:MM"),
        )
        .arg(Arg::new("at").long("at").help("Local time HH:MM"))
        .arg(
            Arg::new("on")
                .long("on")
                .requires("at")
                .help("Days: weekdays or mon,thu"),
        )
        .arg(
            Arg::new("expression")
                .long("cron")
                .help("Standard five-field cron expression"),
        )
        .group(
            ArgGroup::new("schedule")
                .args(["every", "at", "expression"])
                .required(required),
        )
}
pub fn grammar() -> Command {
    let build = tmt_cli_style::command;
    crate::cron_clock::extend(
        build(specs::CRON)
            .subcommand_required(true)
            .arg(
                Arg::new("identity")
                    .long("identity")
                    .global(true)
                    .help("Explicit actor; otherwise verified caller, then recorded user"),
            )
            .subcommand(
                build(specs::CRON_LS).alias("list").arg(
                    Arg::new("squad")
                        .long("squad")
                        .help("List one squad instead of all squads"),
                ),
            )
            .subcommand(selected(build(specs::CRON_SHOW)))
            .subcommand(
                schedules(build(specs::CRON_ADD), true)
                    .arg(Arg::new("squad").required(true).help("Squad name"))
                    .arg(
                        Arg::new("member")
                            .required(true)
                            .help("Member to own the job"),
                    )
                    .arg(
                        Arg::new("message")
                            .required(true)
                            .help("Exact message to send"),
                    )
                    .arg(
                        Arg::new("paused")
                            .long("paused")
                            .action(ArgAction::SetTrue)
                            .help("Create the job paused"),
                    ),
            )
            .subcommand(
                schedules(selected(build(specs::CRON_EDIT)), false)
                    .arg(
                        Arg::new("message")
                            .long("message")
                            .help("Replace the exact message"),
                    )
                    .group(
                        ArgGroup::new("edit")
                            .args(["message", "every", "at", "expression"])
                            .multiple(true)
                            .required(true),
                    ),
            )
            .subcommand(selected(build(specs::CRON_RM)))
            .subcommand(selected(build(specs::CRON_PAUSE)))
            .subcommand(selected(build(specs::CRON_RESUME)))
            .subcommand(
                selected(build(specs::CRON_REASSIGN)).arg(
                    Arg::new("member")
                        .required(true)
                        .help("New owner, a current squad member"),
                ),
            ),
    )
}
fn flag<'a>(matches: &'a ArgMatches, name: &str) -> Option<&'a str> {
    matches
        .try_get_one::<String>(name)
        .ok()
        .flatten()
        .map(String::as_str)
}
fn schedule(matches: &ArgMatches, zone: &str, now_ms: i64) -> Result<Option<Schedule>, SquadError> {
    let input = if let Some(duration) = flag(matches, "every") {
        ScheduleInput::Every {
            duration,
            from: flag(matches, "from"),
        }
    } else if let Some(time) = flag(matches, "at") {
        ScheduleInput::At {
            time,
            on: flag(matches, "on"),
        }
    } else if let Some(expression) = flag(matches, "expression") {
        ScheduleInput::Cron(expression)
    } else {
        return Ok(None);
    };
    Ok(Some(Schedule::parse(input, zone, now_ms)?))
}
pub fn run(core: &Core, config: &Config, parent: &ArgMatches) -> Result<Value, SquadError> {
    let (action, flags) = parent.subcommand().expect("cron subcommand required");
    if matches!(action, "send" | "run" | "tick" | "clock") {
        return crate::cron_clock::run(core, config, parent);
    }
    let now_ms = jiff::Timestamp::now().as_millisecond();
    if action == "ls" {
        let result = service::list_jobs(core, config, flag(flags, "squad"), now_ms)?;
        return Ok(
            json!({"action":action,"jobs":result.jobs.iter().map(service::JobView::document).collect::<Vec<_>>(),"warnings":result.warnings.iter().map(SquadError::to_json).collect::<Vec<_>>() }),
        );
    }
    let selected = Squad::resolve(core, flag(flags, "squad"))?;
    let key = JobKey {
        squad: selected.name.clone(),
        room_id: selected.room_id.clone(),
        id: flag(flags, "id").unwrap_or_default().into(),
    };
    if action == "show" {
        let view = service::show_job(core, config, &key, now_ms)?;
        return Ok(
            json!({"action":action,"job":view.document(),"warnings":view.warnings.iter().map(SquadError::to_json).collect::<Vec<_>>() }),
        );
    }
    let actor = service::actor(core, config, flag(flags, "identity"))?;
    let member = || -> Result<String, SquadError> {
        let name = flag(flags, "member").expect("member required");
        let shown = core.json(&["identity", "show", name])?;
        shown["identity"]["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| {
                SquadError::new(
                    "SQUAD_CORE_UNAVAILABLE",
                    "identity show returned no member id.",
                )
            })
    };
    let change = if action == "add" {
        let zone = Schedule::local_zone()?;
        Change::Add {
            squad: selected.name,
            room_id: selected.room_id,
            owner_id: member()?,
            message: flag(flags, "message").expect("message required").into(),
            schedule: schedule(flags, &zone, now_ms)?.expect("schedule required"),
            paused: flags.get_flag("paused"),
        }
    } else {
        let shown = service::show_job(core, config, &key, now_ms)?;
        let zone = shown.job.schedule.document()["zone"]
            .as_str()
            .expect("schedule zone")
            .to_owned();
        let mutation = match action {
            "edit" => Mutation::Edit {
                message: flag(flags, "message").map(str::to_owned),
                schedule: schedule(flags, &zone, now_ms)?,
            },
            "rm" => Mutation::Remove,
            "pause" => Mutation::Pause,
            "resume" => Mutation::Resume,
            "reassign" => Mutation::Reassign {
                owner_id: member()?,
            },
            _ => unreachable!("registered cron command"),
        };
        Change::Existing {
            key,
            expected_revision: shown.job.revision,
            mutation,
        }
    };
    let result = service::apply(core, config, &actor, change, now_ms)?;
    Ok(
        json!({"action":action,"job":result.job.document(),"changed":result.changed,"warnings":result.warnings.iter().map(SquadError::to_json).collect::<Vec<_>>() }),
    )
}

pub fn text(document: &Value, terminal: Terminal) -> String {
    if matches!(
        document["action"].as_str(),
        Some("send" | "run" | "tick" | "clock")
    ) {
        return crate::cron_clock::text(document, terminal);
    }
    let mut out = Vec::new();
    let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    if document["action"] == "ls" {
        let mut squads = std::collections::BTreeMap::<String, Vec<&Value>>::new();
        for job in document["jobs"].as_array().into_iter().flatten() {
            squads.entry(text(&job["squad"])).or_default().push(job);
        }
        if squads.is_empty() {
            let _ = Section {
                title: "CRON",
                count: Some(0),
                rows: Table::new(&[Column::Detail]),
                note: Some("(no jobs yet)"),
                hint: None,
            }
            .write(&mut out, terminal);
        }
        for (name, jobs) in squads {
            let mut rows = Table::new(&[
                Column::Fixed,
                Column::Name,
                Column::Detail,
                Column::Detail,
                Column::Fixed,
                Column::Fixed,
            ]);
            for job in &jobs {
                rows.row([
                    Cell::from(text(&job["id"])),
                    Cell::from(job["owner"].as_str().unwrap_or("no owner")),
                    Cell::from(text(&job["message"])),
                    Cell::styled(text(&job["scheduleText"]), Token::Dim),
                    Cell::from(text(&job["state"])),
                    Cell::styled(next_text(job), Token::Dim),
                ]);
            }
            let _ = Section {
                title: &name.to_uppercase(),
                count: Some(jobs.len()),
                rows,
                note: None,
                hint: None,
            }
            .write(&mut out, terminal);
        }
    } else if document["action"] == "show" {
        let job = &document["job"];
        let fields = [
            "owner",
            "state",
            "scheduleText",
            "message",
            "revision",
            "pause",
        ];
        let values: Vec<_> = fields
            .iter()
            .filter(|key| **key != "pause" || !job[*key].is_null())
            .map(|key| {
                (
                    if *key == "scheduleText" {
                        "schedule".to_owned()
                    } else {
                        (*key).to_owned()
                    },
                    if job[key].is_null() {
                        "–".into()
                    } else if job[key].is_string() {
                        text(&job[key])
                    } else {
                        job[key].to_string()
                    },
                )
            })
            .chain([
                ("next".into(), next_text(job)),
                ("zone".into(), text(&job["schedule"]["zone"])),
            ])
            .collect();
        let borrowed: Vec<_> = values
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        let _ = detail::write(
            &mut out,
            terminal,
            &format!("{} {}", text(&job["squad"]), text(&job["id"])),
            &borrowed,
        );
    } else {
        let job = &document["job"];
        let verb = match document["action"].as_str() {
            Some("add") => "Added",
            Some("edit") => "Edited",
            Some("rm") => "Removed",
            Some("pause") => "Paused",
            Some("resume") => "Resumed",
            _ => "Reassigned",
        };
        let _ = message::success(
            &mut out,
            terminal,
            &format!("{verb} {} {}", text(&job["squad"]), text(&job["id"])),
        );
    }
    String::from_utf8_lossy(&out).into_owned()
}
fn next_text(job: &Value) -> String {
    let zone = job["schedule"]["zone"].as_str().unwrap_or("UTC");
    job["nextMs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
        .filter_map(|ms| {
            jiff::Timestamp::from_millisecond(ms).ok().and_then(|time| {
                jiff::tz::TimeZone::get(zone)
                    .ok()
                    .map(|zone| time.to_zoned(zone).strftime("%Y-%m-%d %H:%M").to_string())
            })
        })
        .collect::<Vec<_>>()
        .join(", ")
}
