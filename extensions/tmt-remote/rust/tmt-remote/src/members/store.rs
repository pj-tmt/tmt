//! Member SQL shares the existing Store connection, writer fence and journal.
use super::{Association, AssociationOutcome, Change, Member, Owner, Role};
use crate::{
    canonical,
    error::RemoteError,
    store::{Grant, Store, database, grant_in, machine_in},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, RemoteError>;
const COLUMNS: &str =
    "remote_id,member_id,role,display_name,sending_enabled,revision,created_at_ms,revoked_at_ms";
fn invalid() -> RemoteError {
    RemoteError::new(
        "REMOTE_MEMBER_CONFLICT",
        "Member or machine changed, is unavailable, or belongs to another remote. Review and confirm again.",
    )
}
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Member> {
    let role: String = r.get(2)?;
    Ok(Member {
        remote_id: r.get(0)?,
        member_id: r.get(1)?,
        role: if role == "owner" {
            Role::Owner
        } else {
            Role::User
        },
        display_name: r.get(3)?,
        sending_enabled: r.get(4)?,
        revision: r.get::<_, i64>(5)? as u64,
        created_at_ms: r.get::<_, i64>(6)? as u64,
        revoked_at_ms: r.get::<_, Option<i64>>(7)?.map(|n| n as u64),
    })
}
fn member_in(c: &Connection, remote: &str, id: &str) -> Result<Option<Member>> {
    c.query_row(
        &format!("SELECT {COLUMNS} FROM members WHERE remote_id=?1 AND member_id=?2"),
        params![remote, id],
        row,
    )
    .optional()
    .map_err(database)
}
fn for_client(c: &Connection, client: &str) -> Result<Option<Member>> {
    let id: Option<(String, String)> = c
        .query_row(
            "SELECT remote_id,member_id FROM member_machines WHERE client_id=?1",
            [client],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(database)?;
    id.map(|(remote, id)| member_in(c, &remote, &id))
        .transpose()
        .map(Option::flatten)
}
fn clients(c: &Connection, remote: &str, id: &str) -> Result<Vec<String>> {
    c.prepare("SELECT client_id FROM member_machines WHERE remote_id=?1 AND member_id=?2 ORDER BY client_id").and_then(|mut q|q.query_map(params![remote,id],|r|r.get(0))?.collect()).map_err(database)
}
fn bounded_time(now: u64) -> Result<i64> {
    i64::try_from(now)
        .ok()
        .filter(|n| *n <= 9_007_199_254_740_991)
        .ok_or_else(invalid)
}
fn bump(c: &Connection, remote: &str, id: &str) -> Result<()> {
    if c.execute("UPDATE members SET revision=revision+1 WHERE remote_id=?1 AND member_id=?2 AND revision<9007199254740991",params![remote,id]).map_err(database)? !=1 {return Err(invalid());}
    Ok(())
}
pub(crate) fn require_owner(c: &Connection, owner: &Owner<'_>, now: u64) -> Result<String> {
    match owner {
        Owner::LocalOwner => Ok("local-owner".into()),
        Owner::DesignatedBrowser {
            client_id,
            revision,
        } => {
            let g = grant_in(c, client_id)?.ok_or_else(invalid)?;
            let designated:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM settings_designation d JOIN machine m ON m.singleton=d.singleton WHERE d.machine_id=m.machine_id AND d.client_id=?1 AND d.public_key=?2)",params![client_id,g.public_key.as_slice()],|r|r.get(0)).map_err(database)?;
            if !designated || g.kind != "browser" || !g.live_at(now) || g.revision != *revision {
                return Err(RemoteError::new(
                    "REMOTE_MEMBER_OWNER_REQUIRED",
                    "Only the local owner or its currently designated browser may change member associations or policy.",
                ));
            }
            Ok(format!("browser:{client_id}"))
        }
    }
}
/// Every legacy grant is its own member, never grouped by a display/account name.
pub(crate) fn migrate_legacy(c: &Connection) -> Result<()> {
    let grants: Vec<Grant> = c
        .prepare(&format!(
            "SELECT {} FROM grants ORDER BY client_id",
            crate::store::GRANT_COLUMNS
        ))
        .and_then(|mut q| q.query_map([], crate::store::grant_row)?.collect())
        .map_err(database)?;
    for g in grants {
        insert_mapping(c, &g)?;
    }
    sync_designation(c)
}
pub(crate) fn insert_mapping(c: &Connection, g: &Grant) -> Result<()> {
    let remote = machine_in(c)?.id;
    insert_member(c, &remote, &g.client_id, g)?;
    c.execute(
        "INSERT INTO member_machines VALUES(?1,?2,?2,?3)",
        params![remote, g.client_id, bounded_time(g.issued_at_ms)?],
    )
    .map_err(database)?;
    sync_designation(c)?;
    Ok(())
}
fn insert_member(c: &Connection, remote: &str, id: &str, g: &Grant) -> Result<()> {
    c.execute(
        "INSERT INTO members VALUES(?1,?2,'user',?3,?4,1,?5,?6)",
        params![
            remote,
            id,
            g.name,
            g.permits_scope("talk"),
            bounded_time(g.issued_at_ms)?,
            g.disabled.then_some(bounded_time(g.issued_at_ms)?)
        ],
    )
    .map_err(database)?;
    Ok(())
}
/// Role follows exact existing owner designation; a role itself grants no authority.
pub(crate) fn sync_designation(c: &Connection) -> Result<()> {
    c.execute("UPDATE members SET role='user',revision=revision+1 WHERE role='owner' AND NOT EXISTS(SELECT 1 FROM member_machines mm JOIN settings_designation d ON d.client_id=mm.client_id JOIN machine m ON m.singleton=d.singleton JOIN grants g ON g.client_id=d.client_id WHERE mm.remote_id=members.remote_id AND mm.member_id=members.member_id AND d.machine_id=m.machine_id AND d.public_key=g.public_key)",[]).map_err(database)?;
    c.execute("UPDATE members SET role='owner',revision=revision+1 WHERE role!='owner' AND revoked_at_ms IS NULL AND EXISTS(SELECT 1 FROM member_machines mm JOIN settings_designation d ON d.client_id=mm.client_id JOIN machine m ON m.singleton=d.singleton JOIN grants g ON g.client_id=d.client_id WHERE mm.remote_id=members.remote_id AND mm.member_id=members.member_id AND d.machine_id=m.machine_id AND d.public_key=g.public_key AND g.disabled=0)",[]).map_err(database)?;
    Ok(())
}
fn fence(c: &Connection, client: &str, now: u64) -> Result<()> {
    c.execute("UPDATE sessions SET ended_reason='REMOTE_GRANT_CHANGED',ended_at_ms=?2 WHERE client_id=?1 AND ended_reason IS NULL",params![client,bounded_time(now)?]).map_err(database)?;
    Ok(())
}
fn check_member(c: &Connection, id: &str, revision: u64) -> Result<Member> {
    let remote = machine_in(c)?.id;
    member_in(c, &remote, id)?
        .filter(|m| m.revoked_at_ms.is_none() && m.revision == revision)
        .ok_or_else(invalid)
}
impl Store {
    pub fn members(&self) -> Result<Vec<Member>> {
        self.connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM members ORDER BY created_at_ms,member_id"
            ))
            .and_then(|mut q| q.query_map([], row)?.collect())
            .map_err(database)
    }
    pub fn member_for_client(&self, client: &str) -> Result<Option<Member>> {
        for_client(&self.connection, client)
    }
    pub fn member_clients(&self, member: &str) -> Result<Vec<String>> {
        let remote: String = self
            .connection
            .query_row(
                "SELECT machine_id FROM machine WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(database)?;
        clients(&self.connection, &remote, member)
    }
    /// Only the trusted confirmation coordinator calls this sink after owner
    /// confirmation. Proof verification alone must never call it. No secret is saved.
    pub fn confirm_member_grant(&mut self, grant: &Grant) -> Result<Member> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let member = crate::members::confirm_member_grant_in(&tx, grant)?;
        tx.commit().map_err(database)?;
        Ok(member)
    }

    pub fn associate_member(
        &mut self,
        owner: Owner<'_>,
        request: &Association,
        now: u64,
    ) -> Result<AssociationOutcome> {
        bounded_time(now)?;
        if ![
            &request.operation_id,
            &request.remote_id,
            &request.member_id,
            &request.client_id,
        ]
        .iter()
        .all(|s| canonical::is_core_id(s))
        {
            return Err(invalid());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let actor = require_owner(&tx, &owner, now)?;
        let (receipt_client, receipt_revision) = match &owner {
            Owner::LocalOwner => (request.client_id.clone(), request.grant_revision),
            Owner::DesignatedBrowser {
                client_id,
                revision,
            } => (client_id.to_string(), *revision),
        };
        if machine_in(&tx)?.id != request.remote_id {
            return Err(invalid());
        }
        let digest = Sha256::digest(serde_json::to_vec(&(request, actor)).map_err(database)?);
        let saved: Option<(String, Vec<u8>, String, i64)> = tx
            .query_row(
                "SELECT operation,digest,outcome,deadline_ms FROM management_receipts WHERE id=?1",
                [&request.operation_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(database)?;
        if let Some((operation, prior, outcome, deadline)) = saved {
            if operation != "remote.members.associate" || prior.as_slice() != digest.as_slice() {
                return Err(RemoteError::new(
                    "REMOTE_ID_CONFLICT",
                    "This operation ID already names a different action.",
                ));
            }
            if bounded_time(now)? >= deadline {
                return Err(RemoteError::new(
                    "REMOTE_RESULT_EXPIRED",
                    "The original association outcome is no longer available; its identity remains reserved.",
                ));
            }
            return serde_json::from_str(&outcome).map_err(database);
        }
        let (client_count,total):(i64,i64)=tx.query_row("SELECT (SELECT COUNT(*) FROM management_receipts WHERE client_id=?1),COUNT(*) FROM management_receipts",[&receipt_client],|r|Ok((r.get(0)?,r.get(1)?))).map_err(database)?;
        if client_count >= crate::journal::OPERATIONS || total >= 4 * crate::journal::OPERATIONS {
            return Err(RemoteError::new(
                "REMOTE_MANAGEMENT_CAPACITY",
                "Retained management identity limit reached.",
            ));
        }
        let target = check_member(&tx, &request.member_id, request.member_revision)?;
        let mut grant = grant_in(&tx, &request.client_id)?
            .filter(|g| g.live_at(now) && g.revision == request.grant_revision)
            .ok_or_else(invalid)?;
        let source = for_client(&tx, &request.client_id)?
            .filter(|m| m.remote_id == request.remote_id && m.revoked_at_ms.is_none())
            .ok_or_else(invalid)?;
        if source.member_id != target.member_id {
            if !target.sending_enabled {
                crate::store::talk_grant_in(&tx, &grant.client_id, false)?;
            }
            // Association invalidates old admitted work even when scopes were unchanged.
            let current = grant_in(&tx, &grant.client_id)?.ok_or_else(invalid)?;
            if current.revision == grant.revision
                && tx.execute("UPDATE grants SET revision=revision+1 WHERE client_id=?1 AND revision<9007199254740991",[&grant.client_id]).map_err(database)?!=1 {return Err(invalid());}
            tx.execute("UPDATE member_machines SET member_id=?3,associated_at_ms=?4 WHERE remote_id=?1 AND client_id=?2",params![request.remote_id,grant.client_id,target.member_id,bounded_time(now)?]).map_err(database)?;
            bump(&tx, &target.remote_id, &target.member_id)?;
            bump(&tx, &source.remote_id, &source.member_id)?;
            if clients(&tx, &source.remote_id, &source.member_id)?.is_empty() {
                tx.execute("UPDATE members SET revoked_at_ms=?3,sending_enabled=0 WHERE remote_id=?1 AND member_id=?2",params![source.remote_id,source.member_id,bounded_time(now)?]).map_err(database)?;
            }
            fence(&tx, &grant.client_id, now)?;
            sync_designation(&tx)?;
        }
        grant = grant_in(&tx, &grant.client_id)?.ok_or_else(invalid)?;
        let outcome = AssociationOutcome {
            member: member_in(&tx, &target.remote_id, &target.member_id)?.ok_or_else(invalid)?,
            client_id: grant.client_id.clone(),
            grant_revision: grant.revision,
        };
        tx.execute("INSERT INTO management_receipts VALUES(?1,?2,'remote.members.associate',?3,?4,?5,?6,?7)",params![request.operation_id,receipt_client,digest.as_slice(),receipt_revision as i64,bounded_time(now)?,bounded_time(now.saturating_add(crate::journal::RECOVERY))?,serde_json::to_string(&outcome).map_err(database)?]).map_err(database)?;
        tx.commit().map_err(database)?;
        Ok(outcome)
    }
    pub fn set_member_sending(
        &mut self,
        owner: Owner<'_>,
        id: &str,
        expected: u64,
        enabled: bool,
        now: u64,
    ) -> Result<Change> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        require_owner(&tx, &owner, now)?;
        let member = check_member(&tx, id, expected)?;
        let change = sending_in(&tx, member, enabled, now)?;
        tx.commit().map_err(database)?;
        Ok(change)
    }
    pub fn revoke_member(
        &mut self,
        owner: Owner<'_>,
        id: &str,
        expected: u64,
        now: u64,
    ) -> Result<Change> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        require_owner(&tx, &owner, now)?;
        let member = check_member(&tx, id, expected)?;
        let ids = clients(&tx, &member.remote_id, id)?;
        for client in &ids {
            crate::store::revoke_in(&tx, client)?;
            tx.execute(
                "UPDATE cloud_peer_routes SET revoked=1,pending=1 WHERE client_id=?1",
                [client],
            )
            .map_err(database)?;
            fence(&tx, client, now)?;
        }
        tx.execute("UPDATE members SET revoked_at_ms=?3,sending_enabled=0 WHERE remote_id=?1 AND member_id=?2",params![member.remote_id,id,bounded_time(now)?]).map_err(database)?;
        bump(&tx, &member.remote_id, id)?;
        sync_designation(&tx)?;
        let member = member_in(&tx, &member.remote_id, id)?.ok_or_else(invalid)?;
        tx.commit().map_err(database)?;
        Ok(Change {
            member,
            clients: ids,
        })
    }
}

