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
    bind_identity_with_creation_at(repository, endpoint, pane_id, None, name, save)
}

pub fn bind_identity_with_creation_at<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
    target: Option<&BindingTargetEvidence>,
    name: &str,
    save: bool,
) -> Result<BoundIdentity, BindingError<R::Error, O::Error>> {
    bind_with_origin(repository, endpoint, pane_id, target, name, save, false)
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
    target_pane(&preflight, pane_id, None)?;
    repository.with_binding_transaction(|records| {
        endpoint.begin_coordination();
        let entry = records
            .entry_by_id(identity_id)?
            .ok_or_else(|| BindingError::NameNotFound(identity_id.into()))?;
        let probe = observe(endpoint, &entry).map_err(BindingError::Endpoint)?;
        if entry.binding.is_some() && evaluate_binding(&entry, &probe) == BindingEvidence::Unknown {
            return Err(BindingError::Unverified);
        }
        reconcile(records, entry, &probe)?;
        deadline(endpoint)
    })?;
    repository.with_binding_transaction(|records| {
        endpoint.begin_coordination();
        let mut selected = records
            .entry_by_id(identity_id)?
            .ok_or_else(|| BindingError::NameNotFound(identity_id.into()))?;
        if save && selected.identity.lifetime == Lifetime::Temporary {
            selected.identity = records.save_identity(&selected.identity)?;
        }
        bind_selected(records, endpoint, pane_id, None, selected, false)
    })
}

/// Only the launcher may create an identity with automatic-name provenance.
pub fn bind_auto_identity<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
    name: &str,
    save: bool,
) -> Result<BoundIdentity, BindingError<R::Error, O::Error>> {
    bind_with_origin(repository, endpoint, pane_id, None, name, save, true)
}

