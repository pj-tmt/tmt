//! Thin composition of typed commands, one storage handle and binding policy.
//! Preflight caller/target checks precede configuration and database effects.

#[cfg(test)]
mod caller_host_tests;
mod detected;
#[cfg(test)]
mod guarded_tests;
mod presentation;
mod session;

use crate::{
    binding_error::{binding_failure, endpoint_failure},
    invocation::{Invocation, ListScope, OutputMode},
    output::{Failure, after_cleanup},
};
use std::io::{self, Write};
use tmt_adapters::{
    config::{ConfigFiles, ConfigPaths},
    host::{BindingSession, CallerEnvironment, Host, OperationOptions, PaneCosmetics, PaneRefresh},
    storage::Storage,
};
use tmt_core::{
    binding::session::{RememberedSession, RuntimeState},
    binding::{
        self, BindingEntry, BindingTargetEvidence, BoundIdentity, IdentityPresence,
        RenamedIdentity, UnboundIdentity,
    },
    endpoint::PaneObservation,
    host::ServerSelector,
    identity::Identity,
    names::is_pane_target,
    settings::PaneBadge,
};

/// One `tmt ls` row: its presence and its remembered session, if any.
struct ListedRow {
    presence: IdentityPresence,
    remembered: Option<RememberedSession>,
    /// The existing `resume` JSON projection, kept byte for byte.
    resume: Option<serde_json::Value>,
    activity: serde_json::Value,
    unbound: Option<bool>,
    learning: Option<crate::session_learning::LearningView>,
    running_driver: Option<String>,
    /// The identified driver's level, advisory like `driver`.
    driver_support: Option<tmt_core::driver::descriptor::Support>,
    /// The plain program an active binding runs instead of an agent.
    plain_program: Option<String>,
}

/// What the host did with the agent name after a committed bind or rename.
/// The binding stands either way; a failure gets the cause and the fix.
struct AgentNameOutcome {
    name: String,
    result: Result<tmt_adapters::host::AgentNaming, tmt_adapters::host::HostError>,
}

struct ResolvedPane {
    id: String,
    frozen: Option<BindingTargetEvidence>,
    /// The explicit target, whose host observes the pane.
    target: Option<String>,
    /// How the user names the pane.
    label: String,
}

enum Report {
    Bound(BoundIdentity),
    Caller {
        pane: String,
        /// How the user names the pane (`pane` on tmux, `wN:pM` on Herdr).
        label: String,
        identity: Option<Identity>,
        runtime: RuntimeState,
        unbound: Option<bool>,
    },
    Unbound {
        pane: String,
        label: String,
        result: UnboundIdentity,
    },
    Removed(BindingEntry),
    /// `pane` is set after commit when a live pane showed the identity.
    Renamed {
        result: RenamedIdentity,
        pane: Option<(String, PaneRefresh)>,
    },
    /// The rows `tmt ls` shows, after its filters, in storage order.
    /// `bound` holds every stored binding's pane, before the filters, and is
    /// read only for `--detected`; `detected` is that scan, made after
    /// storage closed.
    Listed {
        rows: Vec<ListedRow>,
        scope: ListScope,
        bound: Vec<detected::BoundPane>,
        detected: Option<detected::Detected>,
    },
    Named {
        target: String,
        row: IdentityPresence,
        remembered: Option<RememberedSession>,
        learning: Option<Box<crate::session_learning::LearningView>>,
    },
    Pane {
        target: String,
        pane: PaneObservation,
        identity: Option<Identity>,
        remembered: Option<RememberedSession>,
    },
}

fn remembered_session(
    storage: &Storage,
    identity: &Identity,
) -> Result<Option<RememberedSession>, Failure> {
    storage
        .session_preferences(&identity.id)
        .map(|preferences| preferences.remembered)
        .map_err(|error| {
            Failure::new("IDENTITY_ERROR", "Could not read remembered sessions.", 1)
                .caused_by(error)
        })
}

/// The caller's own pane with the label the user knows it by, or `missing`. These commands
/// cannot do without the pane, so a slow tmux is reported as slow, not as a missing pane
/// (`Host::caller_pane_required`, #4260).
fn labeled_caller_pane(
    host: &Host,
    environment: &CallerEnvironment,
    missing: impl FnOnce() -> Failure,
) -> Result<Option<ResolvedPane>, Failure> {
    let id = host
        .caller_pane_required(environment)
        .map_err(endpoint_failure)?
        .ok_or_else(missing)?;
    let label = host.caller_label(&id).map_err(endpoint_failure)?;
    Ok(Some(ResolvedPane {
        label,
        id,
        frozen: None,
        target: None,
    }))
}

