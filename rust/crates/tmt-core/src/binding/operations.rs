use super::observation::{named_entry, observe, reconcile, selected_entry};
use super::*;
use crate::{
    endpoint::ProcessIncarnation,
    identity::{self, IdentityError, Lifetime},
};

fn deadline<R, O: BindingEndpoint>(endpoint: &O) -> Result<(), BindingError<R, O::Error>> {
    if endpoint.budget_available() {
        Ok(())
    } else {
        Err(BindingError::Deadline)
    }
}

fn refuse_automatic_legacy<R, O: BindingEndpoint>(
    endpoint: &mut O,
    snapshot: &EndpointSnapshot,
    pane: &PaneObservation,
) -> Result<(), BindingError<R, O::Error>> {
    if pane.marker.is_none()
        && endpoint
            .legacy_marker(&snapshot.server, &pane.id)
            .map_err(BindingError::Endpoint)?
            .is_some()
    {
        return Err(BindingError::Unverified);
    }
    Ok(())
}

/// Reconcile only the exact row observed before host I/O; a concurrent change
/// cannot be detached or retired using a stale host observation.
fn reconcile_before_bind<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    entry: Option<BindingEntry>,
) -> Result<(), BindingError<R::Error, O::Error>> {
    let Some(entry) = entry else {
        return Ok(());
    };
    endpoint.begin_coordination();
    let probe = observe(endpoint, &entry).map_err(BindingError::Endpoint)?;
    deadline(endpoint)?;
    repository.with_binding_transaction(|records| {
        if records.entry_by_id(&entry.identity.id)?.as_ref() != Some(&entry) {
            return Err(BindingError::Unverified);
        }
        if entry.binding.is_some() && evaluate_binding(&entry, &probe) == BindingEvidence::Unknown {
            return Err(BindingError::Unverified);
        }
        reconcile(records, entry, &probe)?;
        Ok(())
    })
}

pub fn bind_identity<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
    name: &str,
    save: bool,
) -> Result<IdentityPresence, BindingError<R::Error, O::Error>> {
    bind_identity_with_creation(repository, endpoint, pane_id, name, save)
        .map(|result| result.presence)
}

pub fn bind_identity_at<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
    target: Option<&BindingTargetEvidence>,
    name: &str,
    save: bool,
) -> Result<IdentityPresence, BindingError<R::Error, O::Error>> {
    bind_identity_with_creation_at(repository, endpoint, pane_id, target, name, save)
        .map(|result| result.presence)
}

pub fn bind_identity_with_creation<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
    name: &str,
    save: bool,
) -> Result<BoundIdentity, BindingError<R::Error, O::Error>> {
    bind_with_origin(
        repository,
        endpoint,
        pane_id,
        None,
        name,
        save,
        BindingOrigin::Explicit,
    )
}

/// A launch or restoration is not an explicit user claim of a legacy marker.
pub fn bind_runtime_identity_with_creation<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
    name: &str,
    save: bool,
) -> Result<BoundIdentity, BindingError<R::Error, O::Error>> {
    bind_with_origin(
        repository,
        endpoint,
        pane_id,
        None,
        name,
        save,
        BindingOrigin::Runtime,
    )
}

pub fn bind_identity_with_creation_at<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
    target: Option<&BindingTargetEvidence>,
    name: &str,
    save: bool,
) -> Result<BoundIdentity, BindingError<R::Error, O::Error>> {
    bind_with_origin(
        repository,
        endpoint,
        pane_id,
        target,
        name,
        save,
        BindingOrigin::Runtime,
    )
}

/// Bind an identity chosen earlier by its UUID. The name is never looked up again, so a rename
/// between selection and binding cannot retarget the bind, and no identity is ever created. An
/// unknown or retired id is `NameNotFound` before the pane changes.
pub fn bind_existing_identity<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
    identity_id: &str,
    save: bool,
) -> Result<BoundIdentity, BindingError<R::Error, O::Error>> {
    endpoint.begin_coordination();
    let preflight = endpoint
        .current_snapshot(&[pane_id.into()])
        .map_err(BindingError::Endpoint)?;
    let pane = target_pane(&preflight, pane_id, None)?;
    refuse_automatic_legacy(endpoint, &preflight, pane)?;
    let entry = repository.with_binding_transaction(|records| {
        records
            .entry_by_id(identity_id)?
            .ok_or_else(|| BindingError::NameNotFound(identity_id.into()))
    })?;
    reconcile_before_bind(repository, endpoint, Some(entry))?;
    let selected = repository.with_binding_transaction(|records| {
        records
            .entry_by_id(identity_id)?
            .ok_or_else(|| BindingError::NameNotFound(identity_id.into()))
    })?;
    let observed = observe_for_binding(repository, endpoint, pane_id, None, &selected, false)?;
    let reserved = repository.with_binding_transaction(|records| {
        reserve_binding(records, selected, &observed, false, save, false)
    })?;
    publish_reserved(repository, endpoint, reserved, None)
}

/// Only the launcher may create an identity with automatic-name provenance.
pub fn bind_auto_identity<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
    name: &str,
    save: bool,
) -> Result<BoundIdentity, BindingError<R::Error, O::Error>> {
    bind_with_origin(
        repository,
        endpoint,
        pane_id,
        None,
        name,
        save,
        BindingOrigin::Automatic,
    )
}

#[derive(Clone, Copy)]
enum BindingOrigin {
    Explicit,
    Runtime,
    Automatic,
}

