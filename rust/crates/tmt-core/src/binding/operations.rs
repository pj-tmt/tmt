use super::observation::{named_entry, observe, reconcile};
use super::*;
use crate::identity::{self, IdentityError, Lifetime};

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
    endpoint.begin_coordination();
    let preflight = endpoint
        .current_snapshot(&[pane_id.into()])
        .map_err(BindingError::Endpoint)?;
    target_pane(&preflight, pane_id, target)?;
    // Reconcile only the requested old name before creation. Conclusively lost
    // temporary identities release their name; create_or_resolve then allocates
    // a new UUID instead of resurrecting that old identity's history.
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
    let created = identity::create_or_resolve(
        repository,
        name,
        if save {
            Lifetime::Saved
        } else {
            Lifetime::Temporary
        },
    )
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
                BindingEvidence::EndpointLost
                    if selected.identity.lifetime == Lifetime::Temporary =>
                {
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
            None => records.insert_binding(&selected.identity, &snapshot.server, pane)?,
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
        if persisted.identity.id != selected.identity.id
            || persisted.binding.as_ref() != Some(&binding)
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
            created: created.created,
        })
    });
    bound.map_err(|error| abandon_creation(repository, &created, error))
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
            if endpoint.clear(binding).map_err(BindingError::Endpoint)? {
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
        let entry =
            named_entry(records, name)?.ok_or_else(|| BindingError::NameNotFound(name.into()))?;
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
            named_entry(records, old)?.ok_or_else(|| BindingError::NameNotFound(old.into()))?;
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
        let identity = records.rename_identity(&entry.identity, &name)?;
        Ok(RenamedIdentity {
            previous: entry.identity,
            identity,
            binding: entry.binding,
        })
    })
}