/// `listed_pane`: whether `ls <text>` addresses a pane, decided once by
/// [`listed_target`] before any host is resolved.
fn preflight(
    request: &Invocation,
    host: &Host,
    environment: &CallerEnvironment,
    listed_pane: bool,
) -> Result<Option<ResolvedPane>, Failure> {
    let target = match request {
        Invocation::BindMarked { .. } => {
            let target = host
                .marked_pane(environment, OperationOptions::default())
                .map_err(endpoint_failure)?
                .ok_or_else(|| {
                    Failure::new(
                        "MARKED_PANE_NOT_FOUND",
                        "No marked pane was found on the selected tmux server.",
                        3,
                    )
                    .suggestion("Mark the intended pane in tmux, then retry.".into())
                })?;
            return Ok(Some(ResolvedPane {
                id: target.pane_id.clone(),
                label: target.pane_id.clone(),
                frozen: Some(target),
                target: None,
            }));
        }
        Invocation::Bind {
            pane: Some(pane),
            name,
            ..
        } => {
            if !is_pane_target(pane) && is_pane_target(name) {
                return Err(Failure::new(
                    "LEGACY_ADD_ORDER",
                    "The v4 add argument order is no longer supported.",
                    1,
                )
                .suggestion(format!("Use: tmt add {name} {pane}")));
            }
            Some(pane.as_str())
        }
        Invocation::List {
            target: Some(target),
            ..
        } if listed_pane => Some(target.as_str()),
        Invocation::List {
            target: None,
            scope: ListScope { here: true, .. },
            ..
        } => {
            // `--here` is caller-scoped like `whoami`: it needs a tmux pane.
            crate::caller_context::require_independent_host()?;
            return labeled_caller_pane(host, environment, || {
                Failure::new(
                    "PANE_NOT_FOUND",
                    "`tmt ls --here` needs a tmux pane, and this is not one.",
                    3,
                )
                .suggestion("Run it inside tmux, or drop --here to list every agent.".into())
            });
        }
        Invocation::Bind { pane: None, .. } | Invocation::Whoami | Invocation::Unbind => {
            let name = match request {
                Invocation::Bind { name, .. } => Some(name.as_str()),
                _ => None,
            };
            crate::caller_context::require_independent_host_for(name)?;
            return labeled_caller_pane(host, environment, || {
                Failure::new(
                    "PANE_NOT_FOUND",
                    "Not running inside a resolvable tmux pane.",
                    3,
                )
            });
        }
        _ => None,
    };
    target
        .map(|target| {
            Host::for_target(target)
                .resolve_target(target, OperationOptions::default())
                .map_err(endpoint_failure)?
                .ok_or_else(|| {
                    Failure::new(
                        "PANE_NOT_FOUND",
                        format!("Pane target '{target}' was not found."),
                        3,
                    )
                })
                .map(|id| ResolvedPane {
                    label: tmt_core::host::HostKind::label(&id, Some(target)).to_owned(),
                    id,
                    frozen: None,
                    target: Some(target.to_owned()),
                })
        })
        .transpose()
}

fn open_storage(paths: &ConfigPaths) -> Result<Storage, Failure> {
    Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "No identity was changed.",
            "IDENTITY_ERROR",
            "Could not open identity storage.",
        )
    })
}

/// Whether `ls <text>` addresses a pane rather than an identity, decided
/// once before any host is resolved. Only text an existing identity could
/// hold (a Herdr target) needs storage, which is then kept for the command;
/// other requests open storage only after their preflight, as before.
struct ListedTarget {
    pane: bool,
    opened: Option<(ConfigPaths, Storage)>,
}

fn listed_target(
    request: &Invocation,
    paths: &impl Fn() -> Result<ConfigPaths, Failure>,
) -> Result<ListedTarget, Failure> {
    let target = match request {
        Invocation::List {
            target: Some(target),
            ..
        } if is_pane_target(target) => target,
        _ => {
            return Ok(ListedTarget {
                pane: false,
                opened: None,
            });
        }
    };
    if tmt_core::names::validate_existing_name(target).is_err() {
        return Ok(ListedTarget {
            pane: true,
            opened: None,
        });
    }
    let paths = paths()?;
    let storage = open_storage(&paths)?;
    let pane = tmt_core::identity::addresses_pane(&storage, target).map_err(|error| {
        Failure::new("IDENTITY_ERROR", "Could not read identity storage.", 1).caused_by(error)
    })?;
    Ok(ListedTarget {
        pane,
        opened: Some((paths, storage)),
    })
}