fn bind_with_origin<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
    target: Option<&BindingTargetEvidence>,
    name: &str,
    save: bool,
    origin: BindingOrigin,
) -> Result<BoundIdentity, BindingError<R::Error, O::Error>> {
    let auto_named = matches!(origin, BindingOrigin::Automatic);
    let allow_legacy = matches!(origin, BindingOrigin::Explicit);
    endpoint.begin_coordination();
    let preflight = endpoint
        .current_snapshot(&[pane_id.into()])
        .map_err(BindingError::Endpoint)?;
    let pane = target_pane(&preflight, pane_id, target)?;
    if !allow_legacy {
        refuse_automatic_legacy(endpoint, &preflight, pane)?;
    }
    // A generated-name collision must not reconcile somebody else's identity.
    // Explicit names retain their normal stale-binding reconciliation.
    if !auto_named {
        let entry = repository.with_binding_transaction(|records| named_entry(records, name))?;
        reconcile_before_bind(repository, endpoint, entry)?;
    }
    let lifetime = if save {
        Lifetime::Saved
    } else {
        Lifetime::Temporary
    };
    let created = if auto_named {
        identity::create_auto_named(repository, name, lifetime)
    } else {
        identity::create_or_resolve(repository, name, lifetime)
    }
    .map_err(|error| match error {
        IdentityError::InvalidName(error) => BindingError::InvalidName(error),
        IdentityError::Repository(error) => BindingError::Repository(error),
    })?;
    // Everything below may fail without undoing the separately committed
    // creation/promotion. Re-read ownership after acquiring this second lock.
    let bound = (|| {
        let selected = repository.with_binding_transaction(|records| {
            records
                .entry_by_id(&created.identity.id)?
                .ok_or(BindingError::Unverified)
        })?;
        let observed = observe_for_binding(
            repository,
            endpoint,
            pane_id,
            target,
            &selected,
            allow_legacy,
        )?;
        let reserved = repository.with_binding_transaction(|records| {
            reserve_binding(records, selected, &observed, created.created, false, false)
        })?;
        publish_reserved(repository, endpoint, reserved, target)
    })();
    bound.map_err(|error| abandon_creation(repository, &created, error))
}

/// Move an exact captured identity after the adapter proved its runtime gone.
/// Unlike unbind, this never retires a temporary identity or clears preferences.
/// Native stop authority remains with the caller, not a stored PID or marker.
pub fn move_identity<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    expected: &BindingEntry,
    preferences: &session::SessionPreferences,
    ended: &ProcessIncarnation,
    target: &BindingTargetEvidence,
) -> Result<BoundIdentity, BindingError<R::Error, O::Error>> {
    let old = expected.binding.as_ref().ok_or(BindingError::Unverified)?;
    if old.session.key.as_ref().map(|key| &key.incarnation) != Some(ended)
        || (old.server.host == target.server.host
            && old.server.socket_path == target.server.socket_path
            && old.pane_id == target.pane_id)
    {
        return Err(BindingError::Unverified);
    }
    let observed = observe_for_binding(
        repository,
        endpoint,
        &target.pane_id,
        Some(target),
        expected,
        false,
    )?;
    if evaluate_binding(
        expected,
        observed
            .old_probe
            .as_ref()
            .unwrap_or(&EndpointProbe::Unknown),
    ) == BindingEvidence::Unknown
    {
        return Err(BindingError::Unverified);
    }
    if observed.occupant.is_some() {
        return Err(BindingError::PaneAlreadyBound);
    }
    let reserved = repository.with_binding_transaction(|records| {
        let current = records
            .entry_by_id(&expected.identity.id)?
            .filter(|current| move_entry_matches(current, expected))
            .ok_or(BindingError::Unverified)?;
        if records.session_preferences(&expected.identity.id)? != *preferences {
            return Err(BindingError::Unverified);
        }
        let current_old = current.binding.as_ref().ok_or(BindingError::Unverified)?;
        if current_old.session.key.as_ref().map(|key| &key.incarnation) != Some(ended) {
            return Err(BindingError::Unverified);
        }
        reserve_binding(records, current, &observed, false, false, true)
    })?;
    publish_reserved(repository, endpoint, reserved, Some(target))
        .map_err(|error| BindingError::Partial(Box::new(error)))
}

fn move_entry_matches(current: &BindingEntry, expected: &BindingEntry) -> bool {
    if current == expected {
        return true;
    }
    let mut settled = expected.clone();
    let Some(binding) = settled.binding.as_mut() else {
        return false;
    };
    let Some(key) = binding.session.key.clone() else {
        return false;
    };
    let Some(owner) = binding.session.launch_owner.clone() else {
        return false;
    };
    let Some(next) = binding.session.record_launched_exit(key, owner) else {
        return false;
    };
    binding.session = next;
    current == &settled
}

/// Host and reservation evidence frozen before a short writer transaction.
struct BindingObservation {
    snapshot: EndpointSnapshot,
    pane: PaneObservation,
    occupant: Option<BindingEntry>,
    target_reservation: Option<BindingReservation>,
    target_stale: bool,
    identity_reservation: Option<BindingReservation>,
    identity_stale: bool,
    orphan_marker: Option<String>,
    legacy_raw: Option<String>,
    old_probe: Option<EndpointProbe>,
    publisher: ProcessIncarnation,
    target_id: String,
    now_ms: u64,
}

fn stale_reservation<R, O: BindingEndpoint>(
    endpoint: &mut O,
    reservation: Option<&BindingReservation>,
    now: u64,
) -> Result<bool, BindingError<R, O::Error>> {
    let Some(reservation) = reservation else {
        return Ok(false);
    };
    if reservation.is_stale(now, session::RuntimeLiveness::Unknown) {
        return Ok(true);
    }
    Ok(endpoint
        .publication_liveness(&reservation.publisher)
        .map_err(BindingError::Endpoint)?
        == session::RuntimeLiveness::Gone)
}

