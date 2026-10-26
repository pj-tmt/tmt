//! Signed policy intent uses the current M1 writers inside the management effect transaction.
use super::{
    Change, Member,
    service::{self, Caller},
    store as sql,
};
use crate::{canonical, error::RemoteError, store::Grant};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Target {
    pub remote_id: String,
    pub member_id: String,
    pub member_revision: u64,
}
#[derive(Clone, Debug)]
pub(crate) enum Intent {
    Rename { target: Target, name: String },
    Sending { target: Target, enabled: bool },
    Revoke { target: Target },
}
impl Intent {
    pub fn target(&self) -> &Target {
        match self {
            Self::Rename { target, .. }
            | Self::Sending { target, .. }
            | Self::Revoke { target } => target,
        }
    }
    /// Authorization precedes intent adoption and repeats under the effect fence.
    /// Target revisions are checked only at effect time so original replay remains read-only.
    pub fn authorize(&self, c: &Connection, grant: &Grant, now: u64) -> Result<(), RemoteError> {
        self.authorize_for(c, Caller::Browser(grant), now)
    }
    fn authorize_for(
        &self,
        c: &Connection,
        caller: Caller<'_>,
        now: u64,
    ) -> Result<(), RemoteError> {
        let scope = service::scope(c, caller, now)?;
        if self.target().remote_id != scope.remote {
            return Err(service::denied());
        }
        match self {
            Self::Rename { target, .. } => scope.target(&target.member_id),
            _ if scope.administrator() => Ok(()),
            _ => Err(service::denied()),
        }
    }
    pub fn prepare<'a>(
        &'a self,
        c: &'a Connection,
        grant: &Grant,
        now: u64,
    ) -> Result<PreparedMemberPolicy<'a>, RemoteError> {
        self.prepare_for(c, Caller::Browser(grant), now)
    }
    pub(crate) fn prepare_for<'a>(
        &'a self,
        c: &'a Connection,
        caller: Caller<'_>,
        now: u64,
    ) -> Result<PreparedMemberPolicy<'a>, RemoteError> {
        self.authorize_for(c, caller, now)?;
        let target = self.target();
        let member = sql::check_member(c, &target.member_id, target.member_revision)?;
        Ok(PreparedMemberPolicy {
            connection: c,
            intent: self,
            member,
        })
    }
}
/// Created only by validation while the caller retains the same Store writer
/// transaction. An error after this boundary must abort that transaction.
pub(crate) struct PreparedMemberPolicy<'a> {
    connection: &'a Connection,
    intent: &'a Intent,
    member: Member,
}
impl PreparedMemberPolicy<'_> {
    pub fn apply(self, now: u64) -> Result<Change, RemoteError> {
        match self.intent {
            Intent::Rename { name, .. } => sql::rename_in(self.connection, self.member, name),
            Intent::Sending { enabled, .. } => {
                sql::sending_in(self.connection, self.member, *enabled, now)
            }
            Intent::Revoke { .. } => sql::revoke_in(self.connection, self.member, now),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    operation_id: String,
    remote_id: String,
    member_id: String,
    member_revision: u64,
    name: Option<String>,
    enabled: Option<bool>,
}
pub(crate) fn parse(operation: &str, value: &Value) -> Result<(String, Intent), RemoteError> {
    let mut keys = vec!["operationId", "remoteId", "memberId", "memberRevision"];
    match operation {
        "remote.members.rename" => keys.push("name"),
        "remote.members.sending" => keys.push("enabled"),
        "remote.members.revoke" => {}
        _ => return Err(service::invalid()),
    }
    let object = value.as_object().ok_or_else(service::invalid)?;
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err(service::invalid());
    }
    let input: Input = serde_json::from_value(value.clone()).map_err(|_| service::invalid())?;
    if !canonical::is_core_id(&input.remote_id)
        || input.member_id.is_empty()
        || input.member_revision == 0
        || input.member_revision > 9_007_199_254_740_991
    {
        return Err(service::invalid());
    }
    let target = Target {
        remote_id: input.remote_id,
        member_id: input.member_id,
        member_revision: input.member_revision,
    };
    let intent = match operation {
        "remote.members.rename" if input.enabled.is_none() => Intent::Rename {
            target,
            name: input
                .name
                .filter(|name| canonical::device_name(name))
                .ok_or_else(service::invalid)?,
        },
        "remote.members.sending" if input.name.is_none() => Intent::Sending {
            target,
            enabled: input.enabled.ok_or_else(service::invalid)?,
        },
        "remote.members.revoke" if input.name.is_none() && input.enabled.is_none() => {
            Intent::Revoke { target }
        }
        _ => return Err(service::invalid()),
    };
    Ok((input.operation_id, intent))
}
