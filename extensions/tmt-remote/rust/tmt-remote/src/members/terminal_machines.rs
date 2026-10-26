//! Exact-machine terminal effects reuse the grant/designation writers, never person selection.
use super::terminal::MemberTerminalTarget;
use crate::{
    devices::Devices,
    error::RemoteError,
    management,
    store::{Grant, Store, database, grant_in, machine_in},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
type Result<T> = std::result::Result<T, RemoteError>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemberMachineTarget {
    pub member: MemberTerminalTarget,
    pub client_id: String,
    pub grant_revision: u64,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum MemberMachineMutation {
    Rename {
        target: MemberMachineTarget,
        name: String,
    },
    Remove {
        target: MemberMachineTarget,
    },
    Designate {
        target: MemberMachineTarget,
    },
    Undesignate {
        expected: Option<MemberMachineTarget>,
    },
}
fn conflict() -> RemoteError {
    RemoteError::new(
        "REMOTE_MEMBER_CONFLICT",
        "Member, machine or designation changed. Read tmt remote members and confirm again.",
    )
}
pub fn resolve(store: &Store, client: &str) -> Result<MemberMachineTarget> {
    let (member, grant) = super::selectors::local_machine(store, client)?;
    Ok(MemberMachineTarget {
        member: MemberTerminalTarget {
            remote_id: member.remote_id,
            member_id: member.member_id,
            member_revision: member.revision,
        },
        client_id: grant.client_id,
        grant_revision: grant.revision,
    })
}
fn checked(c: &rusqlite::Connection, target: &MemberMachineTarget) -> Result<Grant> {
    let member = super::member_for_client_in(c, &target.client_id)?.ok_or_else(conflict)?;
    let grant = grant_in(c, &target.client_id)?.ok_or_else(conflict)?;
    if machine_in(c)?.id != target.member.remote_id
        || member.remote_id != target.member.remote_id
        || member.member_id != target.member.member_id
        || member.revision != target.member.member_revision
        || grant.revision != target.grant_revision
    {
        return Err(conflict());
    }
    Ok(grant)
}
impl Store {
    pub(crate) fn terminal_machine_effect(
        &mut self,
        request: &MemberMachineMutation,
        origin: Option<&str>,
    ) -> Result<(Value, Vec<String>)> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database)?;
        let now = crate::pairing::now_ms()?;
        let mut ended = Vec::new();
        let (answer, changed) = match request {
            MemberMachineMutation::Rename { target, name } => {
                let before = checked(&tx, target)?;
                let after =
                    crate::store::rename_in(&tx, &target.client_id, name)?.ok_or_else(conflict)?;
                (
                    json!({"machine":{"clientId":after.client_id,"name":after.name,"revision":after.revision,"revoked":after.disabled}}),
                    before.name != after.name,
                )
            }
            MemberMachineMutation::Remove { target } => {
                let before = checked(&tx, target)?;
                let after = if before.disabled {
                    before.clone()
                } else {
                    crate::store::revoke_in(&tx, &target.client_id)?.ok_or_else(conflict)?
                };
                if !before.disabled {
                    ended.push(target.client_id.clone());
                }
                (
                    json!({"machine":{"clientId":after.client_id,"name":after.name,"revision":after.revision,"revoked":after.disabled}}),
                    !before.disabled,
                )
            }
            MemberMachineMutation::Designate { target } => {
                let before = checked(&tx, target)?;
                let origin = origin.ok_or_else(|| {
                    RemoteError::new(
                        "REMOTE_NOT_RUNNING",
                        "No remembered door origin. Run tmt remote serve first.",
                    )
                })?;
                let current = management::designation_in(&tx)?;
                // The writer validates browser kind, exact trusted door origin and expiry even on a no-op.
                if before.kind != "browser" || before.origin != origin || !before.live_at(now) {
                    return Err(crate::operations::invalid());
                }
                let changed = current.as_ref().is_none_or(|g| {
                    g.client_id != before.client_id || g.public_key != before.public_key
                });
                if changed {
                    management::designate_in(&tx, &before.client_id, origin, now)?;
                }
                (json!({"designatedClientId":before.client_id}), changed)
            }
            MemberMachineMutation::Undesignate { expected } => {
                let current = management::designation_in(&tx)?;
                match (expected, current) {
                    (Some(target), Some(current)) => {
                        let grant = checked(&tx, target)?;
                        if grant.client_id != current.client_id
                            || grant.public_key != current.public_key
                        {
                            return Err(conflict());
                        }
                    }
                    (None, None) => {}
                    _ => return Err(conflict()),
                }
                let changed = management::undesignate_in(&tx)? != 0;
                (json!({"designatedClientId":null}), changed)
            }
        };
        tx.commit().map_err(|_|RemoteError::new("REMOTE_MEMBER_UNCONFIRMED","Could not confirm the machine change. Read tmt remote members before running it again."))?;
        let mut answer = answer;
        answer["state"] = json!("committed");
        answer["changed"] = json!(changed);
        Ok((answer, ended))
    }
}
pub fn mutate(
    devices: &Devices,
    request: &MemberMachineMutation,
    origin: Option<&str>,
) -> Result<Value> {
    let (answer, ended) = {
        let mut store =
            devices.terminal_store(std::time::Instant::now() + crate::limits::AUTHORITY_WAIT)?;
        let remembered =
            if origin.is_none() && matches!(request, MemberMachineMutation::Designate { .. }) {
                store
                    .remembered_port()?
                    .map(|port| format!("http://127.0.0.1:{port}"))
            } else {
                None
            };
        store.terminal_machine_effect(request, origin.or(remembered.as_deref()))?
    };
    for client in &ended {
        devices.committed_change(client);
    }
    if answer["changed"] == true {
        devices.wake();
    }
    Ok(answer)
}