fn observe_for_binding<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
    target: Option<&BindingTargetEvidence>,
    selected: &BindingEntry,
    allow_legacy: bool,
) -> Result<BindingObservation, BindingError<R::Error, O::Error>> {
    endpoint.begin_coordination();
    let mut scope = vec![pane_id.into()];
    if let Some(old) = &selected.binding
        && old.pane_id != pane_id
    {
        scope.push(old.pane_id.clone());
    }
    let snapshot = endpoint
        .current_snapshot(&scope)
        .map_err(BindingError::Endpoint)?;
    let mut pane = target_pane(&snapshot, pane_id, target)?.clone();
    if let Some(fence) = target.and_then(|target| target.discovery.as_ref()) {
        guard_ownership(&pane, fence)?;
    }
    let (occupant, target_reservation, identity_reservation, orphan_marker) = repository
        .with_binding_transaction(|records| {
            let occupant =
                records.entry_by_pane(snapshot.server.host, pane_id, &snapshot.server.server_id)?;
            let target_reservation = records.reservation_by_pane(
                snapshot.server.host,
                &snapshot.server.server_id,
                pane_id,
            )?;
            let identity_reservation = records.reservation_by_identity(&selected.identity.id)?;
            let orphan_marker = match &pane.marker {
                Some(marker)
                    if marker.server_id == snapshot.server.server_id
                        && marker.pane_pid == pane.pane_pid
                        && crate::dispatch::canonical_id(&marker.binding_id)
                        && crate::dispatch::canonical_id(&marker.identity_id)
                        && !records.binding_exists(&marker.binding_id)? =>
                {
                    Some(marker.binding_id.clone())
                }
                _ => None,
            };
            Ok::<_, BindingError<R::Error, O::Error>>((
                occupant,
                target_reservation,
                identity_reservation,
                orphan_marker,
            ))
        })?;
    let legacy_raw = if allow_legacy && pane.marker.is_none() {
        endpoint
            .legacy_marker(&snapshot.server, pane_id)
            .map_err(BindingError::Endpoint)?
    } else {
        None
    };
    let now_ms = endpoint
        .publication_time_ms()
        .ok_or(BindingError::Unverified)?;
    let publisher = endpoint
        .publication_owner()
        .map_err(BindingError::Endpoint)?
        .ok_or(BindingError::Unverified)?;
    let target_stale =
        stale_reservation::<R::Error, O>(endpoint, target_reservation.as_ref(), now_ms)?;
    let identity_stale = if identity_reservation == target_reservation {
        target_stale
    } else {
        stale_reservation::<R::Error, O>(endpoint, identity_reservation.as_ref(), now_ms)?
    };
    let old_probe = if let Some(old) = &selected.binding {
        if old.server.host == snapshot.server.host
            && old.server.socket_path == snapshot.server.socket_path
        {
            Some(EndpointProbe::Live(snapshot.clone()))
        } else {
            Some(observe(endpoint, selected).map_err(BindingError::Endpoint)?)
        }
    } else {
        None
    };
    let already_active = occupant.as_ref().is_some_and(|entry| {
        entry.identity.id == selected.identity.id
            && matches!(
                evaluate_binding(entry, &EndpointProbe::Live(snapshot.clone())),
                BindingEvidence::Active(_)
            )
    });
    if !already_active {
        pane.pane_incarnation = match target.and_then(|target| target.pane_incarnation.clone()) {
            Some(start) => Some(start),
            None => endpoint
                .pane_incarnation(&snapshot.server, pane.pane_pid)
                .unwrap_or(None),
        };
    }
    deadline(endpoint)?;
    Ok(BindingObservation {
        snapshot,
        pane,
        occupant,
        target_reservation,
        target_stale,
        identity_reservation,
        identity_stale,
        orphan_marker,
        legacy_raw,
        old_probe,
        publisher,
        target_id: crate::operation::new_operation_id(),
        now_ms,
    })
}

struct ReservedBinding {
    binding: Binding,
    identity: crate::identity::Identity,
    reservation: Option<BindingReservation>,
    legacy_raw: Option<String>,
    old: Option<BindingEntry>,
    created: bool,
}

fn reserve_binding<R, O>(
    records: &mut dyn BindingRecords<Error = R>,
    selected: BindingEntry,
    observation: &BindingObservation,
    created: bool,
    save: bool,
    moving: bool,
) -> Result<ReservedBinding, BindingError<R, O>> {
    if records.entry_by_id(&selected.identity.id)?.as_ref() != Some(&selected) {
        return Err(BindingError::Unverified);
    }
    let actual = records.entry_by_pane(
        observation.snapshot.server.host,
        &observation.pane.id,
        &observation.snapshot.server.server_id,
    )?;
    if actual != observation.occupant {
        return Err(if actual.is_some() {
            BindingError::PaneAlreadyBound
        } else {
            BindingError::Unverified
        });
    }
    let target_reservation = records.reservation_by_pane(
        observation.snapshot.server.host,
        &observation.snapshot.server.server_id,
        &observation.pane.id,
    )?;
    if target_reservation != observation.target_reservation {
        return Err(if target_reservation.is_some() {
            BindingError::PaneAlreadyBound
        } else {
            BindingError::Unverified
        });
    }
    let identity_reservation = records.reservation_by_identity(&selected.identity.id)?;
    if identity_reservation != observation.identity_reservation {
        return Err(BindingError::Unverified);
    }
    let mut expected_marker = match &observation.orphan_marker {
        Some(id) if !records.binding_exists(id)? => Some(id.clone()),
        Some(_) => return Err(BindingError::Unverified),
        None => None,
    };
    if let Some(reservation) = &target_reservation {
        if !observation.target_stale {
            return Err(BindingError::PaneAlreadyBound);
        }
        match &observation.pane.marker {
            Some(marker)
                if marker.binding_id == reservation.target.id
                    && marker.identity_id == reservation.target.identity_id
                    && marker.server_id == reservation.target.server.server_id
                    && marker.pane_pid == reservation.target.pane_pid =>
            {
                expected_marker = Some(marker.binding_id.clone())
            }
            None => (),
            _ => return Err(BindingError::Unverified),
        }
        if !records.finish_binding_reservation(reservation)? {
            return Err(BindingError::Unverified);
        }
    }
    if let Some(reservation) = &identity_reservation
        && target_reservation
            .as_ref()
            .is_none_or(|target| target.target.id != reservation.target.id)
    {
        if !observation.identity_stale {
            return Err(BindingError::Unverified);
        }
        if !records.finish_binding_reservation(reservation)? {
            return Err(BindingError::Unverified);
        }
    }
    let mut current = None;
    if let Some(occupied) = actual {
        match evaluate_binding(
            &occupied,
            &EndpointProbe::Live(observation.snapshot.clone()),
        ) {
            BindingEvidence::Active(_)
                if occupied.identity.id != selected.identity.id || moving =>
            {
                return Err(BindingError::PaneAlreadyBound);
            }
            BindingEvidence::Active(_) => current = occupied.binding,
            BindingEvidence::Unknown => return Err(BindingError::Unverified),
            BindingEvidence::EndpointLost
                if occupied.identity.id == selected.identity.id
                    && occupied.identity.lifetime == Lifetime::Temporary =>
            {
                return Err(BindingError::Unverified);
            }
            _ => {
                reconcile(
                    records,
                    occupied,
                    &EndpointProbe::Live(observation.snapshot.clone()),
                )?;
            }
        }
    } else if observation.pane.marker.is_some() && expected_marker.is_none() {
        return Err(BindingError::Unverified);
    }
    if !moving
        && let Some(old) = &selected.binding
        && current.as_ref().is_none_or(|item| item.id != old.id)
        && observation
            .occupant
            .as_ref()
            .and_then(|item| item.binding.as_ref())
            .is_none_or(|item| item.id != old.id)
    {
        match evaluate_binding(
            &selected,
            observation
                .old_probe
                .as_ref()
                .unwrap_or(&EndpointProbe::Unknown),
        ) {
            BindingEvidence::Active(_) => return Err(BindingError::NameAlreadyActive),
            BindingEvidence::Unknown => return Err(BindingError::Unverified),
            BindingEvidence::EndpointLost if selected.identity.lifetime == Lifetime::Temporary => {
                return Err(BindingError::Unverified);
            }
            _ => records.detach_binding(&old.id)?,
        }
    }
    let identity = if save && selected.identity.lifetime == Lifetime::Temporary {
        records.save_identity(&selected.identity)?
    } else {
        selected.identity.clone()
    };
    if let Some(binding) = current {
        return Ok(ReservedBinding {
            binding,
            identity,
            reservation: None,
            legacy_raw: None,
            old: None,
            created,
        });
    }
    let expires_at_ms = observation
        .now_ms
        .checked_add(10_000)
        .filter(|value| *value <= crate::limits::MAX_JS_SAFE_INTEGER)
        .ok_or(BindingError::Unverified)?;
    let binding = Binding {
        id: observation.target_id.clone(),
        identity_id: identity.id.clone(),
        server: observation.snapshot.server.clone(),
        pane_id: observation.pane.id.clone(),
        pane_pid: observation.pane.pane_pid,
        pane_incarnation: observation.pane.pane_incarnation.clone(),
        session: Default::default(),
    };
    if moving && selected.binding.is_none() {
        return Err(BindingError::Unverified);
    }
    let old = moving.then_some(selected);
    let reservation = BindingReservation {
        target: binding.clone(),
        old_binding_id: old
            .as_ref()
            .and_then(|entry| entry.binding.as_ref().map(|binding| binding.id.clone())),
        expected_binding_id: expected_marker,
        publisher: observation.publisher.clone(),
        created_at_ms: observation.now_ms,
        expires_at_ms,
    };
    records.begin_binding_reservation(&reservation)?;
    Ok(ReservedBinding {
        binding,
        identity,
        reservation: Some(reservation),
        legacy_raw: observation.legacy_raw.clone(),
        old,
        created,
    })
}

