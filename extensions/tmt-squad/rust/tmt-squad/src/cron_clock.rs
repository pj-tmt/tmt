//! Clock commands and lifecycle; all job/actor admission stays in cron_service.
use crate::{
    config::Config,
    core::{Core, SquadError},
    cron_service::{self as service, CronActor, JobKey},
    runner::Cancellation,
    specs,
    squad::Squad,
};
use clap::{Arg, ArgMatches, Command};
use serde_json::{Value, json};
use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::Duration,
};
use tmt_cli_style::{Terminal, detail, message};
use tmt_squad::cron::{self, Clock, ClockStatus, Dispatch, Job, Lease, Store, Tick};

const EVERY: Duration = Duration::from_secs(1);

fn now() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}
fn failed(code: &str, message: &str) -> SquadError {
    SquadError::new(code, message)
}

pub fn extend(command: Command) -> Command {
    let build = tmt_cli_style::command;
    command
        .subcommand(
            build(specs::CRON_SEND)
                .arg(Arg::new("squad").required(true).help("Squad name"))
                .arg(Arg::new("id").required(true).help("Job id, such as c1")),
        )
        .subcommand(build(specs::CRON_RUN))
        .subcommand(build(specs::CRON_TICK))
        .subcommand(build(specs::CRON_CLOCK))
}

fn status_document(status: ClockStatus) -> Value {
    match status {
        ClockStatus::Running(holder) => {
            json!({"state":"running","pane":holder.pane,"pid":holder.pid,"sinceMs":holder.since_ms,"expiresMs":holder.expires_ms})
        }
        ClockStatus::NoClock => json!({"state":"no clock"}),
        ClockStatus::Unknown => json!({"state":"unknown"}),
    }
}

fn manual_operation() -> Result<String, SquadError> {
    // A new explicit action gets one ID. Revalidation and recovery keep it unchanged.
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| SquadError::new("SQUAD_CRON_CLOCK_IO", error.to_string()))?;
    let nonce: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(cron::operation_id("tmt-squad-cron-manual", &nonce, 0))
}

/// Retains the operator and viewed revision; never silently sends a newer job.
pub fn send_now(
    core: &Core,
    config: &Config,
    key: &JobKey,
    actor: &CronActor,
    expected_revision: u64,
    operation_id: &str,
) -> Result<Value, SquadError> {
    let job = service::authorize_write(core, config, key, actor, expected_revision)?;
    let receipt = dispatch(core, &job, operation_id, Some(&actor.id))?;
    Ok(json!({"action":"send","job":job.document(),"dispatch":receipt,"warnings":[]}))
}

fn dispatch(
    core: &Core,
    job: &Job,
    operation_id: &str,
    identity: Option<&str>,
) -> Result<Value, SquadError> {
    let owner = job.owner_id.as_deref().ok_or_else(|| {
        failed(
            "SQUAD_CRON_NO_OWNER",
            "Reassign this job before sending it.",
        )
    })?;
    let input = json!({"operationId":operation_id,"recipientIds":[owner],"message":job.message,"kind":"request","room":{"kind":"direct","roomId":job.room_id}});
    let receipt = match core.api_write("dispatch.create", input, identity) {
        Ok(receipt) => receipt,
        Err(error)
            if matches!(
                error.code.as_str(),
                "SQUAD_CORE_UNAVAILABLE" | "STORAGE_UNAVAILABLE"
            ) =>
        {
            // Recover acceptance only. Missing evidence never causes another create;
            // neither this path nor a replay re-wakes input.
            core.api("dispatch.show", json!({"operationId":operation_id})).map_err(|_| {
                SquadError::new(&error.code, format!("{} Operation {operation_id}; recover acceptance with dispatch.show before retrying.", error.message))
            })?
        }
        Err(error) => {
            return Err(SquadError::new(
                &error.code,
                format!("{} Operation {operation_id}.", error.message),
            ));
        }
    };
    if receipt["operationId"] != operation_id
        || receipt["items"].as_array().is_none_or(|items| {
            items.len() != 1
                || items[0]["recipientId"] != owner
                || items[0]["acceptance"] != "queued"
                || items[0]["requestId"].as_str().is_none()
        })
    {
        return Err(SquadError::new(
            "SQUAD_CRON_DISPATCH_UNAVAILABLE",
            format!(
                "Operation {operation_id} was not accepted for the job owner; inspect its receipt before retrying."
            ),
        ));
    }
    Ok(receipt)
}

