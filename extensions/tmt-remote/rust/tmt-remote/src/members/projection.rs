//! SQL limits precede materialization. Cursor row positions never carry authority.
use super::{
    service::{
        self, Machine, Machines, MemberViewAccess, Page, Person, PersonObservation, Result, Roster,
    },
    store as sql,
};
use crate::{
    canonical,
    store::{Store, database, grant_in},
};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

pub(super) fn summary(c: &Connection, scope: &MemberViewAccess) -> Result<service::Summary> {
    let (member_count, last_active_at_ms) = if scope.administrator() {
        // Count people once, independently of their number of associated machines. The
        // timestamp is historical accepted use, including use by a now-revoked machine.
        let (count, last) = c.query_row(
            "SELECT (SELECT COUNT(*) FROM members WHERE remote_id=?1 AND revoked_at_ms IS NULL), \
             (SELECT MAX(u.used_at_ms) FROM device_last_use u JOIN member_machines m \
              ON m.client_id=u.client_id WHERE m.remote_id=?1)",
            [&scope.remote],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
        ).map_err(database)?;
        (
            Some(u64::try_from(count).map_err(database)?),
            last.map(u64::try_from).transpose().map_err(database)?,
        )
    } else {
        (None, None)
    };
    Ok(service::Summary {
        remote_id: scope.remote.clone(),
        member_count,
        last_active_at_ms,
    })
}

fn binding(scope: &MemberViewAccess, view: &str, target: &str) -> [u8; 32] {
    let frame = serde_json::to_vec(&(&scope.remote, &scope.caller, &scope.member, view, target))
        .expect("string frame");
    Sha256::digest(frame).into()
}
fn after(page: &Page, binding: &[u8; 32]) -> Result<i64> {
    let Some(cursor) = &page.cursor else {
        return Ok(0);
    };
    if cursor.len() != 54 {
        return Err(service::invalid());
    }
    let bytes = canonical::base64url_decode(cursor).map_err(|_| service::invalid())?;
    if bytes.len() != 40 || bytes[..32] != binding[..] {
        return Err(service::invalid());
    }
    let after = i64::from_be_bytes(bytes[32..].try_into().map_err(|_| service::invalid())?);
    if after <= 0 {
        return Err(service::invalid());
    }
    Ok(after)
}
fn next<T>(rows: &mut Vec<(i64, T)>, limit: usize, binding: &[u8; 32]) -> Option<String> {
    if rows.len() <= limit {
        return None;
    }
    rows.pop();
    let mut bytes = binding.to_vec();
    bytes.extend_from_slice(&rows.last().expect("bounded nonempty page").0.to_be_bytes());
    Some(canonical::base64url(&bytes))
}
pub(super) fn roster(c: &Connection, scope: &MemberViewAccess, page: Page) -> Result<Roster> {
    let limit = service::page_size(&page)?;
    let binding = binding(scope, "members", "");
    let after = after(&page, &binding)?;
    let mut query=c.prepare("SELECT remote_id,member_id,role,display_name,sending_enabled,revision,created_at_ms,revoked_at_ms,rowid, (SELECT MAX(u.used_at_ms) FROM member_machines m JOIN device_last_use u ON u.client_id=m.client_id WHERE m.remote_id=members.remote_id AND m.member_id=members.member_id AND u.used_at_ms>=m.associated_at_ms) FROM members WHERE remote_id=?1 AND (?2 IS NULL OR member_id=?2) AND rowid>?3 ORDER BY rowid LIMIT ?4").map_err(database)?;
    let mut rows = query
        .query_map(params![scope.remote, scope.member, after, limit], |r| {
            Ok((
                r.get::<_, i64>(8)?,
                PersonObservation {
                    person: Person::from(sql::row(r)?),
                    last_used_at_ms: r
                        .get::<_, Option<i64>>(9)?
                        .map(u64::try_from)
                        .transpose()
                        .map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                9,
                                rusqlite::types::Type::Integer,
                                Box::new(error),
                            )
                        })?,
                },
            ))
        })
        .map_err(database)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(database)?;
    let next_cursor = next(&mut rows, page.limit, &binding);
    Ok(Roster {
        remote_id: scope.remote.clone(),
        administrator: scope.administrator(),
        display_name: None,
        invitation: None,
        routes: super::observations::MemberRouteView::Unavailable,
        members: rows.into_iter().map(|(_, m)| m).collect(),
        next_cursor,
    })
}
pub(super) fn machines(
    store: &Store,
    scope: &MemberViewAccess,
    member: &str,
    page: Page,
    now: u64,
) -> Result<Machines> {
    let limit = service::page_size(&page)?;
    let binding = binding(scope, "machines", member);
    let after = after(&page, &binding)?;
    let mut query=store.connection.prepare("SELECT rowid,client_id FROM member_machines WHERE remote_id=?1 AND member_id=?2 AND rowid>?3 ORDER BY rowid LIMIT ?4").map_err(database)?;
    let mut rows = query
        .query_map(params![scope.remote, member, after, limit], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(database)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(database)?;
    let next_cursor = next(&mut rows, page.limit, &binding);
    let machines = rows
        .into_iter()
        .map(|(_, id)| {
            let grant = grant_in(&store.connection, &id)?.ok_or_else(service::invalid)?;
            // Validate stored policy without disclosing recipient identities or keys.
            let authority = grant.authority()?;
            let recipient_access = match authority.agents {
                crate::authority::PermittedAgents::All => "all",
                crate::authority::PermittedAgents::Selected(_) => "restricted",
            };
            Ok(Machine {
                client_id: id.clone(),
                name: grant.name.clone(),
                kind: grant.kind.clone(),
                revision: grant.revision,
                revoked: grant.disabled,
                expires_at_ms: grant.expires_at_ms,
                sending_scope: grant.permits_scope("talk"),
                delivery: grant.mode.clone(),
                recipient_access,
                grant_live: grant.live_at(now),
                last_used_at_ms: store.last_used(&id)?,
                this_browser: id == scope.caller,
                live_session_count: None,
                last_activity_at_ms: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    // Re-read scope in the adapter before disclosure; no provider traffic here.
    let _ =
        sql::member_in(&store.connection, &scope.remote, member)?.ok_or_else(service::invalid)?;
    Ok(Machines {
        remote_id: scope.remote.clone(),
        member_id: member.into(),
        machines,
        next_cursor,
    })
}