fn compensate_reservation<R: BindingRepository, O>(
    repository: &mut R,
    reserved: &ReservedBinding,
    error: BindingError<R::Error, O>,
) -> BindingError<R::Error, O> {
    let Some(reservation) = &reserved.reservation else {
        return error;
    };
    match repository
        .with_binding_transaction(|records| records.finish_binding_reservation(reservation))
    {
        Ok(_) => error,
        Err(cleanup) => BindingError::CleanupFailed {
            error: Box::new(error),
            cleanup,
        },
    }
}

pub(super) fn publication_can_start(
    reservation: &BindingReservation,
    now: u64,
    budget_available: bool,
) -> bool {
    let cutoff = reservation.expires_at_ms.saturating_sub(1_000);
    budget_available && now.checked_add(3_000).is_some_and(|end| end <= cutoff)
}

enum BindingActivation {
    Activated(Box<(Binding, crate::identity::Identity)>),
    Detached,
    Changed,
}

fn publish_reserved<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    reserved: ReservedBinding,
    target: Option<&BindingTargetEvidence>,
) -> Result<BoundIdentity, BindingError<R::Error, O::Error>> {
    let result = (|| {
        if let Some(reservation) = &reserved.reservation {
            let now = endpoint
                .publication_time_ms()
                .ok_or(BindingError::Unverified)?;
            if !publication_can_start(reservation, now, endpoint.budget_available()) {
                return Err(BindingError::Deadline);
            }
            let published = if let Some(raw) = reserved.legacy_raw.as_deref() {
                endpoint.publish_if_legacy(&reserved.binding, &reserved.identity, raw)
            } else {
                endpoint.publish_if(
                    &reserved.binding,
                    &reserved.identity,
                    reservation.expected_binding_id.as_deref(),
                )
            }
            .map_err(BindingError::Endpoint)?;
            if !published {
                return Err(BindingError::Unverified);
            }
        }
        let verified = endpoint
            .current_snapshot(std::slice::from_ref(&reserved.binding.pane_id))
            .map_err(BindingError::Endpoint)?;
        target_pane(&verified, &reserved.binding.pane_id, target)?;
        let expected = BindingEntry {
            identity: reserved.identity.clone(),
            binding: Some(reserved.binding.clone()),
        };
        let BindingEvidence::Active(pane) =
            evaluate_binding(&expected, &EndpointProbe::Live(verified))
        else {
            return Err(BindingError::Unverified);
        };
        deadline(endpoint)?;
        let activation = repository.with_binding_transaction(|records| {
            let current = records.entry_by_id(&reserved.identity.id)?;
            let Some(current) = current else {
                return Ok(BindingActivation::Detached);
            };
            if let Some(reservation) = &reserved.reservation {
                if records
                    .reservation_by_target(&reserved.binding.id)?
                    .as_ref()
                    != Some(reservation)
                {
                    return Ok(BindingActivation::Detached);
                }
                if let Some(old) = &reserved.old {
                    if !move_entry_matches(&current, old) {
                        return Ok(BindingActivation::Changed);
                    }
                    let old_id = old
                        .binding
                        .as_ref()
                        .ok_or(BindingError::Unverified)?
                        .id
                        .clone();
                    records.detach_binding(&old_id)?;
                } else if current.binding.is_some() {
                    return Ok(BindingActivation::Changed);
                }
                let binding = records.insert_reserved_binding(&current.identity, reservation)?;
                if !records.finish_binding_reservation(reservation)? {
                    return Err(BindingError::Unverified);
                }
                Ok(BindingActivation::Activated(Box::new((
                    binding,
                    current.identity,
                ))))
            } else if current.binding.as_ref() == Some(&reserved.binding) {
                records.touch_binding(&reserved.binding.id)?;
                Ok(BindingActivation::Activated(Box::new((
                    reserved.binding.clone(),
                    current.identity,
                ))))
            } else {
                Ok(BindingActivation::Detached)
            }
        })?;
        match activation {
            BindingActivation::Activated(activated) => {
                let (binding, identity) = *activated;
                if let Some(old) = &reserved.old
                    && let Some(old_binding) = &old.binding
                {
                    // After activation, an unchanged old marker is orphaned and
                    // safe to reclaim. Clear is best-effort and never unbinds the target.
                    if let Err(error) = endpoint.clear_if_owned(old_binding, &old.identity)
                        && endpoint.cleanup_failed(&error)
                    {
                        return Err(BindingError::Endpoint(error));
                    }
                }
                Ok(BoundIdentity {
                    presence: IdentityPresence {
                        identity,
                        presence: Presence::Active,
                        pane: Some(*pane),
                        binding: Some(binding),
                    },
                    created: reserved.created,
                })
            }
            BindingActivation::Detached | BindingActivation::Changed => {
                if reserved.reservation.is_some()
                    && let Err(error) =
                        endpoint.clear_if_owned(&reserved.binding, &reserved.identity)
                    && endpoint.cleanup_failed(&error)
                {
                    return Err(BindingError::Endpoint(error));
                }
                Err(BindingError::Unverified)
            }
        }
    })();
    result.map_err(|error| compensate_reservation(repository, &reserved, error))
}