fn run(request: Invocation) -> Result<(Report, Option<AgentNameOutcome>), Failure> {
    run_with(
        request,
        CallerEnvironment::current(),
        || ConfigPaths::discover().map_err(Failure::from),
        crate::caller_context::host,
    )
}

fn run_with(
    request: Invocation,
    environment: CallerEnvironment,
    paths: impl Fn() -> Result<ConfigPaths, Failure>,
    caller_host: impl FnOnce(&CallerEnvironment) -> Result<Host, Failure>,
) -> Result<(Report, Option<AgentNameOutcome>), Failure> {
    let listed = listed_target(&request, &paths)?;
    let caller = match &request {
        Invocation::Bind { pane: None, .. }
        | Invocation::BindMarked { .. }
        | Invocation::Whoami
        | Invocation::Unbind
        | Invocation::List {
            target: None,
            scope: ListScope { here: true, .. },
            ..
        } => caller_host(&environment)?,
        // This session observes stored bindings on their own hosts; its primary
        // host is not caller evidence. Explicit panes are selected below.
        _ => Host::for_host(
            tmt_core::host::HostKind::Tmux,
            tmt_adapters::process::UnixCommandRunner,
            tmt_adapters::host::external::approved(),
        ),
    };
    let pane = preflight(&request, &caller, &environment, listed.pane)?;
    // An explicit target's own host observes it; stored bindings are always
    // probed on theirs.
    let host = match pane.as_ref().and_then(|pane| pane.target.as_deref()) {
        Some(target) => Host::for_target(target),
        None => caller,
    };
    let (paths, storage) = match listed.opened {
        Some((paths, storage)) => (paths, Some(storage)),
        None => (paths()?, None),
    };
    let badge = if matches!(
        request,
        Invocation::Bind { .. } | Invocation::BindMarked { .. } | Invocation::Rename { .. }
    ) {
        ConfigFiles {
            paths: paths.clone(),
        }
        .load()
        .map_err(Failure::from)?
        .settings
        .pane_badge
    } else {
        PaneBadge::Off
    };
    let mut storage = match storage {
        Some(storage) => storage,
        None => open_storage(&paths)?,
    };
    // A pane observation needs its server resolved before any binding
    // transaction; listing and name operations only probe stored bindings.
    let resolved = if pane.is_some()
        || matches!(
            request,
            Invocation::Bind { .. } | Invocation::BindMarked { .. }
        ) {
        host.resolve_servers(&mut storage).map_err(endpoint_failure)
    } else {
        Ok(())
    };
    let mut endpoint = host.session();
    let operated = resolved.and_then(|()| {
        operation(
            &mut storage,
            &mut endpoint,
            request,
            pane,
            host.selected_server(&environment),
        )
    });
    let pending = operated.and_then(|mut report| {
        // Presentation follows successful durable effects, never decides
        // them. The adapter preserves user themes and changed endpoints.
        let badge = badge == PaneBadge::On;
        let update = match &report {
            Report::Bound(result) => result.presence.binding.as_ref().map(|binding| {
                (
                    binding,
                    PaneCosmetics::Bound {
                        identity: &result.presence.identity,
                        badge,
                        border_hint: true,
                    },
                )
            }),
            Report::Unbound { result, .. } => result
                .binding
                .as_ref()
                .map(|binding| (binding, PaneCosmetics::Ended)),
            Report::Removed(entry) => entry
                .binding
                .as_ref()
                .map(|binding| (binding, PaneCosmetics::Ended)),
            _ => None,
        };
        if let Some((binding, cosmetics)) = update {
            Host::for_server(&binding.server)
                .update_binding_cosmetics(binding, cosmetics)
                .map_err(cosmetic_cleanup)?;
        }
        // The rename is committed; its pane only shows it, and a stale
        // marker name never detaches, so a failure here is a warning.
        if let Report::Renamed { result, pane } = &mut report
            && result.changed()
            && let Some(binding) = &result.binding
        {
            let cosmetics = PaneCosmetics::Bound {
                identity: &result.identity,
                badge,
                border_hint: false,
            };
            let refresh = Host::for_server(&binding.server)
                .update_binding_cosmetics(binding, cosmetics)
                .map_err(cosmetic_cleanup)?;
            if refresh != PaneRefresh::Absent {
                *pane = Some((binding.pane_id.clone(), refresh));
            }
        }
        // An explicit bind or rename sets an agent name; `tmt run`, reads and
        // hooks never do. A refusal leaves the binding as it is.
        let named = match &report {
            Report::Bound(result) => result
                .presence
                .binding
                .as_ref()
                .map(|binding| (binding, &result.presence.identity)),
            Report::Renamed { result, .. } if result.changed() => result
                .binding
                .as_ref()
                .map(|binding| (binding, &result.identity)),
            _ => None,
        };
        let outcome = match named {
            Some((binding, identity)) => {
                name_bound_agent(&Host::for_server(&binding.server), binding, identity)?
            }
            None => None,
        };
        Ok((report, outcome))
    });
    after_cleanup(pending, || storage.close())
}