struct Scheduled<'a> {
    core: &'a Core,
    lease: &'a mut Lease,
    cancellation: &'a Cancellation,
    lost: bool,
}
impl Dispatch for Scheduled<'_> {
    fn send(&mut self, job: &Job, operation_id: &str) -> Result<(), cron::Error> {
        let sent = (|| {
            let admitted = service::admit_scheduled(self.core, &JobKey::of(job), job.revision)?;
            if !self.lease.renew(now())? {
                self.lost = true;
                return Err(failed(
                    "SQUAD_CRON_CLOCK_LOST",
                    "The clock lease expired or changed.",
                ));
            }
            dispatch(self.core, &admitted, operation_id, None).map(|_| ())
        })();
        sent.map_err(|error: SquadError| cron::Error {
            code: error.code,
            message: error.message,
        })
    }
    fn stopped(&self) -> bool {
        self.lost || self.cancellation.cancelled()
    }
}

fn acquire(clock: &Clock, time: i64) -> Result<Option<Lease>, SquadError> {
    Ok(clock.acquire(time, std::process::id(), std::env::var("TMUX_PANE").ok())?)
}
fn occupied() -> SquadError {
    failed(
        "SQUAD_CRON_CLOCK_RUNNING",
        "Another clock holds the lease; inspect tmt sq cron clock.",
    )
}

fn pass(
    core: &Core,
    config: &Config,
    root: &std::path::Path,
    lease: &mut Lease,
    tick: Option<&mut Tick>,
    cancellation: &Cancellation,
) -> Result<Value, SquadError> {
    let time = now();
    let mut warnings = service::drain_retired(core, config, time)?;
    let jobs = Store::new(root)?.read()?;
    let mut dispatcher = Scheduled {
        core,
        lease,
        cancellation,
        lost: false,
    };
    let report = match tick {
        Some(tick) => tick.running(time, jobs.jobs(), &mut dispatcher),
        None => Tick::standalone(time, jobs.jobs(), &mut dispatcher),
    };
    let complete = report.failures.is_empty() && !dispatcher.stopped();
    warnings.extend(report.failures.into_iter().map(SquadError::from));
    Ok(
        json!({"action":"tick","accepted":report.sent,"complete":complete,"warnings":warnings.iter().map(SquadError::to_json).collect::<Vec<_>>()}),
    )
}

pub fn run(core: &Core, config: &Config, parent: &ArgMatches) -> Result<Value, SquadError> {
    let (action, flags) = parent.subcommand().expect("cron action");
    if action == "send" {
        let actor = service::actor(
            core,
            config,
            flags.get_one::<String>("identity").map(String::as_str),
        )?;
        let squad = Squad::resolve(core, flags.get_one::<String>("squad").map(String::as_str))?;
        let key = JobKey {
            squad: squad.name,
            room_id: squad.room_id,
            id: flags.get_one::<String>("id").expect("job id").clone(),
        };
        let shown = service::show_job(core, config, &key, now())?;
        let mut sent = send_now(
            core,
            config,
            &key,
            &actor,
            shown.job.revision,
            &manual_operation()?,
        )?;
        sent["warnings"] = json!(
            shown
                .warnings
                .iter()
                .map(SquadError::to_json)
                .collect::<Vec<_>>()
        );
        return Ok(sent);
    }
    if action == "run" {
        return foreground(core, config);
    }
    let root = service::root(core)?;
    let clock = Clock::new(&root)?;
    if action == "clock" {
        let status = clock.status(now());
        return Ok(
            json!({"action":"clock","complete":status != ClockStatus::Unknown,"clock":status_document(status)}),
        );
    }
    let mut lease = acquire(&clock, now())?.ok_or_else(occupied)?;
    let result = pass(
        core,
        config,
        &root,
        &mut lease,
        None,
        &Cancellation::default(),
    );
    let released = lease.release();
    let value = result?;
    released?;
    Ok(value)
}