/// Guarded selection and binding reservation share one short writer transaction;
/// host reobservations and publication run outside that writer.
pub fn bind_guarded<R: BindingRepository, O: GuardedBindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    fence: &crate::discovery::DiscoveryTarget,
    selector: GuardedIdentity<'_>,
    save: bool,
) -> Result<GuardedBoundIdentity, BindingError<R::Error, O::Error>> {
    let name = match selector {
        GuardedIdentity::Name(name) => {
            Some(crate::names::validate_name(name).map_err(BindingError::InvalidName)?)
        }
        GuardedIdentity::Uuid(_) => None,
    };
    endpoint.begin_coordination();
    let admit = |endpoint: &mut O| -> Result<(), BindingError<R::Error, O::Error>> {
        let observed = endpoint
            .discovery_target(&fence.pane)
            .map_err(BindingError::Endpoint)?;
        if observed
            .as_ref()
            .is_none_or(|actual| actual.digest() != fence.digest())
        {
            return Err(BindingError::TargetChanged(fence.pane.clone()));
        }
        Ok(())
    };
    admit(endpoint)?;
    let selected = repository.with_binding_transaction(|records| match selector {
        GuardedIdentity::Uuid(id) => records
            .entry_by_id(id)?
            .map(Some)
            .ok_or_else(|| BindingError::NameNotFound(id.into())),
        GuardedIdentity::Name(_) => {
            let name = name.as_ref().expect("validated name");
            if records.find_identity(name.canonical_name())?.is_some() {
                return Err(BindingError::NameTaken(name.display_name().into()));
            }
            Ok(None)
        }
    })?;
    let occupied =
        repository.with_binding_transaction(|records| match fence.server_id.as_deref() {
            Some(server) => records.entry_by_pane(fence.host, &fence.pane, server),
            None => Ok(None),
        })?;
    if let Some(occupied) = &occupied {
        if selected
            .as_ref()
            .is_none_or(|entry| entry.identity.id != occupied.identity.id)
        {
            return Err(BindingError::PaneAlreadyBound);
        }
        let snapshot = fence.endpoint_snapshot().ok_or(BindingError::Unverified)?;
        if !matches!(
            evaluate_binding(occupied, &EndpointProbe::Live(snapshot)),
            BindingEvidence::Active(_)
        ) {
            return Err(BindingError::Unverified);
        }
    } else if fence.marker.is_some() {
        return Err(BindingError::Unverified);
    }
    if let Some(entry) = &selected
        && let Some(old) = &entry.binding
        && occupied.is_none()
    {
        let probe = observe(endpoint, entry).map_err(BindingError::Endpoint)?;
        match evaluate_binding(entry, &probe) {
            BindingEvidence::Active(_) => return Err(BindingError::NameAlreadyActive),
            BindingEvidence::Unknown => return Err(BindingError::Unverified),
            _ => (),
        }
        if old.pane_id == fence.pane && old.server.socket_path == fence.socket {
            return Err(BindingError::TargetChanged(fence.pane.clone()));
        }
    }
    admit(endpoint)?;
    deadline(endpoint)?;
    // Snapshot may initialize a host marker; errors after this point remain a
    // partial attempt, never a claim that host effects were rolled back.
    let result = (|| {
        let snapshot = endpoint
            .current_snapshot(std::slice::from_ref(&fence.pane))
            .map_err(BindingError::Endpoint)?;
        let pane = target_pane(&snapshot, &fence.pane, None)?;
        if snapshot.server.socket_path != fence.socket
            || snapshot.server.server_pid != fence.server.pid()
            || snapshot.server.server_start_time != fence.server_start_time
            || fence
                .server_id
                .as_ref()
                .is_some_and(|id| id != &snapshot.server.server_id)
            || pane.pane_pid != fence.process.pid()
            || endpoint
                .pane_incarnation(&snapshot.server, pane.pane_pid)
                .map_err(BindingError::Endpoint)?
                .as_deref()
                != Some(fence.process.start_identity())
        {
            return Err(BindingError::TargetChanged(fence.pane.clone()));
        }
        guard_ownership(pane, fence)?;
        let target = BindingTargetEvidence {
            server: snapshot.server,
            pane_id: fence.pane.clone(),
            pane_pid: fence.process.pid(),
            pane_incarnation: Some(fence.process.start_identity().into()),
            discovery: Some(fence.clone()),
        };
        let created = selected.is_none();
        let captured = selected.clone().unwrap_or_else(|| BindingEntry {
            identity: crate::identity::Identity {
                id: String::new(),
                name: name.as_ref().expect("name mode").display_name().into(),
                canonical_name: name.as_ref().expect("name mode").canonical_name().into(),
                lifetime: if save {
                    Lifetime::Saved
                } else {
                    Lifetime::Temporary
                },
                created_at: String::new(),
                updated_at: String::new(),
            },
            binding: None,
        });
        let observed = observe_for_binding(
            repository,
            endpoint,
            &fence.pane,
            Some(&target),
            &captured,
            false,
        )?;
        if let Some(occupied) = &observed.occupant
            && selected
                .as_ref()
                .is_none_or(|entry| entry.identity.id != occupied.identity.id)
        {
            return Err(BindingError::PaneAlreadyBound);
        }
        if observed.occupant.is_none() && fence.marker.is_some() {
            return Err(BindingError::Unverified);
        }
        let already_bound = observed.occupant.is_some();
        let reserved = repository.with_binding_transaction(|records| {
            let selected = match selected {
                Some(selected) => selected,
                None => {
                    let name = name.as_ref().expect("name mode");
                    if records.find_identity(name.canonical_name())?.is_some() {
                        return Err(BindingError::NameTaken(name.display_name().into()));
                    }
                    BindingEntry {
                        identity: records.insert_identity(
                            name,
                            if save {
                                Lifetime::Saved
                            } else {
                                Lifetime::Temporary
                            },
                        )?,
                        binding: None,
                    }
                }
            };
            reserve_binding(records, selected, &observed, created, save, false)
        })?;
        publish_reserved(repository, endpoint, reserved, Some(&target)).map(|bound| {
            GuardedBoundIdentity {
                bound,
                already_bound,
            }
        })
    })();
    result.map_err(|error| BindingError::Partial(Box::new(error)))
}