/// After a committed bind: gives the agent in the binding's pane the
/// identity's name, through `host`'s drivers. tmux has no agent names, so a
/// tmux binding asks nothing and its result gains no member. A refusal or an
/// unknown outcome keeps the binding; only a driver process that could not
/// be cleaned up fails the command.
fn name_bound_agent<R: tmt_adapters::process::CommandRunner>(
    host: &Host<R>,
    binding: &tmt_core::binding::Binding,
    identity: &Identity,
) -> Result<Option<AgentNameOutcome>, Failure> {
    if binding.server.host == tmt_core::host::HostKind::Tmux {
        return Ok(None);
    }
    match host.name_agent(binding, identity, None) {
        Err(error) if error.cleanup_failed() => Err(cosmetic_cleanup(error)),
        result => Ok(Some(AgentNameOutcome {
            name: identity.name.clone(),
            result,
        })),
    }
}

fn cosmetic_cleanup(error: tmt_adapters::host::HostError) -> Failure {
    Failure::new(
        "CLEANUP_ERROR",
        "Could not clean up cosmetic operation resources. Effects may already have occurred.",
        1,
    )
    .caused_by(error)
}

fn operation(
    storage: &mut Storage,
    endpoint: &mut BindingSession<'_, tmt_adapters::process::UnixCommandRunner>,
    request: Invocation,
    pane: Option<ResolvedPane>,
    current_server: Option<ServerSelector<'_>>,
) -> Result<Report, Failure> {
    match request {
        Invocation::Bind {
            name,
            save,
            pane: selector,
        } => {
            let pane = &pane.as_ref().expect("binding preflight").id;
            if selector.is_none()
                && let Some(bound) =
                    binding::name_auto_identity(storage, endpoint, pane, &name, save)
                        .map_err(binding_failure)?
            {
                return Ok(Report::Bound(bound));
            }
            binding::bind_identity_with_creation(storage, endpoint, pane, &name, save)
                .map(Report::Bound)
                .map_err(binding_failure)
        }
        Invocation::BindMarked { name, save } => {
            let pane = pane.as_ref().expect("marked binding preflight");
            binding::bind_identity_with_creation_at(
                storage,
                endpoint,
                &pane.id,
                pane.frozen.as_ref(),
                &name,
                save,
            )
            .map(Report::Bound)
            .map_err(binding_failure)
        }
        Invocation::Whoami => {
            let ResolvedPane {
                id: pane, label, ..
            } = pane.expect("caller preflight");
            let observed =
                binding::pane_presence(storage, endpoint, &pane).map_err(binding_failure)?;
            let runtime = observed
                .binding
                .as_ref()
                .and_then(|binding| endpoint.observed_runtime(binding).ok())
                .unwrap_or(RuntimeState::Unknown);
            let unbound = match observed.identity.as_ref() {
                Some(identity) => {
                    let preferences =
                        storage.session_preferences(&identity.id).map_err(|error| {
                            Failure::new("IDENTITY_ERROR", "Could not read remembered sessions.", 1)
                                .caused_by(error)
                        })?;
                    observed
                        .binding
                        .as_ref()
                        .and_then(|binding| binding.session.unbound(&preferences))
                }
                None => None,
            };
            Ok(Report::Caller {
                pane,
                label,
                identity: observed.identity,
                runtime,
                unbound,
            })
        }
        Invocation::Unbind => {
            let ResolvedPane {
                id: pane, label, ..
            } = pane.expect("caller preflight");
            let result = binding::unbind_identity(storage, endpoint, &pane)
                .map_err(binding_failure)?
                .ok_or_else(|| {
                    Failure::new("UNBOUND_PANE", "Pane has no active global name.", 1)
                })?;
            Ok(Report::Unbound {
                pane,
                label,
                result,
            })
        }
        Invocation::Remove { name, force } => {
            tmt_adapters::verified_stop::remove_by_name(storage, endpoint, &name, force)
                .map(Report::Removed)
                .map_err(binding_failure)
        }
        Invocation::Rename { old, new } => binding::rename_identity(storage, &old, &new)
            .map(|result| Report::Renamed { result, pane: None })
            .map_err(binding_failure),
        Invocation::List {
            target: None,
            room,
            scope,
        } => {
            // Resolve scope first: a typo must not trigger global reconciliation.
            let room = room
                .map(|selector| crate::room_command::resolve(storage, &selector))
                .transpose()?;
            let mut rows = binding::list_presence(storage, endpoint, current_server)
                .map_err(binding_failure)?;
            // From storage, not the rows: an Unknown row keeps its durable
            // binding but shows none, and the filters below drop rows.
            let bound = match scope.detected {
                Some(_) => bound_panes(storage)?,
                None => Vec::new(),
            };
            if let Some(room) = room {
                rows.retain(|row| room.member_ids.contains(&row.identity.id));
            }
            if let Some(lifetime) = scope.lifetime {
                rows.retain(|row| row.identity.lifetime == lifetime);
            }
            if let Some(caller) = &pane {
                // The caller's session is the part of its pane target before ':'.
                let observed = binding::pane_presence(storage, endpoint, &caller.id)
                    .map_err(binding_failure)?;
                let session = |pane: &PaneObservation| {
                    pane.target
                        .as_deref()
                        .and_then(|target| target.split_once(':'))
                        .map(|(session, _)| session.to_owned())
                };
                let here = session(&observed.pane);
                rows.retain(|row| here.is_some() && row.pane.as_ref().and_then(session) == here);
            }
            let registry = tmt_adapters::runtime::RuntimeRegistry::first_party();
            let rows = rows
                .into_iter()
                .map(|row| {
                    let preferences =
                        storage
                            .session_preferences(&row.identity.id)
                            .map_err(|error| {
                                Failure::new(
                                    "IDENTITY_ERROR",
                                    "Could not read remembered sessions.",
                                    1,
                                )
                                .caused_by(error)
                            })?;
                    let learning = crate::session_learning::view(
                        storage,
                        row.binding.as_ref(),
                        &row.identity,
                        &preferences,
                    )
                    .map_err(|error| {
                        Failure::new(
                            "IDENTITY_ERROR",
                            "Could not read session learning diagnostics.",
                            1,
                        )
                        .caused_by(error)
                    })?;
                    let unbound = row
                        .binding
                        .as_ref()
                        .and_then(|b| b.session.unbound(&preferences));
                    let unbound = if learning.as_ref().is_some_and(|l| l.unbound) {
                        Some(true)
                    } else {
                        unbound
                    };
                    let sampled =
                        storage
                            .sampled_rate_account(&row.identity.id)
                            .map_err(|error| {
                                Failure::new(
                                    "IDENTITY_ERROR",
                                    "Could not read remembered sessions.",
                                    1,
                                )
                                .caused_by(error)
                            })?;
                    let resume =
                        crate::output::resume_document(&preferences, &registry, sampled.as_ref());
                    let runtime = row
                        .binding
                        .as_ref()
                        .filter(|_| row.presence == tmt_core::binding::Presence::Active)
                        .and_then(|binding| {
                            // Activity ended requires a process fact, not a stored SessionEnd hook.
                            let mut observed = binding.clone();
                            if observed.session.state == RuntimeState::Ended {
                                observed.session.state = RuntimeState::Unknown;
                            }
                            endpoint.observed_runtime(&observed).ok()
                        })
                        .unwrap_or(RuntimeState::Unknown);
                    let activity = crate::output::activity_document(
                        row.binding.as_ref(),
                        preferences.remembered.as_ref(),
                        runtime,
                        &registry,
                    );
                    let running_driver = presentation::running_driver(
                        &row,
                        preferences.remembered.as_ref(),
                        runtime,
                        &registry,
                    );
                    let driver_support = presentation::driver_support(
                        &row,
                        preferences.remembered.as_ref(),
                        preferences.preferred_harness.as_ref(),
                    );
                    let plain_program = row
                        .binding
                        .as_ref()
                        .filter(|_| row.presence == tmt_core::binding::Presence::Active)
                        .and_then(|binding| binding.session.plain_program.clone());
                    Ok(ListedRow {
                        plain_program,
                        presence: row,
                        remembered: preferences.remembered,
                        resume,
                        activity,
                        unbound,
                        learning,
                        running_driver,
                        driver_support,
                    })
                })
                .collect::<Result<Vec<_>, Failure>>()?;
            Ok(Report::Listed {
                rows,
                scope,
                bound,
                detected: None,
            })
        }
        Invocation::List {
            target: Some(target),
            ..
        } => {
            if let Some(pane) = pane {
                let observed =
                    binding::pane_presence(storage, endpoint, &pane.id).map_err(binding_failure)?;
                let remembered = match &observed.identity {
                    Some(identity) => remembered_session(storage, identity)?,
                    None => None,
                };
                Ok(Report::Pane {
                    target,
                    pane: observed.pane,
                    identity: observed.identity,
                    remembered,
                })
            } else {
                let row =
                    binding::name_presence(storage, endpoint, &target).map_err(binding_failure)?;
                let remembered = remembered_session(storage, &row.identity)?;
                let learning = crate::session_learning::identity_view(
                    storage,
                    &row.identity,
                    &storage
                        .session_preferences(&row.identity.id)
                        .map_err(|error| {
                            Failure::new("IDENTITY_ERROR", "Could not read session preferences.", 1)
                                .caused_by(error)
                        })?,
                )
                .map_err(|error| {
                    Failure::new(
                        "IDENTITY_ERROR",
                        "Could not read session learning diagnostics.",
                        1,
                    )
                    .caused_by(error)
                })?;
                Ok(Report::Named {
                    target,
                    row,
                    remembered,
                    learning: learning.map(Box::new),
                })
            }
        }
        _ => Err(Failure::new(
            "INTERNAL_ERROR",
            "Unexpected binding command.",
            1,
        )),
    }
}

