//! Serialized invocation-owned lifecycle. One mutable owner coalesces all token
//! demands; a persisted generation comparison fences other invocations.
use super::{
    ATTEMPT_MS, AuthError, AuthErrorKind, AuthState, CREDENTIAL_LIFETIME_MS, Cancellation, Clock,
    Credential, CredentialStore, EXPIRY_NOTICE_MS, Http, Jitter, MAX_ATTEMPTS, Project,
    REFRESH_JITTER_MS, REFRESH_LEAD_MS, RESUME_MAX_MS, RESUME_MIN_MS, Request, SERIES_MS, Status,
    Token, rest,
};

pub struct Client<'a> {
    project: Project,
    store: &'a dyn CredentialStore,
    http: &'a dyn Http,
    clock: &'a dyn Clock,
    jitter: &'a dyn Jitter,
    cancel: &'a dyn Cancellation,
    cache: AuthCache,
    stopped: bool,
}
#[derive(Default)]
pub(super) struct AuthCache {
    credential: Option<Credential>,
    token: Option<Token>,
    needs_signin: bool,
    repaired: bool,
    retry_at_ms: u64,
    failed_series: u32,
}
impl AuthCache {
    pub(super) fn take_permission_notice(&mut self) -> Option<&'static str> {
        std::mem::take(&mut self.repaired).then_some(super::PERMISSION_NOTICE)
    }
}