/// A deterministic refusal (the pane or name is taken, or the pane changed)
/// must not leave the identity this invocation created behind: no binding
/// row exists, so no later observation would ever retire it. Uncertain
/// outcomes (`Unverified`, publication, deadline) keep it for a retry.
/// Retirement re-checks, in one transaction, that the identity is still
/// temporary, unretired and unbound, so a concurrent successful bind wins.
fn abandon_creation<R: BindingRepository, O>(
    repository: &mut R,
    created: &identity::CreatedIdentity,
    error: BindingError<R::Error, O>,
) -> BindingError<R::Error, O> {
    let refused = matches!(
        error,
        BindingError::PaneAlreadyBound
            | BindingError::NameAlreadyActive
            | BindingError::PaneNotFound(_)
            | BindingError::TargetChanged(_)
    );
    if !refused || !created.created || created.identity.lifetime != Lifetime::Temporary {
        return error;
    }
    let retired = repository.with_binding_transaction(|records| {
        if let Some(entry) = records.entry_by_id(&created.identity.id)?
            && entry.binding.is_none()
            && entry.identity.lifetime == Lifetime::Temporary
            && records
                .reservation_by_identity(&entry.identity.id)?
                .is_none()
        {
            records.retire_identity(&entry.identity, true)?;
        }
        Ok::<_, R::Error>(())
    });
    match retired {
        Ok(()) => error,
        Err(cleanup) => BindingError::CleanupFailed {
            error: Box::new(error),
            cleanup,
        },
    }
}

fn guard_ownership<R, O>(
    pane: &PaneObservation,
    fence: &crate::discovery::DiscoveryTarget,
) -> Result<(), BindingError<R, O>> {
    let ownership = |marker: &crate::endpoint::BindingMarker| {
        (
            marker.identity_id.clone(),
            marker.binding_id.clone(),
            marker.server_id.clone(),
            marker.pane_pid,
        )
    };
    if pane.marker.as_ref().map(ownership) != fence.marker.as_ref().map(ownership)
        || pane.suggested_name != fence.runtime
    {
        return Err(BindingError::TargetChanged(pane.id.clone()));
    }
    Ok(())
}

fn target_pane<'a, R, O>(
    snapshot: &'a EndpointSnapshot,
    pane_id: &str,
    target: Option<&BindingTargetEvidence>,
) -> Result<&'a PaneObservation, BindingError<R, O>> {
    let pane = snapshot
        .panes
        .iter()
        .find(|pane| pane.id == pane_id)
        .ok_or_else(|| BindingError::PaneNotFound(pane_id.into()))?;
    if target.is_some_and(|target| {
        target.pane_id != pane_id
            || target.server != snapshot.server
            || target.pane_pid != pane.pane_pid
            || target
                .pane_incarnation
                .as_ref()
                .is_some_and(|start| pane.pane_incarnation.as_ref() != Some(start))
    }) {
        return Err(BindingError::TargetChanged(pane_id.into()));
    }
    Ok(pane)
}

/// A staged mutation may commit only while both the captured row and its
/// identity/pane publication fences are unchanged and unreserved.
fn entry_is_unchanged_and_unreserved<R>(
    records: &dyn BindingRecords<Error = R>,
    entry: &BindingEntry,
) -> Result<bool, R> {
    if records.entry_by_id(&entry.identity.id)?.as_ref() != Some(entry)
        || records
            .reservation_by_identity(&entry.identity.id)?
            .is_some()
    {
        return Ok(false);
    }
    if let Some(binding) = &entry.binding
        && records
            .reservation_by_pane(
                binding.server.host,
                &binding.server.server_id,
                &binding.pane_id,
            )?
            .is_some()
    {
        return Ok(false);
    }
    Ok(true)
}

#[derive(Clone, Copy)]
enum Ending {
    Unbind,
    Remove,
    FailedAutomatic,
}

