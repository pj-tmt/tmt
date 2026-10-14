//! The generation-owned loop: activate, bind, follow the active page. Nothing here retries
//! without a bound, and nothing here is progress: only a verified durable admission or a
//! confirmed publication moves a position.
use super::{
    ActivationFailure, Channels, PageInterests, PageRef, Parts, REQUEST_BOUND,
    activation::{Next, Policy},
};
use crate::SealedClient;
use std::time::{Duration, Instant};
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
    client: SealedClient,
    generation: Uuid4,
    interests: PageInterests,
    page: Option<PageRef>,
    established: Instant,
}
impl Session {
    fn alive(&self, channels: &dyn Channels) -> bool {
        channels
            .client()
            .is_some_and(|client| client.generation() == self.generation)
    }
    /// Hold exactly the interests of the most recently active page. A page that is not held
    /// catches up on demand; it is never reported as current.
    fn follow(&mut self, page: &PageRef) -> Result<(), crate::ChannelError> {
        if self.page.as_ref() == Some(page) {
            return Ok(());
        }
        self.page = None;
        self.interests.select(
            &page.space,
            &page.page,
            page.epoch,
            Instant::now() + REQUEST_BOUND,
        )?;
        self.page = Some(page.clone());
        Ok(())
    }
    fn close(&mut self) {
        let _ = self.interests.cancel(Instant::now() + REQUEST_BOUND);
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
    Ok(Session {
        interests: PageInterests::new(client.clone(), context.id),
        generation: client.generation(),
        client,
        page: None,
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
                    if let Some(line) = policy.warning(&failure) {
                        (parts.report)(&line);
                    }
                    if let Next::Retry(wait) = policy.failed(&failure) {
                        retry_at = Some(Instant::now() + wait);
                    }
                }
            }
        }
        if let (Some(held), Some(page)) = (session.as_mut(), wake.page.as_ref())
            && page.space == parts.space
            && held.follow(page).is_err()
        {
            // The group could not be installed or cancelled: the generation is invalid.
            held.client.end_generation();
        }
        let timeout = retry_at.map_or(TICK, |at| at.saturating_duration_since(Instant::now()));
        wake = activity.wait(version, timeout.min(TICK));
    }
    if let Some(mut held) = session {
        held.close();
    }
}