/// The pane of every retained binding of a non-retired identity.
fn bound_panes(storage: &mut Storage) -> Result<Vec<detected::BoundPane>, Failure> {
    use tmt_core::binding::BindingRepository;
    let entries = storage
        .with_binding_transaction(|records| records.binding_entries())
        .map_err(|error| {
            Failure::new("IDENTITY_ERROR", "Could not read identity bindings.", 1).caused_by(error)
        })?;
    Ok(entries
        .into_iter()
        .filter_map(|entry| entry.binding)
        .map(|binding| {
            (
                binding.server.host,
                binding.server.socket_path,
                binding.pane_id,
            )
        })
        .collect())
}

pub fn execute(request: Invocation, mode: OutputMode) -> io::Result<u8> {
    let (mut report, agent_name) = match run(request) {
        Ok(ran) => ran,
        Err(error) => return error.publish(mode),
    };
    // Only a listing that succeeded and closed storage scans; the scan's
    // doubt is data, and only an unreaped child fails the command.
    if let Report::Listed {
        scope: ListScope {
            detected: Some(scope),
            ..
        },
        bound,
        detected,
        ..
    } = &mut report
    {
        *detected = Some(detected::scan(scope, bound));
    }
    let completion = match &report {
        Report::Bound(result) => Some(session::complete(result)),
        _ => None,
    };
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        let mut document = presentation::document(&report);
        if let Some(outcome) = &agent_name {
            presentation::add_agent_name(&mut document, outcome);
        }
        if let Some(completion) = &completion {
            document["session"] = serde_json::json!({"unbound":completion.unbound});
            if let Some(warning) = &completion.warning {
                let warnings = document
                    .as_object_mut()
                    .expect("binding document")
                    .entry("warnings")
                    .or_insert_with(|| serde_json::json!([]));
                warnings
                    .as_array_mut()
                    .expect("warning array")
                    .push(serde_json::json!({
                    "code":"SESSION_NOT_RECORDED", "reason":warning.reason, "fix":warning.fix}));
            }
        }
        writeln!(stdout, "{document}")?;
    } else {
        let terminal = stdout.terminal();
        presentation::text(&mut stdout, terminal, &report)?;
    }
    drop(stdout);
    if !mode.json {
        let mut stderr = tmt_cli_style::stream::stderr();
        let terminal = stderr.terminal();
        write_partial(&mut stderr, terminal, &report)?;
        presentation::warnings(&mut stderr, &report)?;
        if let Some(warning) = completion.as_ref().and_then(|value| value.warning.as_ref()) {
            let terminal = stderr.terminal();
            tmt_cli_style::message::warning(&mut stderr, terminal, &warning.reason, None)?;
            tmt_cli_style::message::fix(&mut stderr, terminal, &warning.fix)?;
        }
        if let Some(AgentNameOutcome {
            name,
            result: Err(error),
        }) = &agent_name
        {
            presentation::agent_name_warning(&mut stderr, name, error)?;
        }
    }
    use crate::skill_reminder::Outcome;
    let outcome = match &report {
        Report::Bound(result) if result.created => {
            if result.presence.identity.lifetime == tmt_core::identity::Lifetime::Temporary {
                Outcome::TemporaryIdentityCreated {
                    name: result.presence.identity.name.clone(),
                }
            } else {
                Outcome::SavedIdentityCreated {
                    name: result.presence.identity.name.clone(),
                }
            }
        }
        _ => Outcome::None,
    };
    crate::skill_reminder::present(outcome, mode, true);
    let changed_binding = match &report {
        Report::Bound(result) => result.presence.binding.as_ref(),
        Report::Unbound { result, .. } => result.binding.as_ref(),
        Report::Removed(entry) => entry.binding.as_ref(),
        Report::Renamed { result, .. } if result.changed() => result.binding.as_ref(),
        _ => None,
    };
    // A member removed on purpose leaves the default built-in restore, bound or not.
    // Advisory: the removal already happened and keeps its result.
    if let Report::Removed(entry) = &report
        && let Ok(paths) = ConfigPaths::discover()
    {
        let _ = tmt_adapters::workspace::forget_builtin_member(&paths, &entry.identity.id);
    }
    if let Some(binding) = changed_binding
        && let Ok(paths) = ConfigPaths::discover()
    {
        let host = Host::for_server(&binding.server);
        tmt_adapters::workspace::refresh_server(&paths, &host, &binding.server);
    }
    Ok(status(&report))
}