/// Non-atomic hosts keep the exact pane reserved until their clear finishes.
/// The ordinary publication lease and deadline protect the normal case; the
/// documented lease-expiry limitation remains for an indefinitely paused host.
fn end_captured_binding<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    entry: &BindingEntry,
    evidence: &BindingEvidence,
    ending: Ending,
) -> Result<bool, BindingError<R::Error, O::Error>> {
    let active = matches!(evidence, BindingEvidence::Active(_));
    let binding = entry.binding.as_ref();
    let conditional = if active {
        endpoint
            .conditional_clear(binding.expect("active evidence has a binding"))
            .map_err(BindingError::Endpoint)?
    } else {
        true
    };
    let mut cleanup_error = None;
    let reservation = if active && !conditional {
        let binding = binding.expect("active evidence has a binding");
        let owner = endpoint
            .publication_owner()
            .map_err(BindingError::Endpoint)?
            .ok_or(BindingError::Unverified)?;
        let now = endpoint
            .publication_time_ms()
            .ok_or(BindingError::Unverified)?;
        let expires_at_ms = now
            .checked_add(10_000)
            .filter(|value| *value <= crate::limits::MAX_JS_SAFE_INTEGER)
            .ok_or(BindingError::Unverified)?;
        let mut target = binding.clone();
        target.session = Default::default();
        let reservation = BindingReservation {
            target,
            old_binding_id: None,
            expected_binding_id: Some(binding.id.clone()),
            publisher: owner,
            created_at_ms: now,
            expires_at_ms,
        };
        let acquired = repository.with_binding_transaction::<_, R::Error>(|records| {
            if !entry_is_unchanged_and_unreserved(records, entry)?
                || matches!(ending, Ending::FailedAutomatic)
                    && !records.is_auto_named(&entry.identity.id)?
            {
                return Ok(false);
            }
            records.begin_binding_reservation(&reservation)?;
            Ok(true)
        })?;
        if !acquired {
            return if matches!(ending, Ending::FailedAutomatic) {
                Ok(false)
            } else {
                Err(BindingError::Unverified)
            };
        }
        let now = endpoint.publication_time_ms();
        if now.is_some_and(|now| {
            publication_can_start(&reservation, now, endpoint.budget_available())
        }) && let Err(error) = endpoint.clear(binding, &entry.identity)
            && endpoint.cleanup_failed(&error)
        {
            cleanup_error = Some(error);
        }
        Some(reservation)
    } else {
        None
    };
    let changed = repository.with_binding_transaction::<_, R::Error>(|records| {
        let unchanged = records.entry_by_id(&entry.identity.id)?.as_ref() == Some(entry)
            && (!matches!(ending, Ending::FailedAutomatic)
                || records.is_auto_named(&entry.identity.id)?);
        let allowed = match &reservation {
            Some(reservation) => {
                records
                    .reservation_by_target(&reservation.target.id)?
                    .as_ref()
                    == Some(reservation)
            }
            None => entry_is_unchanged_and_unreserved(records, entry)?,
        };
        if let Some(reservation) = &reservation
            && !records.finish_binding_reservation(reservation)?
        {
            return Ok(false);
        }
        if !unchanged || !allowed {
            return Ok(false);
        }
        match ending {
            Ending::Unbind if entry.identity.lifetime == Lifetime::Temporary => {
                records.retire_identity(&entry.identity, false)?
            }
            Ending::Unbind => {
                if let Some(binding) = binding {
                    records.detach_binding(&binding.id)?;
                }
            }
            Ending::Remove | Ending::FailedAutomatic => {
                records.retire_identity(&entry.identity, true)?
            }
        }
        Ok(true)
    })?;
    if !changed {
        return if matches!(ending, Ending::FailedAutomatic) {
            Ok(false)
        } else {
            Err(BindingError::Unverified)
        };
    }
    if active
        && conditional
        && let Err(error) = endpoint.clear_if_owned(
            binding.expect("active evidence has a binding"),
            &entry.identity,
        )
        && endpoint.cleanup_failed(&error)
    {
        cleanup_error = Some(error);
    }
    if let Some(error) = cleanup_error {
        return Err(BindingError::Endpoint(error));
    }
    Ok(true)
}

pub fn unbind_identity<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
) -> Result<Option<UnboundIdentity>, BindingError<R::Error, O::Error>> {
    let candidates = repository.with_binding_transaction::<_, R::Error>(|records| {
        records.entries_for_pane(endpoint.current_host(), pane_id)
    })?;
    endpoint.begin_coordination();
    let snapshot = endpoint
        .current_snapshot(&[pane_id.into()])
        .map_err(BindingError::Endpoint)?;
    target_pane(&snapshot, pane_id, None)?;
    let Some(entry) = candidates.into_iter().find(|entry| {
        entry
            .binding
            .as_ref()
            .is_some_and(|binding| binding.server.server_id == snapshot.server.server_id)
    }) else {
        return Ok(None);
    };
    let evidence = evaluate_binding(&entry, &EndpointProbe::Live(snapshot));
    if entry.binding.is_some() && evidence == BindingEvidence::Unknown {
        return Err(BindingError::Unverified);
    }
    deadline(endpoint)?;
    let retired = entry.identity.lifetime == Lifetime::Temporary;
    end_captured_binding(repository, endpoint, &entry, &evidence, Ending::Unbind)?;
    let binding = matches!(evidence, BindingEvidence::Active(_))
        .then(|| entry.binding.clone())
        .flatten();
    Ok(Some(UnboundIdentity {
        identity: entry.identity,
        retired,
        binding,
    }))
}

pub fn remove_identity<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    name: &str,
    force: bool,
) -> Result<BindingEntry, BindingError<R::Error, O::Error>> {
    let entry = repository.with_binding_transaction(|records| {
        selected_entry(records, name)?.ok_or_else(|| BindingError::NameNotFound(name.into()))
    })?;
    if entry.identity.lifetime == Lifetime::Saved && !force {
        return Err(BindingError::ConfirmationRequired);
    }
    endpoint.begin_coordination();
    let evidence = if entry.binding.is_some() {
        let probe = observe(endpoint, &entry).map_err(BindingError::Endpoint)?;
        evaluate_binding(&entry, &probe)
    } else {
        BindingEvidence::EndpointLost
    };
    if entry.binding.is_some() && evidence == BindingEvidence::Unknown {
        return Err(BindingError::Unverified);
    }
    deadline(endpoint)?;
    end_captured_binding(repository, endpoint, &entry, &evidence, Ending::Remove)?;
    Ok(BindingEntry {
        identity: entry.identity,
        binding: matches!(evidence, BindingEvidence::Active(_))
            .then_some(entry.binding)
            .flatten(),
    })
}

