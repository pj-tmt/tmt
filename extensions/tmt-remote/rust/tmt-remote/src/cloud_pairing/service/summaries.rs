//! The serving worker samples public Ops only for eligible native read peers.
//! Network work keeps the existing provider, project guard and immutable-ID owner.
use super::*;
use crate::squads::{Record, SummaryReader, publisher::Prepared};

/// Verified feature evidence is a cache, never authority. Failed lookups back
/// off exponentially; unsupported peers are not sent periodic metadata.
pub(super) struct SummarySupport {
    revision: u64,
    key: [u8; 32],
    checked: Instant,
    failures: u32,
    result: Result<bool, String>,
}
impl SummarySupport {
    fn delay(&self) -> Duration {
        let ttl = crate::limits::SQUAD_SUMMARY_CAPABILITY_TTL_MS;
        let ms = if self.result.is_ok() {
            ttl
        } else {
            crate::limits::CLOUD_POLL_IDLE_MS
                .saturating_mul(1u64 << self.failures.saturating_sub(1).min(11))
                .min(ttl)
        };
        Duration::from_millis(ms)
    }
    pub(super) fn cached(
        &self,
        grant: &crate::store::Grant,
        now: Instant,
    ) -> Option<Result<bool, String>> {
        (self.revision == grant.revision
            && self.key == grant.public_key
            && now.saturating_duration_since(self.checked) < self.delay())
        .then(|| self.result.clone())
    }
}

