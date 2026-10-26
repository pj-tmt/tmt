//! Owner-terminal selectors freeze person identity without choosing one of its machines.
//! Browser wire operations keep their explicit member IDs and existing admission.
use super::{Member, service, store as sql};
use crate::{
    error::RemoteError,
    store::{Grant, Store, database},
};
use rusqlite::params;

type Result<T> = std::result::Result<T, RemoteError>;
fn missing() -> RemoteError {
    RemoteError::new(
        "REMOTE_MEMBER_NOT_FOUND",
        "No member has that ID or unique name.",
    )
}
/// Stable IDs win over display names, including opaque IDs retained by migration.
/// Names never select a machine or acquire authority; the effect rechecks its revision.
pub fn local_member(store: &Store, reference: &str) -> Result<Member> {
    if reference.is_empty() {
        return Err(service::invalid());
    }
    let scope = service::scope(&store.connection, service::Caller::LocalOwner, 0)?;
    if let Some(member) = sql::member_in(&store.connection, &scope.remote, reference)? {
        return Ok(member);
    }
    let mut query = store.connection.prepare(
        "SELECT member_id FROM members WHERE remote_id=?1 AND display_name=?2 ORDER BY member_id LIMIT 2"
    ).map_err(database)?;
    let matches = query
        .query_map(params![scope.remote, reference], |row| {
            row.get::<_, String>(0)
        })
        .map_err(database)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(database)?;
    match matches.as_slice() {
        [id] => sql::member_in(&store.connection, &scope.remote, id)?.ok_or_else(missing),
        [] => Err(missing()),
        _ => Err(RemoteError::new(
            "REMOTE_MEMBER_AMBIGUOUS",
            "More than one member has that name. Use the member ID shown by tmt remote members.",
        )),
    }
}
/// Machine operations require the exact client ID; a person selector never expands to all machines.
pub fn local_machine(store: &Store, client: &str) -> Result<(Member, Grant)> {
    let scope = service::scope(&store.connection, service::Caller::LocalOwner, 0)?;
    let grant = store.grant(client)?.ok_or_else(|| {
        RemoteError::new("REMOTE_MACHINE_NOT_FOUND", "No machine has that client ID.")
    })?;
    let member = sql::for_client(&store.connection, &grant.client_id)?
        .filter(|m| m.remote_id == scope.remote)
        .ok_or_else(missing)?;
    Ok((member, grant))
}