/// Renames by name in one binding transaction, so it serializes with binding
/// and removal. Everything keyed by the UUID follows. The pane marker and badge
/// are cosmetic and refreshed by the caller after commit; a stale marker name
/// never detaches (see `evaluate_binding`).
pub fn rename_identity<R: BindingRepository, O>(
    repository: &mut R,
    old: &str,
    new: &str,
) -> Result<RenamedIdentity, BindingError<R::Error, O>> {
    let name = crate::names::validate_name(new).map_err(BindingError::InvalidName)?;
    repository.with_binding_transaction(|records| {
        let entry =
            selected_entry(records, old)?.ok_or_else(|| BindingError::NameNotFound(old.into()))?;
        rename_entry(records, entry, &name)
    })
}

/// Rename only the exact active UUID whose current display name matches the
/// caller's preview. Selection, expectation and collision checks share the same
/// writer transaction as ordinary name-based rename, bind and retirement.
pub fn rename_identity_by_id<R: BindingRepository, O>(
    repository: &mut R,
    identity_id: &str,
    expected_name: &str,
    new: &str,
) -> Result<RenameOutcome, BindingError<R::Error, O>> {
    let name = crate::names::validate_name(new).map_err(BindingError::InvalidName)?;
    repository.with_binding_transaction(|records| {
        let Some(entry) = records.entry_by_id(identity_id)? else {
            return Ok(RenameOutcome::NotFound);
        };
        if entry.identity.name != expected_name {
            return Ok(RenameOutcome::Conflict(entry.identity));
        }
        rename_entry(records, entry, &name).map(|result| RenameOutcome::Applied(Box::new(result)))
    })
}

fn rename_entry<R, O>(
    records: &mut dyn BindingRecords<Error = R>,
    entry: BindingEntry,
    name: &ValidatedName,
) -> Result<RenamedIdentity, BindingError<R, O>> {
    if name.display_name() == entry.identity.name {
        return Ok(RenamedIdentity {
            identity: entry.identity.clone(),
            previous: entry.identity,
            binding: entry.binding,
        });
    }
    // A case-only change keeps the canonical name, so the holder is itself.
    if name.canonical_name() != entry.identity.canonical_name
        && records.find_identity(name.canonical_name())?.is_some()
    {
        return Err(BindingError::NameTaken(name.display_name().into()));
    }
    let identity = records.rename_identity(&entry.identity, name)?;
    Ok(RenamedIdentity {
        previous: entry.identity,
        identity,
        binding: entry.binding,
    })
}

/// Name only the verified caller's auto-named identity. Observe the host outside
/// the writer, then recheck ownership, provenance and collisions before promotion.
/// Ordinary panes return None and retain the existing bind policy.
pub fn name_auto_identity<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
    new: &str,
    save: bool,
) -> Result<Option<BoundIdentity>, BindingError<R::Error, O::Error>> {
    let name = crate::names::validate_name(new).map_err(BindingError::InvalidName)?;
    let candidates = repository.with_binding_transaction::<_, R::Error>(|records| {
        records
            .entries_for_pane(endpoint.current_host(), pane_id)?
            .into_iter()
            .map(|entry| Ok((records.is_auto_named(&entry.identity.id)?, entry)))
            .collect::<Result<Vec<_>, R::Error>>()
    })?;
    endpoint.begin_coordination();
    let snapshot = endpoint
        .current_snapshot(&[pane_id.into()])
        .map_err(BindingError::Endpoint)?;
    target_pane(&snapshot, pane_id, None)?;
    let Some((automatic, entry)) = candidates.into_iter().find(|(_, entry)| {
        entry
            .binding
            .as_ref()
            .is_some_and(|binding| binding.server.server_id == snapshot.server.server_id)
    }) else {
        return Ok(None);
    };
    if !automatic {
        return Ok(None);
    }
    let BindingEvidence::Active(pane) = evaluate_binding(&entry, &EndpointProbe::Live(snapshot))
    else {
        return Err(BindingError::Unverified);
    };
    deadline(endpoint)?;
    repository.with_binding_transaction(|records| {
        if !entry_is_unchanged_and_unreserved(records, &entry)? {
            return Err(BindingError::Unverified);
        }
        if !records.is_auto_named(&entry.identity.id)? {
            return Ok(None);
        }
        if records
            .find_identity(name.canonical_name())?
            .is_some_and(|other| other.id != entry.identity.id)
        {
            return Err(BindingError::NameTaken(name.display_name().into()));
        }
        let mut identity = records.rename_identity(&entry.identity, &name)?;
        if save && identity.lifetime == Lifetime::Temporary {
            identity = records.save_identity(&identity)?;
        }
        Ok(Some(BoundIdentity {
            presence: IdentityPresence {
                identity,
                binding: entry.binding,
                presence: Presence::Active,
                pane: Some(*pane),
            },
            created: false,
        }))
    })
}

/// A spawn failed before any child existed. Retire only this exact unchanged
/// temporary automatic binding, never a replacement or a saved identity.
pub fn retire_failed_auto_launch<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    binding: &Binding,
) -> Result<(), BindingError<R::Error, O::Error>> {
    let entry = repository.with_binding_transaction::<_, R::Error>(|records| {
        let Some(entry) = records.entry_by_id(&binding.identity_id)? else {
            return Ok(None);
        };
        if entry.identity.lifetime != Lifetime::Temporary
            || entry.binding.as_ref() != Some(binding)
            || !records.is_auto_named(&entry.identity.id)?
        {
            return Ok(None);
        }
        Ok(Some(entry))
    })?;
    let Some(entry) = entry else {
        return Ok(());
    };
    endpoint.begin_coordination();
    let probe = observe(endpoint, &entry).map_err(BindingError::Endpoint)?;
    let evidence = evaluate_binding(&entry, &probe);
    if evidence == BindingEvidence::Unknown {
        return Err(BindingError::Unverified);
    }
    deadline(endpoint)?;
    end_captured_binding(
        repository,
        endpoint,
        &entry,
        &evidence,
        Ending::FailedAutomatic,
    )?;
    Ok(())
}
