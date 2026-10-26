//! Typed owner-terminal reads share the signed projection and serving Store owner.
use super::{observations::MemberObservationSource, selectors, service};
use crate::{devices::Devices, error::RemoteError, store::database};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "view",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MemberTerminalRead {
    Roster {
        cursor: Option<String>,
        limit: usize,
    },
    Machines {
        reference: String,
        cursor: Option<String>,
        limit: usize,
    },
    Resolve {
        reference: String,
    },
    ResolveMachine {
        client_id: String,
    },
    Designation,
}
/// Only owner-local adapters invoke this; no browser operation accepts this request type.
pub fn read(
    devices: &Devices,
    source: &dyn MemberObservationSource,
    request: &MemberTerminalRead,
) -> Result<Value, RemoteError> {
    match request {
        MemberTerminalRead::Roster { cursor, limit } => super::reads::local(
            devices,
            source,
            "remote.members.list",
            &json!({"cursor":cursor,"limit":limit}),
        ),
        MemberTerminalRead::Machines {
            reference,
            cursor,
            limit,
        } => {
            let person = {
                let store = devices.store()?;
                selectors::local_member(&store, reference)?
            };
            super::reads::local(
                devices,
                source,
                "remote.members.machines",
                &json!({"memberId":person.member_id,"cursor":cursor,"limit":limit}),
            )
        }
        MemberTerminalRead::ResolveMachine { client_id } => {
            let store = devices.store()?;
            let target = super::terminal_machines::resolve(&store, client_id)?;
            Ok(json!({"target":target}))
        }
        MemberTerminalRead::Designation => {
            let store = devices.store()?;
            let current = crate::management::designation_in(&store.connection)?;
            let target = current
                .map(|g| super::terminal_machines::resolve(&store, &g.client_id))
                .transpose()?;
            Ok(json!({"target":target}))
        }
        MemberTerminalRead::Resolve { reference } => {
            let person = {
                let store = devices.store()?;
                selectors::local_member(&store, reference)?
            };
            Ok(
                json!({"target":{"remoteId":person.remote_id,"memberId":person.member_id,"memberRevision":person.revision},"member":serde_json::to_value(service::Person::from(person)).map_err(database)?}),
            )
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemberTerminalTarget {
    pub remote_id: String,
    pub member_id: String,
    pub member_revision: u64,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum MemberTerminalMutation {
    Rename {
        target: MemberTerminalTarget,
        name: String,
    },
    Sending {
        target: MemberTerminalTarget,
        enabled: bool,
    },
    Revoke {
        target: MemberTerminalTarget,
    },
}
impl MemberTerminalMutation {
    fn intent(&self) -> super::policy::Intent {
        let target = match self {
            Self::Rename { target, .. }
            | Self::Sending { target, .. }
            | Self::Revoke { target } => target,
        };
        let target = super::policy::Target {
            remote_id: target.remote_id.clone(),
            member_id: target.member_id.clone(),
            member_revision: target.member_revision,
        };
        match self {
            Self::Rename { name, .. } => super::policy::Intent::Rename {
                target,
                name: name.clone(),
            },
            Self::Sending { enabled, .. } => super::policy::Intent::Sending {
                target,
                enabled: *enabled,
            },
            Self::Revoke { .. } => super::policy::Intent::Revoke { target },
        }
    }
}
impl crate::store::Store {
    /// One terminal effect, with no adopted ID or receipt. The caller retains the
    /// Store owner; session cleanup happens only after this transaction commits.
    pub(crate) fn member_terminal_effect(
        &mut self,
        request: &MemberTerminalMutation,
    ) -> Result<(Value, Vec<String>), RemoteError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database)?;
        let now = crate::pairing::now_ms()?;
        let intent = request.intent();
        let target = intent.target();
        let scope = service::scope(&tx, service::Caller::LocalOwner, now)?;
        if target.remote_id != scope.remote {
            return Err(service::denied());
        }
        let current = super::store::member_in(&tx, &scope.remote, &target.member_id)?
            .filter(|m| m.revision == target.member_revision)
            .ok_or_else(|| {
                RemoteError::new(
                    "REMOTE_MEMBER_CONFLICT",
                    "Member changed. Read tmt remote members and confirm again.",
                )
            })?;
        if let MemberTerminalMutation::Rename { name, .. } = request
            && !crate::canonical::device_name(name)
        {
            return Err(service::invalid());
        }
        let already_revoked = current.revoked_at_ms.is_some();
        let unchanged = match request {
            MemberTerminalMutation::Revoke { .. } => already_revoked,
            MemberTerminalMutation::Rename { name, .. } => {
                !already_revoked && current.display_name == *name
            }
            MemberTerminalMutation::Sending { enabled, .. } => {
                !already_revoked && current.sending_enabled == *enabled
            }
        };
        let change = if unchanged {
            super::Change {
                member: current.clone(),
                clients: Vec::new(),
            }
        } else {
            intent
                .prepare_for(&tx, service::Caller::LocalOwner, now)?
                .apply(now)?
        };
        let changed = change.member != current || !change.clients.is_empty();
        tx.commit().map_err(|_| RemoteError::new("REMOTE_MEMBER_UNCONFIRMED", "Could not confirm the member change. Read tmt remote members before running it again."))?;
        Ok((
            json!({"state":"committed","changed":changed,"alreadyRevoked":already_revoked,"member":service::Person::from(change.member)}),
            change.clients,
        ))
    }
}
/// Acknowledgment follows the SQL commit and release of the Store guard.
pub fn mutate(devices: &Devices, request: &MemberTerminalMutation) -> Result<Value, RemoteError> {
    let (answer, clients) = {
        let mut store =
            devices.terminal_store(std::time::Instant::now() + crate::limits::AUTHORITY_WAIT)?;
        store.member_terminal_effect(request)?
    };
    for client in &clients {
        devices.committed_change(client);
    }
    if answer["changed"] == true {
        devices.wake();
    }
    Ok(answer)
}
