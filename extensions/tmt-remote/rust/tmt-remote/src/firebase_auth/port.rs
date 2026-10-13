//! Serving-owner adapter for the frozen AuthPort. The mutex coalesces refresh
//! demand; only this owner's cache is held during HTTP, never a credential lock.
use super::{
    AuthError, AuthErrorKind, AuthState, Cancellation, Client, Clock, CredentialStore, Http,
    Jitter, Project, Request, Response, Time, lifecycle::AuthCache,
};
use crate::{
    objects::IoBudget,
    sealed_sync::{AuthFailure, AuthIdentity, AuthPort, AuthToken},
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, TryLockError,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

pub struct AuthOwner {
    identity: AuthIdentity,
    project: Project,
    store: Arc<dyn CredentialStore + Send + Sync>,
    http: Arc<dyn Http + Send + Sync>,
    clock: Arc<dyn Clock + Send + Sync>,
    jitter: Arc<dyn Jitter + Send + Sync>,
    cache: Mutex<Option<AuthCache>>,
    stopped: AtomicBool,
}
impl AuthOwner {
    /// Load an existing sign-in only. This adapter has no signup or revoke path.
    pub fn load(
        project: Project,
        store: Arc<dyn CredentialStore + Send + Sync>,
        http: Arc<dyn Http + Send + Sync>,
        clock: Arc<dyn Clock + Send + Sync>,
        jitter: Arc<dyn Jitter + Send + Sync>,
    ) -> Result<Self, AuthError> {
        let cancel = AtomicBool::new(false);
        let mut client = Client::load(
            project.clone(),
            store.as_ref(),
            http.as_ref(),
            clock.as_ref(),
            jitter.as_ref(),
            &cancel,
        )?;
        let status = client.status();
        if status.state != AuthState::Active {
            return Err(AuthError::new(AuthErrorKind::ReSignInNeeded));
        }
        let identity = AuthIdentity {
            project: project.project_id.clone(),
            uid: status
                .uid
                .ok_or_else(|| AuthError::new(AuthErrorKind::ReSignInNeeded))?,
        };
        let cache = client.take_cache();
        drop(client);
        Ok(Self {
            identity,
            project,
            store,
            http,
            clock,
            jitter,
            cache: Mutex::new(Some(cache)),
            stopped: AtomicBool::new(false),
        })
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Ok(mut cache) = self.cache.try_lock() {
            *cache = None;
        }
    }
    /// Startup composition consumes the credential-only mode repair notice once.
    pub fn take_permission_notice(&self) -> Option<&'static str> {
        self.cache
            .try_lock()
            .ok()?
            .as_mut()?
            .take_permission_notice()
    }
}
struct Call<'a> {
    io: &'a IoBudget<'a>,
    stopped: &'a AtomicBool,
    source_clock: &'a dyn Clock,
    deadline: u64,
}
impl Cancellation for Call<'_> {
    fn cancelled(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
            || self.io.check().is_err()
            || self.source_clock.now().monotonic_ms >= self.deadline
    }
}
struct BoundClock<'a>(&'a Call<'a>);
impl Clock for BoundClock<'_> {
    fn now(&self) -> Time {
        self.0.source_clock.now()
    }
    fn wait_until(&self, deadline: u64, cancel: &dyn Cancellation) -> bool {
        if self.0.cancelled() {
            return false;
        }
        self.0
            .source_clock
            .wait_until(deadline.min(self.0.deadline), cancel)
            && !self.0.cancelled()
    }
}
struct BoundHttp<'a> {
    source: &'a dyn Http,
    call: &'a Call<'a>,
}
impl Http for BoundHttp<'_> {
    fn send(
        &self,
        request: &Request,
        deadline: u64,
        cancel: &dyn Cancellation,
    ) -> Result<Response, AuthError> {
        if self.call.cancelled() {
            return Err(AuthError::new(AuthErrorKind::Cancelled));
        }
        self.source
            .send(request, deadline.min(self.call.deadline), cancel)
    }
}
fn failure(error: AuthError) -> AuthFailure {
    use crate::sealed_sync::AuthState as State;
    AuthFailure {
        state: match error.kind {
            AuthErrorKind::ReSignInNeeded
            | AuthErrorKind::UserNotFound
            | AuthErrorKind::StateChanged => State::SignInNeeded,
            AuthErrorKind::Cancelled | AuthErrorKind::Stopped => State::Cancelled,
            _ => State::Unavailable,
        },
    }
}
impl AuthPort for AuthOwner {
    fn identity(&self) -> AuthIdentity {
        self.identity.clone()
    }
    fn token<'a>(
        &'a self,
        io: &'a IoBudget<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AuthToken, AuthFailure>> + Send + 'a>> {
        Box::pin(async move {
            let now = self.clock.now();
            let remaining = u64::try_from(
                io.deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis(),
            )
            .unwrap_or(u64::MAX);
            let call = Call {
                io,
                stopped: &self.stopped,
                source_clock: self.clock.as_ref(),
                deadline: now.monotonic_ms.saturating_add(remaining),
            };
            let bounded_clock = BoundClock(&call);
            let bounded_http = BoundHttp {
                source: self.http.as_ref(),
                call: &call,
            };
            let mut cache = loop {
                if call.cancelled() {
                    return Err(failure(AuthError::new(AuthErrorKind::Cancelled)));
                }
                match self.cache.try_lock() {
                    Ok(guard) => break guard,
                    Err(TryLockError::Poisoned(_)) => {
                        return Err(failure(AuthError::unavailable()));
                    }
                    Err(TryLockError::WouldBlock) => {
                        let wake = self.clock.now().monotonic_ms.saturating_add(10);
                        if !bounded_clock.wait_until(wake, &call) {
                            return Err(failure(AuthError::new(AuthErrorKind::Cancelled)));
                        }
                    }
                }
            };
            let mut client = Client::load(
                self.project.clone(),
                self.store.as_ref(),
                &bounded_http,
                &bounded_clock,
                self.jitter.as_ref(),
                &call,
            )
            .map_err(failure)?;
            if let Some(previous) = cache.take() {
                client.restore(previous);
            }
            let result = (|| {
                if client.status().uid.as_deref() != Some(&self.identity.uid) {
                    return Err(AuthError::new(AuthErrorKind::ReSignInNeeded));
                }
                match client.refresh_if_due() {
                    Ok(_) => {}
                    // A transient proactive refresh failure may still use the
                    // same unexpired token. token() performs the final fence.
                    Err(error) if error.kind == AuthErrorKind::Unavailable => {}
                    Err(error) => return Err(error),
                }
                let token = client.token()?;
                if token.uid != self.identity.uid {
                    return Err(AuthError::new(AuthErrorKind::ReSignInNeeded));
                }
                let now = self.clock.now();
                Ok(AuthToken {
                    identity: self.identity.clone(),
                    token: token.value.clone(),
                    expires_at_ms: now.wall_ms.saturating_add(
                        token.expires_monotonic_ms.saturating_sub(now.monotonic_ms),
                    ),
                })
            })();
            if call.cancelled() {
                client.stop();
            }
            *cache = if self.stopped.load(Ordering::SeqCst) {
                None
            } else {
                Some(client.take_cache())
            };
            if call.cancelled() {
                return Err(failure(AuthError::new(AuthErrorKind::Cancelled)));
            }
            result.map_err(failure)
        })
    }
}
