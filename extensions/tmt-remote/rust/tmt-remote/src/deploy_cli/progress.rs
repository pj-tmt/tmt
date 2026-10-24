//! One cosmetic live line. The runner names work; only this CLI owner measures
//! time, rewrites stderr, and joins its heartbeat before the command returns.
use crate::deploy_run::{DeployPlan, DeployProgress, HostingStep, StepKind};
use std::{
    io::{self, Write},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tmt_cli_style::{Interaction, Mode, grid};

const TICK: Duration = Duration::from_secs(1);
const ELAPSED_AFTER: Duration = Duration::from_secs(5);

pub fn enabled(interaction: Interaction, agent: bool, ci: bool) -> bool {
    !agent
        && !ci
        && interaction.view() == Mode::Interactive
        && interaction.prompt() == Mode::Interactive
}

fn phrase(plan: &DeployPlan, event: DeployProgress) -> String {
    let DeployProgress::Step(index) = event else {
        return "Waiting for indexes to build…".into();
    };
    let Some(step) = plan.steps().get(index) else {
        return String::new();
    };
    let uploads = |step: &&crate::deploy_run::DeployStep| {
        matches!(step.kind, StepKind::Hosting(HostingStep::Upload { .. }))
    };
    step_phrase(
        &step.kind,
        plan.steps()[..=index].iter().filter(uploads).count(),
        plan.steps().iter().filter(uploads).count(),
    )
}

fn step_phrase(kind: &StepKind, ordinal: usize, total: usize) -> String {
    match kind {
        StepKind::Database => "Setting up the database…",
        StepKind::SignIn(_) => "Setting up sign-in…",
        StepKind::Index { .. } | StepKind::CompositeIndex(_) => "Setting up indexes…",
        StepKind::Hosting(kind) => match kind {
            HostingStep::WebAppCreate | HostingStep::SiteCreate | HostingStep::SiteConfigure => {
                "Setting up the website…"
            }
            HostingStep::VersionCreate | HostingStep::Populate => "Preparing website files…",
            HostingStep::Upload { .. } => {
                return format!("Uploading website files {ordinal}/{total}…");
            }
            HostingStep::Finalize | HostingStep::Release => "Publishing the website…",
        },
        StepKind::OwnerAccess => "Register this machine as owner",
        StepKind::Rules => "Setting up security rules…",
        StepKind::Verify => "Checking the finished setup…",
    }
    .into()
}

fn line(phrase: &str, elapsed: Duration) -> String {
    if elapsed < ELAPSED_AFTER {
        return phrase.into();
    }
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{phrase} {seconds}s")
    } else {
        format!("{phrase} {}m {:02}s", seconds / 60, seconds % 60)
    }
}

/// All text is fixed CLI copy. Leave the last column unused so even a full line
/// cannot wrap. Width and Unicode fitting come from the shared CLI style owner.
fn rewrite(output: &mut impl Write, columns: Option<u16>, text: Option<&str>) -> io::Result<()> {
    let Some(columns) = columns.filter(|columns| *columns > 1) else {
        return Ok(());
    };
    let fitted = grid::fit(
        text.unwrap_or(""),
        usize::from(columns - 1),
        grid::Align::Left,
        grid::Truncate::End,
    );
    // Carriage return plus a padded cell replaces/clears the line without ANSI.
    write!(output, "\r{fitted}\r")?;
    output.flush()
}

type Display = Box<dyn FnMut(Option<&str>) -> io::Result<()> + Send>;
struct State {
    active: Option<(String, Instant)>,
    stopped: bool,
    display: Display,
}
impl State {
    fn clear(&mut self) {
        if self.active.take().is_some() {
            let _ = (self.display)(None);
        }
    }
}
struct Shared {
    cancel: Arc<AtomicBool>,
    state: Mutex<State>,
    wake: Condvar,
}