fn bind_with_origin<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
    target: Option<&BindingTargetEvidence>,
    name: &str,
    save: bool,
    auto_named: bool,
) -> Result<BoundIdentity, BindingError<R::Error, O::Error>> {
    endpoint.begin_coordination();
    let preflight = endpoint
        .current_snapshot(&[pane_id.into()])
        .map_err(BindingError::Endpoint)?;
    target_pane(&preflight, pane_id, target)?;
    // A generated-name collision must not reconcile somebody else's identity.
    // Explicit names retain their normal stale-binding reconciliation.
    if !auto_named {
        repository.with_binding_transaction(|records| {
            endpoint.begin_coordination();
            if let Some(entry) = named_entry(records, name)? {
                let probe = observe(endpoint, &entry).map_err(BindingError::Endpoint)?;
                if entry.binding.is_some()
                    && evaluate_binding(&entry, &probe) == BindingEvidence::Unknown
                {
                    return Err(BindingError::Unverified);
                }
                reconcile(records, entry, &probe)?;
            }
            deadline(endpoint)
        })?;
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
    let bound = repository.with_binding_transaction(|records| {
        endpoint.begin_coordination();
        let selected = records
            .entry_by_id(&created.identity.id)?
            .ok_or(BindingError::Unverified)?;
        bind_selected(
            records,
            endpoint,
            pane_id,
            target,
            selected,
            created.created,
        )
    });
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
    repository.with_binding_transaction(|records| {
        endpoint.begin_coordination();
        let current = records
            .entry_by_id(&expected.identity.id)?
            .filter(|current| {
                if current == expected {
                    return true;
                }
                // The stopped launch's wrapper may settle after the caller's
                // post-stop snapshot. Admit only its exact Ended observation;
                // every identity, attachment, key and owner field stays fenced.
                let mut settled = expected.clone();
                let Some(binding) = settled.binding.as_mut() else {
                    return false;
                };
                let Some(key) = binding
                    .session
                    .key
                    .as_ref()
                    .filter(|key| &key.incarnation == ended)
                else {
                    return false;
                };
                let Some(owner) = binding.session.launch_owner.clone() else {
                    return false;
                };
                let Some(next) = binding.session.record_launched_exit(key.clone(), owner) else {
                    return false;
                };
                binding.session = next;
                current == &settled
            })
            .ok_or(BindingError::Unverified)?;
        if records.session_preferences(&expected.identity.id)? != *preferences {
            return Err(BindingError::Unverified);
        }
        let old = current.binding.as_ref().ok_or(BindingError::Unverified)?;
        if old.session.key.as_ref().map(|key| &key.incarnation) != Some(ended)
            || (old.server.host == target.server.host
                && old.server.socket_path == target.server.socket_path
                && old.pane_id == target.pane_id)
        {
            return Err(BindingError::Unverified);
        }
        // Refuse an occupied or changed destination before clearing anything.
        let snapshot = endpoint
            .current_snapshot(std::slice::from_ref(&target.pane_id))
            .map_err(BindingError::Endpoint)?;
        target_pane(&snapshot, &target.pane_id, Some(target))?;
        if records
            .entry_by_pane(
                snapshot.server.host,
                &target.pane_id,
                &snapshot.server.server_id,
            )?
            .is_some()
        {
            return Err(BindingError::PaneAlreadyBound);
        }
        let probe = observe(endpoint, &current).map_err(BindingError::Endpoint)?;
        let evidence = evaluate_binding(&current, &probe);
        deadline(endpoint)?;
        // Host publication cannot be rolled back with SQL. Any error after
        // this boundary is partial and never grants automatic replay.
        (|| {
            clear_observed(endpoint, &current, &evidence)?;
            records.detach_binding(&old.id)?;
            bind_selected(
                records,
                endpoint,
                &target.pane_id,
                Some(target),
                BindingEntry {
                    identity: current.identity.clone(),
                    binding: None,
                },
                false,
            )
        })()
        .map_err(|error| BindingError::Partial(Box::new(error)))
    })
}

// Ordinary, guarded and moved bindings use this binding/publication owner.
fn bind_selected<R, O: BindingEndpoint>(
    records: &mut dyn BindingRecords<Error = R>,
    endpoint: &mut O,
    pane_id: &str,
    target: Option<&BindingTargetEvidence>,
    selected: BindingEntry,
    created: bool,
) -> Result<BoundIdentity, BindingError<R, O::Error>> {
    let mut scope = vec![pane_id.into()];
    if let Some(binding) = &selected.binding
        && binding.pane_id != pane_id
    {
        scope.push(binding.pane_id.clone());
    }
    let snapshot = endpoint
        .current_snapshot(&scope)
        .map_err(BindingError::Endpoint)?;
    let pane = target_pane(&snapshot, pane_id, target)?;
    if let Some(fence) = target.and_then(|target| target.discovery.as_ref()) {
        guard_ownership(pane, fence)?;
    }
    let mut current = None;
    if let Some(occupied) =
        records.entry_by_pane(snapshot.server.host, pane_id, &snapshot.server.server_id)?
    {
        match evaluate_binding(&occupied, &EndpointProbe::Live(snapshot.clone())) {
            BindingEvidence::Active(_) if occupied.identity.id != selected.identity.id => {
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
                reconcile(records, occupied, &EndpointProbe::Live(snapshot.clone()))?;
            }
        }
    }
    if let Some(old) = &selected.binding
        && current.as_ref().is_none_or(|current| current.id != old.id)
    {
        let probe = if old.server.host == snapshot.server.host
            && old.server.socket_path == snapshot.server.socket_path
        {
            EndpointProbe::Live(snapshot.clone())
        } else {
            observe(endpoint, &selected).map_err(BindingError::Endpoint)?
        };
        match evaluate_binding(&selected, &probe) {
            BindingEvidence::Active(_) => return Err(BindingError::NameAlreadyActive),
            BindingEvidence::Unknown => return Err(BindingError::Unverified),
            BindingEvidence::EndpointLost if selected.identity.lifetime == Lifetime::Temporary => {
                return Err(BindingError::Unverified);
            }
            _ => {
                records.detach_binding(&old.id)?;
            }
        }
    }
    let publish = current.is_none();
    let binding = match current {
        Some(binding) => binding,
        None => {
            // Recorded once, when the binding is made, so a reused pid
            // can later be told from the same pane. A failed observation
            // is unknown, never a failed bind.
            let mut pane = pane.clone();
            pane.pane_incarnation = match target.and_then(|target| target.pane_incarnation.clone())
            {
                Some(start) => Some(start),
                None => endpoint
                    .pane_incarnation(&snapshot.server, pane.pane_pid)
                    .unwrap_or(None),
            };
            records.insert_binding(&selected.identity, &snapshot.server, &pane)?
        }
    };
    if publish {
        endpoint
            .publish(&binding, &selected.identity)
            .map_err(BindingError::Endpoint)?;
    }
    let verified = endpoint
        .current_snapshot(&[pane_id.into()])
        .map_err(BindingError::Endpoint)?;
    target_pane(&verified, pane_id, target)?;
    let persisted = records
        .entry_by_pane(binding.server.host, pane_id, &binding.server.server_id)?
        .ok_or(BindingError::Unverified)?;
    if persisted.identity.id != selected.identity.id || persisted.binding.as_ref() != Some(&binding)
    {
        return Err(BindingError::Unverified);
    }
    let BindingEvidence::Active(pane) =
        evaluate_binding(&persisted, &EndpointProbe::Live(verified))
    else {
        return Err(BindingError::Unverified);
    };
    records.touch_binding(&binding.id)?;
    deadline(endpoint)?;
    Ok(BoundIdentity {
        presence: IdentityPresence {
            identity: selected.identity,
            presence: Presence::Active,
            pane: Some(*pane),
            binding: Some(binding),
        },
        created,
    })
}

/// Admission and creation share the binding transaction; refused guarded adds
/// never leave a separately committed identity creation or promotion.
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
    let mut effects = false;
    let result = repository.with_binding_transaction(|records| {
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
        let selected = match selector {
            GuardedIdentity::Uuid(id) => Some(
                records
                    .entry_by_id(id)?
                    .ok_or_else(|| BindingError::NameNotFound(id.into()))?,
            ),
            GuardedIdentity::Name(_) => {
                let name = name.as_ref().expect("validated name");
                if records.find_identity(name.canonical_name())?.is_some() {
                    return Err(BindingError::NameTaken(name.display_name().into()));
                }
                None
            }
        };
        let occupied = match fence.server_id.as_deref() {
            Some(server) => records.entry_by_pane(fence.host, &fence.pane, server)?,
            None => None,
        };
        if let Some(occupied) = &occupied {
            if selected
                .as_ref()
                .is_none_or(|s| s.identity.id != occupied.identity.id)
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
        if let Some(selected) = &selected
            && let Some(old) = &selected.binding
            && occupied.is_none()
        {
            let probe = observe(endpoint, selected).map_err(BindingError::Endpoint)?;
            match evaluate_binding(selected, &probe) {
                BindingEvidence::Active(_) => return Err(BindingError::NameAlreadyActive),
                BindingEvidence::Unknown => return Err(BindingError::Unverified),
                _ => {}
            }
            if old.pane_id == fence.pane && old.server.socket_path == fence.socket {
                return Err(BindingError::TargetChanged(fence.pane.clone()));
            }
        }
        admit(endpoint)?;
        deadline(endpoint)?;
        // Effects become possible only after both read-only admissions and all
        // selector/ownership refusals. Later failure reports a partial attempt.
        effects = true;
        (|| {
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
            let mut selected = match selected {
                Some(selected) => selected,
                None => BindingEntry {
                    identity: records.insert_identity(
                        name.as_ref().expect("name mode"),
                        if save {
                            Lifetime::Saved
                        } else {
                            Lifetime::Temporary
                        },
                    )?,
                    binding: None,
                },
            };
            if save && selected.identity.lifetime == Lifetime::Temporary {
                selected.identity = records.save_identity(&selected.identity)?;
            }
            bind_selected(
                records,
                endpoint,
                &fence.pane,
                Some(&target),
                selected,
                created,
            )
            .map(|bound| GuardedBoundIdentity {
                bound,
                already_bound: occupied.is_some(),
            })
        })()
    });
    result.map_err(|error| {
        if effects {
            BindingError::Partial(Box::new(error))
        } else {
            error
        }
    })
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

pub fn unbind_identity<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
) -> Result<Option<UnboundIdentity>, BindingError<R::Error, O::Error>> {
    repository.with_binding_transaction(|records| {
        endpoint.begin_coordination();
        let snapshot = endpoint
            .current_snapshot(&[pane_id.into()])
            .map_err(BindingError::Endpoint)?;
        if !snapshot.panes.iter().any(|pane| pane.id == pane_id) {
            return Err(BindingError::PaneNotFound(pane_id.into()));
        }
        let Some(entry) =
            records.entry_by_pane(snapshot.server.host, pane_id, &snapshot.server.server_id)?
        else {
            return Ok(None);
        };
        let evidence = evaluate_binding(&entry, &EndpointProbe::Live(snapshot));
        clear_observed(endpoint, &entry, &evidence)?;
        let retired = entry.identity.lifetime == Lifetime::Temporary;
        if retired {
            records.retire_identity(&entry.identity, false)?;
        } else if let Some(binding) = &entry.binding {
            records.detach_binding(&binding.id)?;
        }
        deadline(endpoint)?;
        let binding = matches!(evidence, BindingEvidence::Active(_))
            .then_some(entry.binding)
            .flatten();
        Ok(Some(UnboundIdentity {
            identity: entry.identity,
            retired,
            binding,
        }))
    })
}

fn clear_observed<R, O: BindingEndpoint>(
    endpoint: &mut O,
    entry: &BindingEntry,
    evidence: &BindingEvidence,
) -> Result<(), BindingError<R, O::Error>> {
    match (evidence, &entry.binding) {
        (BindingEvidence::Unknown, Some(_)) => Err(BindingError::Unverified),
        (BindingEvidence::Active(_), Some(binding)) => {
            // Clear under the SQL write lock to serialize replacement binds (publication-race.e2e.test.ts).
            if endpoint
                .clear(binding, &entry.identity)
                .map_err(BindingError::Endpoint)?
            {
                Ok(())
            } else {
                Err(BindingError::Unverified)
            }
        }
        // A mismatched marker belongs to neither this binding nor its removal.
        // Explicit retirement may detach the obsolete row but never erase it.
        _ => Ok(()),
    }
}

pub fn remove_identity<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    name: &str,
    force: bool,
) -> Result<BindingEntry, BindingError<R::Error, O::Error>> {
    repository.with_binding_transaction(|records| {
        let entry = selected_entry(records, name)?
            .ok_or_else(|| BindingError::NameNotFound(name.into()))?;
        if entry.identity.lifetime == Lifetime::Saved && !force {
            return Err(BindingError::ConfirmationRequired);
        }
        endpoint.begin_coordination();
        let mut cleared_binding = None;
        if entry.binding.is_some() {
            let probe = observe(endpoint, &entry).map_err(BindingError::Endpoint)?;
            let evidence = evaluate_binding(&entry, &probe);
            clear_observed(endpoint, &entry, &evidence)?;
            if matches!(evidence, BindingEvidence::Active(_)) {
                cleared_binding = entry.binding.clone();
            }
        }
        records.retire_identity(&entry.identity, true)?;
        deadline(endpoint)?;
        Ok(BindingEntry {
            identity: entry.identity,
            binding: cleared_binding,
        })
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

/// Name only the verified caller's auto-named identity. Verification, collision
/// checks, promotion and provenance consumption share one write transaction.
/// Ordinary panes return None and retain the existing bind policy.
pub fn name_auto_identity<R: BindingRepository, O: BindingEndpoint>(
    repository: &mut R,
    endpoint: &mut O,
    pane_id: &str,
    new: &str,
    save: bool,
) -> Result<Option<BoundIdentity>, BindingError<R::Error, O::Error>> {
    let name = crate::names::validate_name(new).map_err(BindingError::InvalidName)?;
    repository.with_binding_transaction(|records| {
        endpoint.begin_coordination();
        let snapshot = endpoint
            .current_snapshot(&[pane_id.into()])
            .map_err(BindingError::Endpoint)?;
        target_pane(&snapshot, pane_id, None)?;
        let Some(entry) =
            records.entry_by_pane(snapshot.server.host, pane_id, &snapshot.server.server_id)?
        else {
            return Ok(None);
        };
        if !records.is_auto_named(&entry.identity.id)? {
            return Ok(None);
        }
        let BindingEvidence::Active(pane) =
            evaluate_binding(&entry, &EndpointProbe::Live(snapshot))
        else {
            return Err(BindingError::Unverified);
        };
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
        deadline(endpoint)?;
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
    repository.with_binding_transaction(|records| {
        let Some(entry) = records.entry_by_id(&binding.identity_id)? else {
            return Ok(());
        };
        if entry.identity.lifetime != Lifetime::Temporary
            || entry.binding.as_ref() != Some(binding)
            || !records.is_auto_named(&entry.identity.id)?
        {
            return Ok(());
        }
        endpoint.begin_coordination();
        let probe = observe(endpoint, &entry).map_err(BindingError::Endpoint)?;
        clear_observed(endpoint, &entry, &evaluate_binding(&entry, &probe))?;
        records.retire_identity(&entry.identity, true)?;
        deadline(endpoint)
    })
}
