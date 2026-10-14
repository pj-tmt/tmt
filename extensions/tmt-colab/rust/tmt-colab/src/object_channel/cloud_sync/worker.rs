//! The generation-owned loop: activate, bind, publish the active page. Nothing here retries
//! without a bound, and nothing here is progress: only a verified durable admission or a
//! confirmed publication moves a position.
use super::{
    ActivationFailure, Bound, Channels, PageRef, Parts,
    activation::{Next, Policy},
    publisher::{Fault, Local, Publisher},
};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};
use tmt_extension_objects::{Uuid4, sync};

/// How long activation, the arrival of its channel and the bind may take together.
const ESTABLISH_BOUND: Duration = Duration::from_secs(20);
/// How often an idle worker looks again; every real reason to act wakes it sooner.
const TICK: Duration = if cfg!(test) {
    Duration::from_millis(50)
} else {
    Duration::from_secs(1)
};
/// The soonest a lost session is established again: a channel that keeps ending must not turn
/// into an activation loop.
const REESTABLISH: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(30)
};
const POLL: Duration = Duration::from_millis(25);

struct Session {
    generation: Uuid4,
    publisher: Publisher,
    local: Box<dyn Local + Send>,
    page: Option<PageRef>,
    /// Pages whose publication stopped, remembered for this session so a touch neither retries
    /// them nor shows their line again. Another epoch of the page is another entry.
    stuck: HashSet<PageRef>,
    /// When the next publication pass is due; none while the page is current or stopped.
    pass_at: Option<Instant>,
    backoff: Backoff,
    established: Instant,
}
/// The wait before an uncertain pass runs again: it doubles up to a cap and starts over after
/// a pass that finished.
struct Backoff(Duration);
impl Backoff {
    const START: Duration = if cfg!(test) {
        Duration::from_millis(50)
    } else {
        Duration::from_secs(5)
    };
    const CAP: Duration = if cfg!(test) {
        Duration::from_millis(200)
    } else {
        Duration::from_secs(300)
    };
    fn new() -> Self {
        Self(Self::START)
    }
    fn next(&mut self) -> Duration {
        let wait = self.0;
        self.0 = (self.0 * 2).min(Self::CAP);
        wait
    }
    fn reset(&mut self) {
        self.0 = Self::START;
    }
}
impl Session {
    fn alive(&self, channels: &dyn Channels) -> bool {
        channels
            .client()
            .is_some_and(|client| client.generation() == self.generation)
    }
    /// The most recently active page is the one published; another page is published when it
    /// is touched. Every touch asks for a pass.
    fn touched(&mut self, page: &PageRef) {
        self.page = Some(page.clone());
        self.pass_at = Some(Instant::now());
    }
    /// One bounded publication pass for the active page, and when the next one is due.
    fn pass(&mut self, report: &dyn Fn(&str)) {
        let Some(page) = self.page.clone().filter(|page| !self.stuck.contains(page)) else {
            self.pass_at = None;
            return;
        };
        self.pass_at = match self.publisher.pass(self.local.as_mut(), &page) {
            Ok(true) => Some(Instant::now()),
            Ok(false) => {
                self.backoff.reset();
                None
            }
            Err(Fault::Retry) => Some(Instant::now() + self.backoff.next()),
            Err(Fault::Budget(wait)) => Some(Instant::now() + Duration::from_millis(wait)),
            Err(Fault::Moved) => None,
            Err(Fault::Stuck(line)) => {
                self.stuck.insert(page);
                report(line);
                None
            }
        };
    }
}

/// One bounded attempt: Remote activates, its channel arrives, the bind succeeds and agrees
/// with every frozen input. A refusal Remote wrote is returned as written.
fn establish(parts: &Parts) -> Result<Session, ActivationFailure> {
    let deadline = Instant::now() + ESTABLISH_BOUND;
    let activation = (parts.activate)(deadline)?;
    let client = loop {
        if let Some(client) = parts.channels.client()
            && client.generation() == activation.generation
        {
            break client;
        }
        if Instant::now() >= deadline {
            return Err(ActivationFailure::Local);
        }
        std::thread::sleep(POLL);
    };
    if !client.supports_sync().unwrap_or(false) {
        return Err(ActivationFailure::Local);
    }
    let (_, outcome) = client
        .sealed_request(
            sync::Call::Bind {
                project: activation.project.clone(),
                extension: "colab".into(),
                declaration_digest: parts.declaration_digest.clone(),
                space: parts.space.clone(),
            },
            deadline,
        )
        .map_err(|_| ActivationFailure::Local)?;
    let sync::Outcome::Binding { context } = outcome else {
        return Err(ActivationFailure::Local);
    };
    if context.project != activation.project
        || context.extension != "colab"
        || context.declaration_digest != parts.declaration_digest
        || context.space != parts.space
        || context.uid.is_empty()
        || Uuid4::parse(&context.id).is_err()
    {
        client.end_generation();
        return Err(ActivationFailure::Local);
    }
    let local = (parts.local)(Bound {
        project: activation.project,
        declaration_digest: parts.declaration_digest.clone(),
        uid: context.uid,
    })
    .ok_or(ActivationFailure::Local)?;
    Ok(Session {
        publisher: Publisher::new(client.clone(), context.id),
        local,
        generation: client.generation(),
        page: None,
        stuck: HashSet::new(),
        pass_at: None,
        backoff: Backoff::new(),
        established: Instant::now(),
    })
}

pub(super) fn run(parts: Parts) {
    let Parts {
        channels, activity, ..
    } = &parts;
    let mut policy = Policy::default();
    policy.trigger();
    let mut session: Option<Session> = None;
    let mut retry_at = Some(Instant::now());
    let mut seen_channel = channels.started();
    let mut seen_version = 0;
    let mut wake = activity.wait(0, Duration::ZERO);
    loop {
        if wake.stopped {
            break;
        }
        let version = wake.version;
        let now = Instant::now();
        // A session whose channel ended is gone; its replacement waits out REESTABLISH.
        if let Some(lost) = session.take_if(|s| !s.alive(channels.as_ref())) {
            policy.trigger();
            retry_at = Some(lost.established + REESTABLISH).map(|at| at.max(now));
        }
        // A channel this worker did not activate is a new trigger: Remote may have restarted.
        let started = channels.started();
        if started != seen_channel {
            seen_channel = started;
            if session.is_none() {
                policy.trigger();
                retry_at = Some(now);
            }
        }
        if session.is_none() && !policy.is_sticky() && retry_at.is_some_and(|at| now >= at) {
            retry_at = None;
            match establish(&parts) {
                Ok(established) => {
                    seen_channel = channels.started();
                    session = Some(established);
                }
                Err(failure) => {
                    if let Next::Retry(wait) = policy.failed(&failure) {
                        retry_at = Some(Instant::now() + wait);
                    }
                }
            }
        }
        if let Some(held) = session.as_mut() {
            if version != seen_version
                && let Some(page) = wake.page.as_ref().filter(|page| page.space == parts.space)
            {
                held.touched(page);
            }
            if held.pass_at.is_some_and(|at| Instant::now() >= at) {
                held.pass(parts.report.as_ref());
            }
        }
        seen_version = version;
        let due = [retry_at, session.as_ref().and_then(|held| held.pass_at)]
            .into_iter()
            .flatten()
            .min();
        let timeout = due.map_or(TICK, |at| at.saturating_duration_since(Instant::now()));
        wake = activity.wait(version, timeout.min(TICK));
    }
    // Dropping the session is the whole teardown: it holds no interest in this slice.
    drop(session);
}