pub(crate) fn sending_for_client(
    c: &Connection,
    client: &str,
    enabled: bool,
) -> Result<Option<Grant>> {
    let Some(grant) = grant_in(c, client)? else {
        return Ok(None);
    };
    if grant.disabled {
        return Err(RemoteError::new(
            "REMOTE_DEVICE_REVOKED",
            "A revoked device cannot grant sending.",
        ));
    }
    let member = for_client(c, client)?
        .filter(|m| m.revoked_at_ms.is_none())
        .ok_or_else(invalid)?;
    sending_in(c, member, enabled, crate::pairing::now_ms()?)?;
    grant_in(c, client)
}
fn sending_in(c: &Connection, member: Member, enabled: bool, now: u64) -> Result<Change> {
    let ids = clients(c, &member.remote_id, &member.member_id)?;
    let mut changed = Vec::new();
    for client in ids {
        let before = grant_in(c, &client)?.ok_or_else(invalid)?;
        if before.disabled {
            continue;
        }
        let after = crate::store::talk_grant_in(c, &client, enabled)?.ok_or_else(invalid)?;
        if after.revision != before.revision {
            fence(c, &client, now)?;
            changed.push(client);
        }
    }
    if member.sending_enabled != enabled {
        c.execute(
            "UPDATE members SET sending_enabled=?3 WHERE remote_id=?1 AND member_id=?2",
            params![member.remote_id, member.member_id, enabled],
        )
        .map_err(database)?;
        bump(c, &member.remote_id, &member.member_id)?;
    }
    let member = member_in(c, &member.remote_id, &member.member_id)?.ok_or_else(invalid)?;
    Ok(Change {
        member,
        clients: changed,
    })
}
pub(crate) fn renamed(c: &Connection, client: &str, name: &str) -> Result<()> {
    let Some(member) = for_client(c, client)? else {
        return Ok(());
    };
    if member.display_name != name {
        c.execute(
            "UPDATE members SET display_name=?3 WHERE remote_id=?1 AND member_id=?2",
            params![member.remote_id, member.member_id, name],
        )
        .map_err(database)?;
        bump(c, &member.remote_id, &member.member_id)?;
    }
    Ok(())
}
pub(crate) fn revoked(c: &Connection, client: &str) -> Result<()> {
    if let Some(member) = for_client(c, client)? {
        let live:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM member_machines mm JOIN grants g ON g.client_id=mm.client_id WHERE mm.remote_id=?1 AND mm.member_id=?2 AND g.disabled=0)",params![member.remote_id,member.member_id],|r|r.get(0)).map_err(database)?;
        if !live && member.revoked_at_ms.is_none() {
            c.execute("UPDATE members SET revoked_at_ms=?3,sending_enabled=0 WHERE remote_id=?1 AND member_id=?2",params![member.remote_id,member.member_id,bounded_time(crate::pairing::now_ms()?)?]).map_err(database)?;
            bump(c, &member.remote_id, &member.member_id)?;
        }
        sync_designation(c)?;
    }
    Ok(())
}