impl Service {
    pub(super) fn publish_summaries(&self) -> Result<bool, RemoteError> {
        let Some(reads) = &self.summary_reads else {
            return Ok(false);
        };
        let now = crate::pairing::now_ms()?;
        let mut eligible = BTreeMap::new();
        {
            let store = self.store()?;
            for config in store.cloud_owner_configs()? {
                let project = config["projectId"].as_str().ok_or_else(unavailable)?;
                for grant in store.cloud_project_peers(project)? {
                    if !grant.live_at(now)
                        || !grant.permits_scope("agents.read")
                        || store
                            .member_for_client(&grant.client_id)?
                            .is_some_and(|member| member.revoked_at_ms.is_some())
                    {
                        continue;
                    }
                    let Some(record) = store.cloud_connection(&grant.client_id)?.filter(|c| {
                        c.confirmed
                            && c.owner_key == grant.public_key
                            && c.config["projectId"] == config["projectId"]
                    }) else {
                        continue;
                    };
                    eligible.insert(grant.client_id.clone(), (grant, record));
                }
            }
        }
        if eligible.len() > crate::limits::SQUAD_SUMMARY_MACHINES {
            return Err(RemoteError::new(
                "REMOTE_SUMMARY_CAPACITY",
                "Summary peer limit reached.",
            ));
        }
        self.summary_schedules
            .lock()
            .map_err(crate::store::database)?
            .retain(|peer, _| eligible.contains_key(peer));
        self.summary_support
            .lock()
            .map_err(crate::store::database)?
            .retain(|peer, _| eligible.contains_key(peer));
        if eligible.is_empty() {
            return Ok(true);
        }
        let mut ready = BTreeMap::new();
        let mut probe = None;
        let mut last_error = None;
        let mut activity = false;
        for (peer, (grant, record)) in eligible {
            let cached = self
                .summary_support
                .lock()
                .map_err(crate::store::database)?
                .get(&peer)
                .and_then(|support| support.cached(&grant, Instant::now()));
            match cached {
                Some(Ok(true)) => {
                    ready.insert(peer, (grant, record));
                }
                Some(Ok(false)) => {}
                Some(Err(code)) => {
                    last_error.get_or_insert(RemoteError::new(
                        &code,
                        "Summary capability lookup is pending.",
                    ));
                }
                None if probe.is_none() => {
                    probe = Some((peer, grant, record));
                }
                None => {}
            }
        }
        let summary = if ready.is_empty() {
            None
        } else {
            let name =
                crate::settings::read(self.layout.directory.parent().ok_or_else(unavailable)?)?
                    .remote_name()
                    .unwrap_or(&self.machine)
                    .to_owned();
            // Serialize the full read, and start the minute interval only after
            // it completes. Other publication ticks reuse its typed outcome.
            let mut sample = self.summary_sample.lock().map_err(crate::store::database)?;
            let mono = || {
                self.summary_clock
                    .elapsed()
                    .as_millis()
                    .min(u64::MAX as u128) as u64
            };
            if sample.due(mono()) {
                let result = reads.squad_summary(&name);
                sample.finished(mono());
                sample.record(result?);
            }
            sample.latest().cloned()
        };
        let wall = crate::pairing::now_ms()?;
        let mono = self
            .summary_clock
            .elapsed()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        let io = self.io();
        for (peer, (grant, mut record)) in ready {
            let (generation, observed) = summary.as_ref().ok_or_else(unavailable)?;
            let next = Prepared::for_peer(observed, &grant.authority()?)?;
            let retained = self
                .store()?
                .summary_last_published(&grant.client_id)?
                .map(|last| Prepared::for_peer(&last, &grant.authority()?))
                .transpose()?;
            let (chosen, due) = {
                let mut schedules = self
                    .summary_schedules
                    .lock()
                    .map_err(crate::store::database)?;
                if !schedules.contains_key(&peer) {
                    let restored = self
                        .store()?
                        .summary_schedule(&peer, wall, mono)?
                        .unwrap_or_default();
                    schedules.insert(peer.clone(), restored);
                }
                let schedule = schedules.get_mut(&peer).ok_or_else(unavailable)?;
                let chosen = schedule.observe_sample(next, retained, *generation, mono);
                (chosen, schedule.due(mono).cloned())
            };
            self.store()?.summary_observed(&grant, &chosen, wall)?;
            let Some(due) = due else {
                continue;
            };
            // A previous peer may have used the cycle deadline. Do not apply
            // that peer's retry hold to untouched peers: they remain due next tick.
            if io.check().is_err() {
                continue;
            }
            activity = true;
            let pending = self.store()?.summary_pending(&grant, &due, wall)?;
            let outcome = (|| {
                let provider = self.provider(&record.config, &peer, false)?;
                let descriptor =
                    self.descriptor(&provider, &grant.public_key, &record.descriptor, &io)?;
                record.descriptor = descriptor.bytes().to_vec();
                self.store()?.save_cloud_connection(&record)?;
                self.reroute(&mut record, &provider, &io)?;
                let sender =
                    Sender::new(provider, self.store.clone(), &record.peer, self.key.clone())?;
                let publication = run(
                    sender.push_summary(
                        &pending,
                        &grant,
                        &descriptor,
                        crate::pairing::now_ms()?,
                        &io,
                    ),
                    &io,
                )?;
                let completed_wall = crate::pairing::now_ms()?;
                let completed_mono = self
                    .summary_clock
                    .elapsed()
                    .as_millis()
                    .min(u64::MAX as u128) as u64;
                match publication {
                    crate::squads::publisher::Publication::Stored => {
                        self.store()?.summary_published(
                            &grant,
                            &pending.id,
                            &pending.payload,
                            completed_wall,
                        )?;
                        let (_, published) = crate::squads::store::frame(&pending.payload)?;
                        let hash = Prepared::published_fingerprint(&published)?;
                        if let Some(schedule) = self
                            .summary_schedules
                            .lock()
                            .map_err(crate::store::database)?
                            .get_mut(&peer)
                        {
                            schedule.published(hash, completed_mono);
                        }
                    }
                    crate::squads::publisher::Publication::Unobserved => {
                        self.store()?
                            .summary_unobserved(&grant, &pending.id, completed_wall)?;
                        if let Some(schedule) = self
                            .summary_schedules
                            .lock()
                            .map_err(crate::store::database)?
                            .get_mut(&peer)
                        {
                            schedule.unconfirmed(completed_mono);
                            schedule.defer_until(
                                completed_mono
                                    .saturating_add(crate::limits::SQUAD_SUMMARY_HEARTBEAT_MS),
                            );
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = outcome {
                // This is a hold, not a new quota. Even a refused local budget
                // check cannot create a five-second provider/Auth retry loop.
                let attempted = self
                    .store()?
                    .cloud_one_way_outgoing(&record.peer, &pending.id, &pending.payload)?
                    .is_some_and(|row| row.attempted);
                let delay = crate::limits::CLOUD_POLL_IDLE_MS;
                self.store()?
                    .summary_defer(&peer, crate::pairing::now_ms()?.saturating_add(delay))?;
                if let Some(schedule) = self
                    .summary_schedules
                    .lock()
                    .map_err(crate::store::database)?
                    .get_mut(&peer)
                {
                    if attempted {
                        schedule.unconfirmed(
                            self.summary_clock
                                .elapsed()
                                .as_millis()
                                .min(u64::MAX as u128) as u64,
                        );
                    }
                    schedule.defer_until(
                        (self
                            .summary_clock
                            .elapsed()
                            .as_millis()
                            .min(u64::MAX as u128) as u64)
                            .saturating_add(delay),
                    );
                }
                last_error.get_or_insert(error);
            }
        }
        // Known peers publish first. At most one expired/unknown feature lookup
        // follows, so slow absent peers cannot monopolize known peers' work.
        if let Some((peer, grant, mut record)) = probe {
            let prior = self
                .summary_support
                .lock()
                .map_err(crate::store::database)?
                .get(&peer)
                .filter(|old| old.revision == grant.revision && old.key == grant.public_key)
                .map_or(0, |old| old.failures);
            let outcome = (|| {
                let io = self.io();
                io.check().map_err(|_| unavailable())?;
                let provider = self.provider(&record.config, &peer, false)?;
                let descriptor =
                    self.descriptor(&provider, &grant.public_key, &record.descriptor, &io)?;
                record.descriptor = descriptor.bytes().to_vec();
                self.store()?.save_cloud_connection(&record)?;
                self.reroute(&mut record, &provider, &io)?;
                let sender =
                    Sender::new(provider, self.store.clone(), &record.peer, self.key.clone())?;
                run(
                    sender.summary_supported(&descriptor, crate::pairing::now_ms()?, &io),
                    &io,
                )
            })();
            let (result, failures) = match outcome {
                Ok(supported) => {
                    activity = true;
                    (Ok(supported), 0)
                }
                Err(error) => {
                    let code = error.code.clone();
                    last_error.get_or_insert(error);
                    (Err(code), prior.saturating_add(1))
                }
            };
            self.summary_support
                .lock()
                .map_err(crate::store::database)?
                .insert(
                    peer,
                    SummarySupport {
                        revision: grant.revision,
                        key: grant.public_key,
                        checked: Instant::now(),
                        failures,
                        result,
                    },
                );
        }
        last_error.map_or(Ok(activity), Err)
    }
}
impl SummaryReader for Service {
    fn read(&self) -> Result<Vec<Record>, RemoteError> {
        self.store()?.read()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn support_cache_is_scoped_to_key_and_revision_and_backoff_is_bounded() {
        let mut grant = crate::store::Grant {
            client_id: "33333333-3333-4333-8333-333333333333".into(),
            public_key: [8; 32],
            kind: "cli".into(),
            origin: "cli".into(),
            name: "Peer".into(),
            agents: "all".into(),
            scopes: vec!["agents.read".into()],
            mode: "hold".into(),
            issued_at_ms: 1,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        };
        let now = Instant::now();
        let mut support = SummarySupport {
            revision: 1,
            key: grant.public_key,
            checked: now,
            failures: 0,
            result: Ok(false),
        };
        assert_eq!(support.cached(&grant, now), Some(Ok(false)));
        assert!(support.cached(&grant, now + support.delay()).is_none());
        grant.revision = 2;
        assert!(support.cached(&grant, now).is_none());
        grant.revision = 1;
        grant.public_key = [9; 32];
        assert!(support.cached(&grant, now).is_none());
        grant.public_key = [8; 32];
        support.result = Err("REMOTE_CLOUD_UNAVAILABLE".into());
        support.failures = 1;
        assert_eq!(
            support.delay(),
            Duration::from_millis(crate::limits::CLOUD_POLL_IDLE_MS)
        );
        support.failures = 2;
        assert_eq!(
            support.delay(),
            Duration::from_millis(2 * crate::limits::CLOUD_POLL_IDLE_MS)
        );
        support.failures = u32::MAX;
        assert_eq!(
            support.delay(),
            Duration::from_millis(crate::limits::SQUAD_SUMMARY_CAPABILITY_TTL_MS)
        );
    }
}