/// Independent of the refresh generation, selected tab, repaint and refresh-off.
pub struct ClockWorker {
    cancellation: Cancellation,
    stop: Option<Sender<()>>,
    finished: Receiver<()>,
    thread: Option<std::thread::JoinHandle<Result<Value, SquadError>>>,
}
impl ClockWorker {
    pub fn spawn(core: Core, config: Config, foreground: bool) -> Self {
        let cancellation = Cancellation::default();
        let reader = core.cancellable(cancellation.clone());
        let token = cancellation.clone();
        let (stop, waiting) = mpsc::channel();
        let (completed, finished) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let result = serve(&reader, &config, &waiting, &token, foreground);
            let _ = completed.send(());
            result
        });
        Self {
            cancellation,
            stop: Some(stop),
            finished,
            thread: Some(thread),
        }
    }
    pub fn stop(&mut self) -> Result<Value, SquadError> {
        self.cancellation.cancel();
        self.stop.take(); // Disconnect wakes an idle receiver immediately.
        self.thread.take().map_or_else(
            || Ok(json!({"action":"run"})),
            |thread| {
                thread
                    .join()
                    .map_err(|_| failed("SQUAD_CRON_CLOCK_FAILED", "The clock worker failed."))?
            },
        )
    }
}
impl Drop for ClockWorker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn serve(
    core: &Core,
    config: &Config,
    stop: &Receiver<()>,
    cancellation: &Cancellation,
    foreground: bool,
) -> Result<Value, SquadError> {
    let root = service::root(core)?;
    let clock = Clock::new(&root)?;
    let mut lease: Option<Lease> = None;
    let mut tick = Tick::new(now());
    let mut last = json!({"action":"tick","accepted":0,"warnings":[],"complete":true});
    let result = (|| {
        while !cancellation.cancelled() {
            let time = now();
            if let Some(owned) = lease.as_mut() {
                match owned.renew(time) {
                    Ok(true) => {}
                    Ok(false) => {
                        lease.take().expect("owned lease").release()?;
                    }
                    Err(error) if error.code == "SQUAD_CRON_CLOCK_BUSY" => {
                        match stop.recv_timeout(EVERY) {
                            Err(RecvTimeoutError::Timeout) => continue,
                            _ => break,
                        }
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            if lease.is_none() {
                lease = match acquire(&clock, time) {
                    Ok(lease) => lease,
                    Err(error) if !foreground && error.code == "SQUAD_CRON_CLOCK_BUSY" => None,
                    Err(error) => return Err(error),
                };
                if lease.is_none() && foreground {
                    return Err(occupied());
                }
                tick = Tick::new(time);
            }
            if let Some(owned) = lease.as_mut() {
                last = match pass(core, config, &root, owned, Some(&mut tick), cancellation) {
                    Ok(value) => value,
                    Err(error)
                        if matches!(
                            error.code.as_str(),
                            "SQUAD_CRON_STORE_BUSY"
                                | "SQUAD_CRON_CLOCK_BUSY"
                                | "SQUAD_CORE_UNAVAILABLE"
                        ) =>
                    {
                        json!({"action":"tick","accepted":0,"complete":false,"warnings":[error.to_json()]})
                    }
                    Err(error) => return Err(error),
                };
            }
            match stop.recv_timeout(EVERY) {
                Err(RecvTimeoutError::Timeout) => {}
                _ => break,
            }
        }
        last["action"] = json!("run");
        Ok(last)
    })();
    let released = lease.map_or(Ok(()), Lease::release);
    if cancellation.cancelled() {
        released?;
        return Ok(json!({"action":"run","warnings":[],"complete":true}));
    }
    let value = result?;
    released?;
    Ok(value)
}

fn foreground(core: &Core, config: &Config) -> Result<Value, SquadError> {
    let flag = Arc::new(AtomicUsize::new(0));
    let mut registrations = Vec::new();
    for signal in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ] {
        match signal_hook::flag::register_usize(signal, flag.clone(), signal as usize) {
            Ok(id) => registrations.push(id),
            Err(error) => {
                for id in registrations {
                    signal_hook::low_level::unregister(id);
                }
                return Err(SquadError::new("SQUAD_CRON_CLOCK_IO", error.to_string()));
            }
        }
    }
    let mut worker = ClockWorker::spawn(core.clone(), config.clone(), true);
    while flag.load(Ordering::Relaxed) == 0 {
        match worker.finished.recv_timeout(Duration::from_millis(200)) {
            Err(RecvTimeoutError::Timeout) => {}
            _ => break,
        }
    }
    let result = worker.stop();
    for id in registrations {
        signal_hook::low_level::unregister(id);
    }
    result
}

pub fn text(document: &Value, terminal: Terminal) -> String {
    let mut out = Vec::new();
    if document["action"] == "send" {
        let job = &document["job"];
        let _ = message::success(
            &mut out,
            terminal,
            &format!(
                "Accepted {} {}",
                job["squad"].as_str().unwrap_or_default(),
                job["id"].as_str().unwrap_or_default()
            ),
        );
    } else if document["action"] == "run" {
        let _ = message::success(&mut out, terminal, "Stopped clock");
    } else if document["action"] == "tick" {
        let _ = detail::write(
            &mut out,
            terminal,
            "cron tick",
            &[("accepted", document["accepted"].to_string())],
        );
    } else {
        let clock = &document["clock"];
        let mut fields = vec![("state", clock["state"].as_str().unwrap_or("unknown").into())];
        if clock["state"] == "running" {
            fields.extend([
                (
                    "pane",
                    clock["pane"].as_str().unwrap_or("(outside tmux)").into(),
                ),
                ("pid", clock["pid"].to_string()),
                (
                    "since",
                    clock["sinceMs"]
                        .as_i64()
                        .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
                        .map(|t| t.to_string())
                        .unwrap_or_default(),
                ),
            ]);
        } else if clock["state"] == "no clock" {
            fields.push(("start", "tmt sq cron run".into()));
        }
        let _ = detail::write(&mut out, terminal, "cron clock", &fields);
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests;