/// The concrete confirmation adapter owns one IMMEDIATE Store transaction and
/// saves its admitted original receipt/policy in that same transaction. It must
/// not call the convenience Store method while holding this transaction.
pub(crate) fn confirm_member_grant_in(
    tx: &rusqlite::Transaction<'_>,
    grant: &Grant,
) -> Result<Member> {
    if grant.disabled || grant.revision != 1 {
        return Err(invalid());
    }
    let mut grant = grant.clone();
    grant.scopes.retain(|s| s != "talk");
    grant.mode = "direct".into();
    if let Some(previous) = grant_in(tx, &grant.client_id)? {
        if !previous.disabled {
            return Err(RemoteError::new(
                "REMOTE_ALREADY_PAIRED",
                "This device key already has a live grant.",
            ));
        }
        if !canonical::is_core_id(&grant.client_id) {
            return Err(database("invalid grant identity for member association"));
        }
        // Keep the final JSON-safe revision available for revocation. Reusing
        // revision 1 would revive old sessions and captured receipt claims.
        grant.revision = previous
            .revision
            .checked_add(1)
            .filter(|revision| *revision < 9_007_199_254_740_991)
            .ok_or_else(|| database("grant revision exhausted"))?;
        grant.authority()?;
        let member = crate::store::uuid_v4()?;
        let remote = machine_in(tx)?.id;
        insert_member(tx, &remote, &member, &grant)?;
        tx.execute(
            "UPDATE grants SET public_key=?2,kind=?3,origin=?4,name=?5,agents=?6,scopes=?7,mode=?8,issued_at_ms=?9,expires_at_ms=?10,revision=?11,disabled=0 WHERE client_id=?1 AND disabled=1 AND revision=?12",
            params![grant.client_id,grant.public_key.as_slice(),grant.kind,grant.origin,grant.name,grant.agents,grant.scopes.join(" "),grant.mode,bounded_time(grant.issued_at_ms)?,grant.expires_at_ms.map(bounded_time).transpose()?,grant.revision as i64,previous.revision as i64],
        ).map_err(|error| match error {
            rusqlite::Error::SqliteFailure(failure, _) if failure.code == rusqlite::ErrorCode::ConstraintViolation => RemoteError::new("REMOTE_ALREADY_PAIRED", "This device key already has a live grant."),
            other => database(other),
        })?;
        if tx.execute(
            "UPDATE member_machines SET member_id=?3,associated_at_ms=?4 WHERE remote_id=?1 AND client_id=?2",
            params![remote,grant.client_id,member,bounded_time(grant.issued_at_ms)?],
        ).map_err(database)? != 1 {
            return Err(invalid());
        }
        fence(tx, &grant.client_id, grant.issued_at_ms)?;
        sync_designation(tx)?;
    } else {
        crate::store::insert_grant_in(tx, &grant)?;
    }
    for_client(tx, &grant.client_id)?.ok_or_else(invalid)
}
pub(crate) fn grants_for_client(c: &Connection, client: &str) -> Result<Vec<Grant>> {
    let Some(member) = for_client(c, client)? else {
        return Ok(Vec::new());
    };
    clients(c, &member.remote_id, &member.member_id)?
        .iter()
        .map(|id| grant_in(c, id)?.ok_or_else(invalid))
        .collect()
}