/// The one error of a partial list, whose scan child was not reaped.
fn partial(report: &Report) -> Option<Failure> {
    match report {
        Report::Listed {
            detected: Some(detected),
            ..
        } => detected.failure(),
        _ => None,
    }
}

/// The exit status after the output: the partial list's, else 0.
fn status(report: &Report) -> u8 {
    partial(report).map_or(0, |failure| failure.status)
}

/// In text mode, a partial list's error on stderr; the JSON document
/// carries it instead.
fn write_partial(
    output: &mut impl Write,
    terminal: tmt_cli_style::Terminal,
    report: &Report,
) -> io::Result<()> {
    match partial(report) {
        Some(failure) => failure.write_human(output, terminal),
        None => Ok(()),
    }
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core("Use: tmt add {name} {pane}", &[""], &[]),
    crate::cli_style_tests::HintSpec::core(
        "`tmt ls --here` needs a tmux pane, and this is not one.",
        &["`"],
        &[],
    ),
];

#[cfg(test)]
pub(crate) use presentation::PRINTED_HINTS as PRESENTATION_HINTS;
#[cfg(test)]
pub(crate) use session::PRINTED_HINTS as SESSION_HINTS;

/// A pane ID no admitted host reads: tmux's, or an approved driver's
/// declared syntax. A host whose approval is a gap registers none.
fn unknown_pane_syntax(admitted: &tmt_adapters::driver_protocol::registry::Admitted) -> Failure {
    let mut message =
        String::from("Guarded add requires an exact pane ID of tmux or an approved host driver.");
    for gap in &admitted.gaps {
        message.push_str(&format!(
            " The {} driver is not usable: {}.",
            gap.name, gap.reason
        ));
    }
    Failure::new("USAGE_ERROR", message, 1)
}