/// A cloned notification endpoint never owns the heartbeat's lifetime.
#[derive(Clone, Default)]
pub struct Reporter(Option<Arc<Shared>>);
impl Reporter {
    pub fn notify(&self, plan: &DeployPlan, event: DeployProgress) {
        let Some(shared) = &self.0 else {
            return;
        };
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if shared.cancel.load(Ordering::Acquire) {
            state.stopped = true;
            state.clear();
            shared.wake.notify_one();
        }
        if state.stopped {
            return;
        }
        let text = phrase(plan, event);
        state.active = Some((text.clone(), Instant::now()));
        if (state.display)(Some(&text)).is_err() {
            state.stopped = true;
            state.clear();
        }
        shared.wake.notify_one();
    }
}

/// Dropping the invocation guard wakes and joins the worker, then erases its
/// line. Output failures remain cosmetic and never change a deploy outcome.
pub struct Progress {
    reporter: Reporter,
    worker: Option<JoinHandle<()>>,
}
impl Progress {
    pub fn stderr(enabled: bool, cancel: Arc<AtomicBool>) -> Self {
        // Unknown width cannot promise a single physical line; print no extra.
        let mut columns = tmt_cli_style::stream::stdout(false).terminal().width;
        Self::with_display(
            enabled && columns.is_some(),
            Box::new(move |text| {
                columns = tmt_cli_style::stream::stdout(false)
                    .terminal()
                    .width
                    .or(columns);
                rewrite(&mut tmt_cli_style::stream::stderr(), columns, text)
            }),
            TICK,
            cancel,
        )
    }
    fn with_display(
        enabled: bool,
        display: Display,
        tick: Duration,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        if !enabled {
            return Self {
                reporter: Reporter::default(),
                worker: None,
            };
        }
        let shared = Arc::new(Shared {
            cancel,
            state: Mutex::new(State {
                active: None,
                stopped: false,
                display,
            }),
            wake: Condvar::new(),
        });
        let worker_state = shared.clone();
        let worker = thread::Builder::new()
            .name("deploy-progress".into())
            .spawn(move || {
                let mut state = worker_state
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                while !state.stopped && !worker_state.cancel.load(Ordering::Acquire) {
                    let (next, _) = worker_state
                        .wake
                        .wait_timeout(state, tick)
                        .unwrap_or_else(|error| error.into_inner());
                    state = next;
                    if state.stopped || worker_state.cancel.load(Ordering::Acquire) {
                        break;
                    }
                    if let Some((phrase, start)) = &state.active {
                        let elapsed = start.elapsed();
                        if elapsed >= ELAPSED_AFTER {
                            let text = line(phrase, elapsed);
                            if (state.display)(Some(&text)).is_err() {
                                state.stopped = true;
                            }
                        }
                    }
                }
                state.stopped = true;
                state.clear();
            });
        match worker {
            Ok(worker) => Self {
                reporter: Reporter(Some(shared)),
                worker: Some(worker),
            },
            // Failure to start a cosmetic worker must not block a deploy.
            Err(_) => Self {
                reporter: Reporter::default(),
                worker: None,
            },
        }
    }
    pub fn reporter(&self) -> Reporter {
        self.reporter.clone()
    }
}
impl Drop for Progress {
    fn drop(&mut self) {
        if let Some(shared) = &self.reporter.0 {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.stopped = true;
            shared.wake.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(shared) = &self.reporter.0 {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    const PERSON: Interaction = Interaction {
        json: false,
        stdin: true,
        stdout: true,
        stderr: true,
        dumb: false,
    };

    #[test]
    fn only_human_rewrite_capable_streams_enable_progress() {
        assert!(enabled(PERSON, false, false));
        for interaction in [
            Interaction {
                json: true,
                ..PERSON
            },
            Interaction {
                stdin: false,
                ..PERSON
            },
            Interaction {
                stdout: false,
                ..PERSON
            },
            Interaction {
                stderr: false,
                ..PERSON
            },
            Interaction {
                dumb: true,
                ..PERSON
            },
        ] {
            assert!(!enabled(interaction, false, false));
        }
        assert!(!enabled(PERSON, true, false));
        assert!(!enabled(PERSON, false, true));
    }

    #[test]
    fn all_step_phrases_are_fixed_and_uploads_only_expose_the_ordinal() {
        use crate::{deploy_plan::CompositeIndexPlan, deploy_run::SignInProvider};
        for (kind, expected) in [
            (StepKind::Database, "Setting up the database…"),
            (
                StepKind::SignIn(SignInProvider::Anonymous),
                "Setting up sign-in…",
            ),
            (
                StepKind::Index {
                    path: "private-path".into(),
                    field: "private-field".into(),
                    direction: "ASCENDING",
                },
                "Setting up indexes…",
            ),
            (
                StepKind::CompositeIndex(CompositeIndexPlan {
                    collection_group: "private-group".into(),
                    query_scope: "COLLECTION",
                    fields: vec![],
                }),
                "Setting up indexes…",
            ),
            (
                StepKind::Hosting(HostingStep::WebAppCreate),
                "Setting up the website…",
            ),
            (
                StepKind::Hosting(HostingStep::SiteCreate),
                "Setting up the website…",
            ),
            (
                StepKind::Hosting(HostingStep::SiteConfigure),
                "Setting up the website…",
            ),
            (
                StepKind::Hosting(HostingStep::VersionCreate),
                "Preparing website files…",
            ),
            (
                StepKind::Hosting(HostingStep::Populate),
                "Preparing website files…",
            ),
            (
                StepKind::Hosting(HostingStep::Upload {
                    path: "private-path".into(),
                    hash: "private-hash".into(),
                }),
                "Uploading website files 7/15…",
            ),
            (
                StepKind::Hosting(HostingStep::Finalize),
                "Publishing the website…",
            ),
            (
                StepKind::Hosting(HostingStep::Release),
                "Publishing the website…",
            ),
            (StepKind::OwnerAccess, "Register this machine as owner"),
            (StepKind::Rules, "Setting up security rules…"),
            (StepKind::Verify, "Checking the finished setup…"),
        ] {
            assert_eq!(step_phrase(&kind, 7, 15), expected);
        }
    }

    #[test]
    fn elapsed_copy_starts_at_five_seconds_and_uses_compact_units() {
        for (seconds, expected) in [
            (0, "Checking the finished setup…"),
            (4, "Checking the finished setup…"),
            (5, "Checking the finished setup… 5s"),
            (40, "Checking the finished setup… 40s"),
            (60, "Checking the finished setup… 1m 00s"),
            (65, "Checking the finished setup… 1m 05s"),
        ] {
            assert_eq!(
                line("Checking the finished setup…", Duration::from_secs(seconds)),
                expected
            );
        }
        assert_eq!(TICK, Duration::from_secs(1));
    }

    #[test]
    fn rewrite_keeps_one_physical_line_and_unknown_width_is_silent() {
        let mut bytes = Vec::new();
        rewrite(&mut bytes, None, Some("Setting up indexes…")).unwrap();
        rewrite(&mut bytes, Some(1), Some("Setting up indexes…")).unwrap();
        assert!(bytes.is_empty());
        rewrite(&mut bytes, Some(12), Some("Setting up indexes…")).unwrap();
        rewrite(&mut bytes, Some(12), Some("Done")).unwrap();
        rewrite(&mut bytes, Some(12), None).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "\rSetting up…\r\rDone       \r\r           \r"
        );
    }

    fn captured(enabled: bool) -> (Progress, Arc<Mutex<Vec<String>>>) {
        let output = Arc::new(Mutex::new(Vec::new()));
        let copy = output.clone();
        let progress = Progress::with_display(
            enabled,
            Box::new(move |line| {
                copy.lock().unwrap().push(line.unwrap_or("<clear>").into());
                Ok(())
            }),
            TICK,
            Arc::new(AtomicBool::new(false)),
        );
        (progress, output)
    }

    #[test]
    fn disabled_and_never_started_invocations_print_nothing() {
        for enabled in [false, true] {
            let (progress, output) = captured(enabled);
            drop(progress);
            assert!(output.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn guard_clears_and_joins_before_final_output_error_or_interrupt() {
        for result in [Ok(()), Err("error"), Err("interrupt")] {
            let (progress, output) = captured(true);
            let reporter = progress.reporter();
            let shared = reporter.0.as_ref().unwrap();
            {
                let mut state = shared.state.lock().unwrap();
                state.active = Some(("Checking the finished setup…".into(), Instant::now()));
                (state.display)(Some("Checking the finished setup…")).unwrap();
            }
            drop(progress);
            assert!(shared.state.lock().unwrap().stopped);
            let mut output = output.lock().unwrap();
            output.push(format!("final {result:?}"));
            assert_eq!(&output[..2], ["Checking the finished setup…", "<clear>"]);
            assert_eq!(output.len(), 3, "no detached worker or late tick");
        }
    }

    #[test]
    fn heartbeat_uses_active_elapsed_time_and_drop_wakes_a_long_wait() {
        let (sent, received) = mpsc::channel();
        let progress = Progress::with_display(
            true,
            Box::new(move |line| {
                sent.send(line.map(str::to_owned)).unwrap();
                Ok(())
            }),
            Duration::from_millis(10),
            Arc::new(AtomicBool::new(false)),
        );
        let reporter = progress.reporter();
        let shared = reporter.0.as_ref().unwrap();
        shared.state.lock().unwrap().active = Some((
            "Checking the finished setup…".into(),
            Instant::now() - Duration::from_secs(65),
        ));
        shared.wake.notify_one();
        assert_eq!(
            received
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .as_deref(),
            Some("Checking the finished setup… 1m 05s")
        );
        drop(progress);
        assert!(shared.state.lock().unwrap().stopped);
        let remaining = received.try_iter().collect::<Vec<_>>();
        assert_eq!(remaining.last(), Some(&None), "last action clears");
        // The endpoint itself must not keep a worker alive.
        drop(reporter);
    }
    #[test]
    fn cancellation_clears_a_blocked_step_without_waiting_for_the_provider() {
        let (sent, received) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Progress::with_display(
            true,
            Box::new(move |line| {
                sent.send(line.map(str::to_owned)).unwrap();
                Ok(())
            }),
            Duration::from_millis(10),
            cancel.clone(),
        );
        let reporter = progress.reporter();
        let shared = reporter.0.as_ref().unwrap();
        shared.state.lock().unwrap().active =
            Some(("Checking the finished setup…".into(), Instant::now()));
        cancel.store(true, Ordering::Release);
        shared.wake.notify_one();
        assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), None);
        assert!(shared.state.lock().unwrap().stopped);
        drop(progress);
        assert!(
            received.try_iter().next().is_none(),
            "no second clear or late tick"
        );
    }

    #[test]
    fn display_failure_stops_cosmetic_updates_and_clears_the_line() {
        let (sent, received) = mpsc::channel();
        let progress = Progress::with_display(
            true,
            Box::new(move |line| {
                sent.send(line.map(str::to_owned)).unwrap();
                if line.is_some() {
                    Err(io::ErrorKind::BrokenPipe.into())
                } else {
                    Ok(())
                }
            }),
            Duration::from_millis(10),
            Arc::new(AtomicBool::new(false)),
        );
        let reporter = progress.reporter();
        let shared = reporter.0.as_ref().unwrap();
        shared.state.lock().unwrap().active = Some((
            "Checking the finished setup…".into(),
            Instant::now() - Duration::from_secs(5),
        ));
        shared.wake.notify_one();
        assert!(
            received
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .is_some()
        );
        assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), None);
        assert!(shared.state.lock().unwrap().stopped);
        drop(progress);
        assert!(received.try_iter().next().is_none());
    }
}