impl<'a> Client<'a> {
    pub fn load(
        project: Project,
        store: &'a dyn CredentialStore,
        http: &'a dyn Http,
        clock: &'a dyn Clock,
        jitter: &'a dyn Jitter,
        cancel: &'a dyn Cancellation,
    ) -> Result<Self, AuthError> {
        let read = store.load()?;
        if read
            .credential
            .as_ref()
            .is_some_and(|c| c.project_id != project.project_id)
        {
            return Err(AuthError::new(AuthErrorKind::InvalidCredential));
        }
        let mut client = Self {
            project,
            store,
            http,
            clock,
            jitter,
            cancel,
            cache: AuthCache {
                credential: read.credential,
                repaired: read.permissions_repaired,
                ..AuthCache::default()
            },
            stopped: false,
        };
        // The bounded secret lifetime also applies to cold status/serve loads;
        // observing an ended sign-in must not leave its refresh token behind.
        client.expire()?;
        Ok(client)
    }
    pub(super) fn restore(&mut self, cache: AuthCache) {
        if cache.credential == self.cache.credential {
            self.cache = cache;
        }
    }
    pub(super) fn take_cache(&mut self) -> AuthCache {
        std::mem::take(&mut self.cache)
    }
    pub fn status(&self) -> Status {
        let now = self.clock.now().wall_ms;
        let state = match &self.cache.credential {
            Some(c) if c.revoking => AuthState::Revoking,
            Some(c) if now >= c.expires_at_ms => AuthState::ReSignInNeeded,
            Some(_) => AuthState::Active,
            None if self.cache.needs_signin => AuthState::ReSignInNeeded,
            None => AuthState::Unenrolled,
        };
        Status {
            state,
            uid: self.cache.credential.as_ref().map(|c| c.uid.clone()),
            expires_at_ms: self.cache.credential.as_ref().map(|c| c.expires_at_ms),
            expiry_notice: self.cache.credential.as_ref().is_some_and(|c| {
                !c.revoking && c.expires_at_ms.saturating_sub(now) <= EXPIRY_NOTICE_MS
            }),
            permissions_repaired: self.cache.repaired,
        }
    }
    pub fn stop(&mut self) {
        self.stopped = true;
        self.cache.token = None;
    }
    /// CLI composition consumes this once; repeated status reads never nag.
    pub fn take_permission_notice(&mut self) -> Option<&'static str> {
        self.cache.take_permission_notice()
    }
    fn fence(&self) -> Result<(), AuthError> {
        if self.stopped {
            Err(AuthError::new(AuthErrorKind::Stopped))
        } else if self.cancel.cancelled() {
            Err(AuthError::new(AuthErrorKind::Cancelled))
        } else {
            Ok(())
        }
    }
    fn check_generation(&mut self) -> Result<(), AuthError> {
        let read = match self.store.load() {
            Ok(read) => read,
            Err(error) => {
                self.cache.token = None;
                return Err(error);
            }
        };
        self.cache.repaired |= read.permissions_repaired;
        if read.credential != self.cache.credential {
            self.cache.token = None;
            self.cache.credential = read.credential;
            return Err(AuthError::state_changed());
        }
        Ok(())
    }
    fn expire(&mut self) -> Result<(), AuthError> {
        if self
            .cache
            .credential
            .as_ref()
            .is_some_and(|c| !c.revoking && self.clock.now().wall_ms >= c.expires_at_ms)
        {
            self.cache.token = None;
            self.cache.needs_signin = true;
            let old = self.cache.credential.as_ref().expect("captured credential");
            self.store.remove(old)?;
            self.cache.credential = None;
        }
        Ok(())
    }
    pub fn sign_in(&mut self) -> Result<(), AuthError> {
        self.fence()?;
        self.check_generation()?;
        self.expire()?;
        if self.cache.credential.as_ref().is_some_and(|c| c.revoking) {
            return Err(AuthError::new(AuthErrorKind::ReSignInNeeded));
        }
        if self.cache.credential.is_some() {
            return self.token().map(|_| ());
        }
        let started = self.clock.now();
        let expires = started
            .wall_ms
            .checked_add(CREDENTIAL_LIFETIME_MS)
            .ok_or_else(|| AuthError::new(AuthErrorKind::InputInvalid))?;
        let deadline = started.monotonic_ms.saturating_add(ATTEMPT_MS);
        // Signup is a single explicit attempt. Its unknown outcome creates no
        // retry task/marker; a later explicit sign-in may leave a harmless orphan.
        let response = self
            .http
            .send(&Request::signup(&self.project), deadline, self.cancel)
            .map_err(AuthError::uncertain)?;
        self.fence().map_err(AuthError::uncertain)?;
        if self.clock.now().monotonic_ms >= deadline {
            return Err(AuthError::unavailable().uncertain());
        }
        let issued = rest::signup_reply(&self.project, response).map_err(AuthError::uncertain)?;
        let credential = Credential {
            project_id: self.project.project_id.clone(),
            uid: issued.uid.clone(),
            refresh_token: issued.refresh.clone(),
            created_at_ms: started.wall_ms,
            expires_at_ms: expires,
            last_used_at_ms: started.wall_ms,
            revoking: false,
        };
        self.store
            .replace(None, &credential)
            .map_err(AuthError::uncertain)?;
        self.fence().map_err(AuthError::uncertain)?;
        self.cache.credential = Some(credential);
        self.cache.needs_signin = false;
        self.install(issued, started.monotonic_ms);
        Ok(())
    }
    pub fn token(&mut self) -> Result<&Token, AuthError> {
        self.fence()?;
        self.check_generation()?;
        self.expire()?;
        if self.cache.credential.as_ref().is_none_or(|c| c.revoking) {
            return Err(AuthError::new(AuthErrorKind::ReSignInNeeded));
        }
        if self
            .cache
            .token
            .as_ref()
            .is_none_or(|t| self.clock.now().monotonic_ms >= t.expires_monotonic_ms)
        {
            self.refresh_series()?;
        }
        self.fence()?;
        self.expire()?;
        self.cache
            .token
            .as_ref()
            .filter(|t| self.clock.now().monotonic_ms < t.expires_monotonic_ms)
            .ok_or_else(AuthError::unavailable)
    }
    pub fn refresh_if_due(&mut self) -> Result<bool, AuthError> {
        self.fence()?;
        self.check_generation()?;
        self.expire()?;
        if self.cache.credential.as_ref().is_none_or(|c| c.revoking) {
            return Ok(false);
        }
        if self
            .cache
            .token
            .as_ref()
            .is_some_and(|t| self.clock.now().monotonic_ms < t.refresh_monotonic_ms)
        {
            return Ok(false);
        }
        self.refresh_series()?;
        Ok(true)
    }
    pub fn next_refresh_monotonic_ms(&self) -> Option<u64> {
        if self.stopped || self.cache.credential.as_ref().is_none_or(|c| c.revoking) {
            return None;
        }
        Some(
            self.cache
                .token
                .as_ref()
                .map_or(self.cache.retry_at_ms, |t| {
                    t.refresh_monotonic_ms.max(self.cache.retry_at_ms)
                }),
        )
    }
    fn refresh_series(&mut self) -> Result<(), AuthError> {
        self.fence()?;
        let start = self.clock.now().monotonic_ms;
        if start < self.cache.retry_at_ms {
            return Err(AuthError::unavailable());
        }
        let old = self
            .cache
            .credential
            .clone()
            .ok_or_else(|| AuthError::new(AuthErrorKind::ReSignInNeeded))?;
        let series_end = start.saturating_add(SERIES_MS);
        let mut last = AuthError::unavailable();
        for attempt in 0..MAX_ATTEMPTS {
            self.fence()?;
            self.expire()?;
            if self.cache.credential.as_ref() != Some(&old) {
                return Err(AuthError::state_changed());
            }
            let now = self.clock.now();
            let local_remaining = old.expires_at_ms.saturating_sub(now.wall_ms);
            let deadline = now
                .monotonic_ms
                .saturating_add(ATTEMPT_MS.min(local_remaining))
                .min(series_end);
            if deadline <= now.monotonic_ms {
                break;
            }
            let response = self.http.send(
                &Request::refresh(&self.project, &old),
                deadline,
                self.cancel,
            );
            self.fence()?;
            self.expire()?;
            if self.cache.credential.as_ref() != Some(&old) {
                return Err(AuthError::new(AuthErrorKind::ReSignInNeeded));
            }
            let issued = if self.clock.now().monotonic_ms >= deadline {
                Err(AuthError::unavailable())
            } else {
                response.and_then(|r| rest::refresh_reply(&self.project, &old, r))
            };
            match issued {
                Ok(issued) => {
                    let mut next = old.clone();
                    next.refresh_token = issued.refresh.clone();
                    next.last_used_at_ms = self
                        .clock
                        .now()
                        .wall_ms
                        .max(old.last_used_at_ms)
                        .min(old.expires_at_ms);
                    self.store.replace(Some(&old), &next)?;
                    self.fence()?;
                    self.cache.credential = Some(next);
                    self.install(issued, now.monotonic_ms);
                    return Ok(());
                }
                Err(error)
                    if matches!(
                        error.kind,
                        AuthErrorKind::ReSignInNeeded | AuthErrorKind::UserNotFound
                    ) =>
                {
                    self.cache.token = None;
                    self.cache.needs_signin = true;
                    self.store.remove(&old)?;
                    self.cache.credential = None;
                    return Err(error);
                }
                Err(error) if error.kind != AuthErrorKind::Unavailable => return Err(error),
                Err(error) => last = error,
            }
            if attempt + 1 < MAX_ATTEMPTS {
                let ceiling = 1000_u64 << attempt;
                let wait = self.jitter.milliseconds(ceiling).min(ceiling);
                let deadline = self
                    .clock
                    .now()
                    .monotonic_ms
                    .saturating_add(wait)
                    .min(series_end);
                if !self.clock.wait_until(deadline, self.cancel) {
                    return Err(AuthError::new(AuthErrorKind::Cancelled));
                }
            }
        }
        let backoff = RESUME_MIN_MS
            .saturating_mul(1_u64 << self.cache.failed_series.min(5))
            .min(RESUME_MAX_MS);
        let jitter = self.jitter.milliseconds(backoff / 10).min(backoff / 10);
        self.cache.retry_at_ms = self
            .clock
            .now()
            .monotonic_ms
            .saturating_add(backoff.saturating_add(jitter).min(RESUME_MAX_MS));
        self.cache.failed_series = self.cache.failed_series.saturating_add(1);
        Err(last)
    }
    fn install(&mut self, issued: rest::Issued, request_started_ms: u64) {
        let expires = request_started_ms.saturating_add(issued.lifetime_ms);
        let jitter = self
            .jitter
            .milliseconds(REFRESH_JITTER_MS)
            .min(REFRESH_JITTER_MS);
        let refresh = expires
            .saturating_sub(REFRESH_LEAD_MS + jitter)
            .max(request_started_ms);
        self.cache.token = Some(Token {
            uid: issued.uid,
            value: issued.token,
            expires_monotonic_ms: expires,
            refresh_monotonic_ms: refresh,
        });
        self.cache.retry_at_ms = 0;
        self.cache.failed_series = 0;
    }
    pub fn revoke(&mut self) -> Result<(), AuthError> {
        self.fence()?;
        self.check_generation()?;
        let Some(old) = self.cache.credential.clone() else {
            return Ok(());
        };
        let mut revoking = old.clone();
        revoking.mark_revoking();
        if !old.revoking {
            self.store.replace(Some(&old), &revoking)?;
            self.cache.credential = Some(revoking.clone());
        }
        // Take the cached ID token out of normal access before any Auth effect.
        let cached = self
            .cache
            .token
            .take()
            .filter(|t| self.clock.now().monotonic_ms < t.expires_monotonic_ms);
        let started = self.clock.now().monotonic_ms;
        let token = match cached {
            Some(token) => token,
            None => {
                let deadline = started.saturating_add(ATTEMPT_MS);
                let response = self.http.send(
                    &Request::refresh(&self.project, &revoking),
                    deadline,
                    self.cancel,
                )?;
                self.fence()?;
                if self.clock.now().monotonic_ms >= deadline {
                    return Err(AuthError::unavailable());
                }
                let issued = match rest::refresh_reply(&self.project, &revoking, response) {
                    Ok(issued) => issued,
                    Err(error) if error.kind == AuthErrorKind::UserNotFound => {
                        self.store.remove(&revoking)?;
                        self.cache.credential = None;
                        self.cache.needs_signin = true;
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                };
                revoking.refresh_token = issued.refresh;
                self.store
                    .replace(self.cache.credential.as_ref(), &revoking)?;
                self.cache.credential = Some(revoking.clone());
                Token {
                    uid: issued.uid,
                    value: issued.token,
                    expires_monotonic_ms: started.saturating_add(issued.lifetime_ms),
                    refresh_monotonic_ms: started,
                }
            }
        };
        self.fence()?;
        if self.store.load()?.credential.as_ref() != Some(&revoking) {
            return Err(AuthError::state_changed());
        }
        let deadline = self.clock.now().monotonic_ms.saturating_add(ATTEMPT_MS);
        let response = self
            .http
            .send(
                &Request::delete(&self.project, &token),
                deadline,
                self.cancel,
            )
            .map_err(AuthError::uncertain)?;
        self.fence().map_err(AuthError::uncertain)?;
        if self.clock.now().monotonic_ms >= deadline {
            return Err(AuthError::unavailable().uncertain());
        }
        rest::delete_reply(response).map_err(AuthError::uncertain)?;
        self.store.remove(&revoking)?;
        self.cache.credential = None;
        self.cache.needs_signin = true;
        Ok(())
    }
}
impl Drop for Client<'_> {
    fn drop(&mut self) {
        self.stop();
    }
}