/// A changed or unverifiable discovered target is drift; anything else is
/// the endpoint's own failure.
fn guarded_failure(error: tmt_adapters::host::HostError) -> Failure {
    match error {
        error @ tmt_adapters::host::HostError::Evidence { .. } => {
            Failure::new("PANE_CHANGED", error.to_string(), 3).caused_by(error)
        }
        error => endpoint_failure(error),
    }
}

/// The guarded add after parsing, on `host`, with the drivers the command
/// read once. Admission 1 reads only; an external host's server is then
/// recorded for the fenced incarnation alone, before the binding
/// transaction admits the target twice more. `partial` turns true once host
/// effects may have happened.
fn run_guarded<R: tmt_adapters::process::CommandRunner + Clone>(
    runner: R,
    host: tmt_core::host::HostKind,
    admitted: &tmt_adapters::driver_protocol::registry::Admitted,
    paths: &ConfigPaths,
    request: &GuardedRequest<'_>,
    partial: &mut bool,
) -> Result<GuardedOutcome, Failure> {
    use std::time::{Duration, Instant};
    use tmt_core::binding::{BindingError, GuardedIdentity};
    let host = match host {
        tmt_core::host::HostKind::Tmux => {
            Host::for_caller_with(&CallerEnvironment::admitted(admitted), runner)
                .map_err(crate::caller_context::selection_failed)?
        }
        external => Host::for_host(external, runner, admitted.records.clone()),
    };
    let candidate = tmt_adapters::discovery::guarded_candidate(
        &host,
        request.socket,
        request.pane,
        &paths.database,
        request.expect,
        Instant::now() + Duration::from_secs(2),
    )
    .map_err(guarded_failure)?;
    let mut fence = candidate.fence.expect("admitted native fence");
    let mut storage = open_storage(paths)?;
    let result = (|| {
        if let Some(server_id) = host
            .resolve_fenced_server(&fence, &mut storage)
            .map_err(guarded_failure)?
        {
            fence.server_id = Some(server_id);
        }
        let mut endpoint = host.session().with_discovery_target(fence.clone());
        let selector = match request.identity {
            Some(id) => GuardedIdentity::Uuid(id),
            None => GuardedIdentity::Name(request.name.expect("parser selected name")),
        };
        let bound =
            binding::bind_guarded(&mut storage, &mut endpoint, &fence, selector, request.save)
                .map_err(|error| match error {
                    BindingError::Partial(error) => {
                        *partial = true;
                        binding_failure(*error)
                    }
                    error => binding_failure(error),
                })?;
        // A naming or close failure follows admitted effects.
        *partial = true;
        let presence = &bound.bound.presence;
        let agent_name = match &presence.binding {
            Some(binding) => name_bound_agent(&host, binding, &presence.identity)?,
            None => None,
        };
        Ok((bound, agent_name))
    })();
    after_cleanup(result, || storage.close())
}

/// A guarded add's one result: its document, or its failure with `partial`
/// once host effects may have happened.
fn present_guarded(
    output: &mut impl Write,
    mode: OutputMode,
    pane: &str,
    outcome: Result<GuardedOutcome, Failure>,
    partial: bool,
) -> io::Result<u8> {
    match outcome {
        Ok((result, agent_name)) => {
            let row = &result.bound.presence;
            let pane = &row.pane.as_ref().expect("verified binding").id;
            if mode.json {
                let mut document = serde_json::json!({"version":1,"status":if result.already_bound {"already_bound"}else{"bound"},"bound":true,"id":row.identity.id,"name":row.identity.name,"pane":pane,"lifetime":row.identity.lifetime.as_str()});
                if let Some(outcome) = &agent_name {
                    presentation::add_agent_name(&mut document, outcome);
                }
                writeln!(output, "{document}")?;
            } else {
                writeln!(output, "Bound {:?} on {}", row.identity.name, pane)?;
                if let Some(AgentNameOutcome {
                    name,
                    result: Err(error),
                }) = &agent_name
                {
                    let mut stderr = tmt_cli_style::stream::stderr();
                    presentation::agent_name_warning(&mut stderr, name, error)?;
                }
            }
            Ok(0)
        }
        Err(error) => {
            if mode.json {
                let mut document = error.document();
                document["status"] = if partial { "partial" } else { "refused" }.into();
                document["pane"] = pane.into();
                writeln!(output, "{document}")?;
                Ok(error.status)
            } else {
                error.publish(mode)
            }
        }
    }
}

/// A committed guarded bind and what the host did with the agent name.
type GuardedOutcome = (
    tmt_core::binding::GuardedBoundIdentity,
    Option<AgentNameOutcome>,
);

/// One guarded add, as parsed.
struct GuardedRequest<'a> {
    pane: &'a str,
    name: Option<&'a str>,
    identity: Option<&'a str>,
    expect: &'a str,
    socket: Option<&'a str>,
    save: bool,
}

/// Guarded add bypasses optional learning/capture on refusal. It shares the
/// existing binding/publication policy, with admission before storage creation.
pub fn execute_guarded(request: Invocation, mode: OutputMode) -> io::Result<u8> {
    let Invocation::GuardedBind {
        pane,
        name,
        identity,
        expect,
        socket,
        save,
    } = request
    else {
        unreachable!("guarded add")
    };
    let mut partial = false;
    let outcome: Result<_, Failure> = (|| {
        // One read of the approvals: guarded add never adopts or rewrites
        // one, and only their syntax is registered, before name admission.
        let paths = ConfigPaths::discover();
        let admitted = tmt_adapters::host::external::admitted_at(paths.as_ref());
        tmt_adapters::host::external::register(&admitted.records);
        if let Some(name) = &name {
            tmt_core::names::validate_name(name)
                .map_err(|error| Failure::new("USAGE_ERROR", error.to_string(), 1))?;
        }
        let host = tmt_core::host::HostKind::of_pane_id(&pane)
            .ok_or_else(|| unknown_pane_syntax(&admitted))?;
        let paths = paths.map_err(Failure::from)?;
        let request = GuardedRequest {
            pane: &pane,
            name: name.as_deref(),
            identity: identity.as_deref(),
            expect: &expect,
            socket: socket.as_deref(),
            save,
        };
        run_guarded(
            tmt_adapters::process::UnixCommandRunner,
            host,
            &admitted,
            &paths,
            &request,
            &mut partial,
        )
    })();
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    present_guarded(&mut output, mode, &pane, outcome, partial)
}
