//! Cloud enrollment and desired uid routes share Remote's sole Store/serve lease.
//! HTTP publication happens outside SQLite; acknowledgments compare the snapshot.
use super::{Challenge, VerifiedCandidate, VerifiedReroute};
use crate::{
    crypto,
    error::RemoteError,
    store::{DEFAULT_SCOPES, Grant, Store, database, grant_in, insert_grant_in},
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

/// Private pairing record; the public catalog alone never supplies this pin.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub version: u64,
    pub name: String,
    pub owner: String,
    pub owner_key: [u8; 32],
    pub config: serde_json::Value,
    pub peer: String,
    pub uid: String,
    pub request: Vec<u8>,
    pub confirmed: bool,
    pub descriptor: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reroute_token: Option<[u8; 32]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairReceipt {
    pub bytes: Vec<u8>,
    pub proof: [u8; 32],
}

/// Desired owner-written route. A batch for one peer must be published in one
/// provider commit so retiring old uid rows and creating the new row are atomic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteChange {
    pub project: String,
    pub uid: String,
    pub peer: String,
    pub generation: u64,
    pub revoked: bool,
    pub reroute_hash: Option<[u8; 32]>,
}

fn refused() -> RemoteError {
    RemoteError::new(
        "REMOTE_CLOUD_PAIR_INVALID",
        "Cloud enrollment could not be verified.",
    )
}
fn live(connection: &rusqlite::Connection, peer: &str, now: u64) -> Result<Grant, RemoteError> {
    grant_in(connection, peer)?
        .filter(|grant| !grant.disabled && grant.expires_at_ms.is_none_or(|expiry| now < expiry))
        .ok_or_else(refused)
}

fn discovery_key(project: &str, peer: &str) -> Result<String, RemoteError> {
    if !crate::deploy_run::project_ok(project) {
        return Err(refused());
    }
    crate::canonical::uuid(peer).map_err(|_| refused())?;
    // This namespace cannot collide with enrollment UUIDs or the inbox cursor.
    Ok(format!("peer-discovery:{project}:{peer}"))
}

impl Store {
    /// The joiner creates its receiving grant only after verifying the owner's
    /// receipt and obtaining its own local sending choice. Re-observation never
    /// resets policy, revocation, or an existing UID route.
    pub(crate) fn confirm_reciprocal(
        &mut self,
        connection: &Connection,
        owner_uid: &str,
        grant: &Grant,
        local_key: &[u8; 32],
        talk: bool,
        reverse_token: &[u8; 32],
    ) -> Result<Grant, RemoteError> {
        if self.machine()?.id != connection.peer
            || !connection.confirmed
            || !super::uid_ok(owner_uid)
            || grant.client_id != connection.owner
            || grant.public_key != connection.owner_key
            || grant.kind != "cli"
            || grant.origin != "cli"
        {
            return Err(refused());
        }
        let project = connection.config["projectId"]
            .as_str()
            .ok_or_else(refused)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        if let Some(old) = grant_in(&tx, &grant.client_id)? {
            if old.public_key != grant.public_key || old.kind != "cli" || old.origin != "cli" {
                return Err(refused());
            }
            return Ok(old);
        }
        insert_grant_in(&tx, grant)?;
        let grant = if talk {
            crate::store::talk_in(&tx, &grant.client_id, true)?.ok_or_else(refused)?
        } else {
            grant.clone()
        };
        use sha2::{Digest, Sha256};
        let reverse_hash = Sha256::digest(reverse_token);
        tx.execute(
            "INSERT INTO cloud_peers(project,client_id,owner,owner_key,generation,reroute_hash) VALUES(?1,?2,?3,?4,1,?5)",
            params![
                project,
                connection.owner,
                connection.peer,
                local_key.as_slice(),
                reverse_hash.as_slice()
            ],
        )
        .map_err(database)?;
        tx.execute(
            "INSERT INTO cloud_peer_routes VALUES(?1,?2,?3,1,0,1)",
            params![project, owner_uid, connection.owner],
        )
        .map_err(database)?;
        tx.execute("INSERT INTO cloud_owner_projects(project,config) VALUES(?1,?2) ON CONFLICT(project) DO NOTHING",
            params![project, connection.config.to_string()]).map_err(database)?;
        tx.execute("INSERT INTO cloud_remotes(name,owner,connection) VALUES(?1,?2,?3) ON CONFLICT(name) DO UPDATE SET connection=excluded.connection",
            params![connection.name, connection.owner, serde_json::to_string(connection).map_err(database)?]).map_err(database)?;
        tx.commit().map_err(database)?;
        Ok(grant)
    }
    pub fn prune_cloud_receipts(&self, now: u64) -> Result<(), RemoteError> {
        self.connection.execute("DELETE FROM cloud_pair_receipts WHERE offer IN (SELECT offer FROM cloud_pair_receipts WHERE deadline_ms<=?1 LIMIT ?2)",params![now as i64,crate::journal::OPERATIONS]).map_err(database)?;
        Ok(())
    }
    pub fn cloud_connection(&self, name: &str) -> Result<Option<Connection>, RemoteError> {
        self.connection
            .query_row(
                "SELECT connection FROM cloud_remotes WHERE name=?1 OR owner=?1 ORDER BY name=?1 DESC LIMIT 1",
                [name],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(database)?
            .map(|s| {
                let record: Connection = serde_json::from_str(&s).map_err(database)?;
                if record.version != 1 { return Err(super::wire::upgrade()); }
                Ok(record)
            })
            .transpose()
    }
    pub fn save_cloud_connection(&self, record: &Connection) -> Result<(), RemoteError> {
        if let Some(old) = self.cloud_connection(&record.name)?
            && (old.owner != record.owner
                || old.owner_key != record.owner_key
                || old.peer != record.peer)
        {
            return Err(refused());
        }
        self.connection.execute("INSERT INTO cloud_remotes VALUES(?1,?2,?3) ON CONFLICT(name) DO UPDATE SET connection=excluded.connection",params![record.name,record.owner,serde_json::to_string(record).map_err(database)?]).map_err(database)?;
        Ok(())
    }
    pub(crate) fn cloud_connections(&self) -> Result<Vec<Connection>, RemoteError> {
        let mut statement = self
            .connection
            .prepare("SELECT name FROM cloud_remotes ORDER BY name")
            .map_err(database)?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database)?;
        names
            .into_iter()
            .map(|name| self.cloud_connection(&name))
            .collect::<Result<Vec<_>, _>>()
            .map(|connections| connections.into_iter().flatten().collect())
    }
    pub fn remember_cloud_owner(
        &self,
        project: &str,
        config: &serde_json::Value,
    ) -> Result<(), RemoteError> {
        self.connection.execute("INSERT INTO cloud_owner_projects VALUES(?1,?2) ON CONFLICT(project) DO UPDATE SET config=excluded.config",params![project,serde_json::to_string(config).map_err(database)?]).map_err(database)?;
        Ok(())
    }
    pub fn cloud_owner_configs(&self) -> Result<Vec<serde_json::Value>, RemoteError> {
        let values:Vec<String>=self.connection.prepare("SELECT config FROM cloud_owner_projects WHERE EXISTS(SELECT 1 FROM cloud_peers WHERE cloud_peers.project=cloud_owner_projects.project)").and_then(|mut q|q.query_map([],|r|r.get(0))?.collect()).map_err(database)?;
        values
            .into_iter()
            .map(|v| serde_json::from_str(&v).map_err(database))
            .collect()
    }
    /// Negative discovery is polling metadata, never a peer pin or grant.
    pub(crate) fn cloud_peer_discovery_ready(
        &self,
        project: &str,
        peer: &str,
        now: u64,
    ) -> Result<bool, RemoteError> {
        crate::cloud_inbox::Checkpoint::new(now)?;
        let key = discovery_key(project, peer)?;
        let observed: Option<i64> = self.connection.query_row(
            "SELECT created_ms FROM cloud_transport WHERE peer_id=?1 AND kind='checkpoint' AND id=''",
            [&key], |row| row.get(0),
        ).optional().map_err(database)?;
        let observed = observed
            .map(|time| crate::cloud_inbox::Checkpoint::new(time as u64))
            .transpose()?;
        Ok(observed.is_none_or(|time| {
            now.saturating_sub(time.created_at_ms()) >= crate::limits::CLOUD_PEER_DISCOVERY_RETRY_MS
        }))
    }
    pub(crate) fn cloud_defer_peer_discovery(
        &mut self,
        project: &str,
        peer: &str,
        now: u64,
    ) -> Result<(), RemoteError> {
        self.cloud_complete(&[discovery_key(project, peer)?], now)
    }
    pub(crate) fn cloud_project_peers(&self, project: &str) -> Result<Vec<Grant>, RemoteError> {
        let mut statement = self
            .connection
            .prepare("SELECT client_id FROM cloud_peers WHERE project=?1 ORDER BY client_id")
            .map_err(database)?;
        let peers = statement
            .query_map([project], |row| row.get::<_, String>(0))
            .map_err(database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database)?;
        peers
            .into_iter()
            .map(|peer| self.grant(&peer))
            .collect::<Result<Vec<_>, _>>()
            .map(|grants| grants.into_iter().flatten().collect())
    }
    pub fn recover_cloud_join(
        &self,
        offer: &str,
        submission: &super::wire::Submission,
        uid: &str,
        now: u64,
    ) -> Result<PairReceipt, RemoteError> {
        let raw: String = self
            .connection
            .query_row(
                "SELECT challenge FROM cloud_pair_receipts WHERE offer=?1",
                [offer],
                |r| r.get(0),
            )
            .optional()
            .map_err(database)?
            .ok_or_else(refused)?;
        let challenge: Challenge = serde_json::from_str(&raw).map_err(database)?;
        self.cloud_join_receipt(
            &challenge,
            &submission.candidate,
            uid,
            &submission.mac,
            &submission.signature,
            now,
        )
    }
    /// Return only the original confirmed receipt for an exact candidate retry.
    /// No raw code or response key is retained, and this path can create no grant.
    pub fn cloud_join_receipt(
        &self,
        challenge: &Challenge,
        candidate: &super::Candidate,
        authenticated_uid: &str,
        mac: &[u8; 32],
        signature: &[u8; 64],
        now: u64,
    ) -> Result<PairReceipt, RemoteError> {
        if authenticated_uid != candidate.uid || now >= challenge.expires_at_ms {
            return Err(refused());
        }
        let digest = super::possession_digest(challenge, candidate, mac, signature)
            .map_err(|_| refused())?;
        let receipt: Option<(Vec<u8>, Vec<u8>)> = self.connection.query_row(
            "SELECT receipt,proof FROM cloud_pair_receipts WHERE offer=?1 AND digest=?2 AND deadline_ms>?3",
            params![challenge.offer, digest.as_slice(), now as i64], |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional().map_err(database)?;
        let (bytes, proof) = receipt.ok_or_else(refused)?;
        Ok(PairReceipt {
            bytes,
            proof: proof
                .try_into()
                .map_err(|_| database("invalid pairing receipt proof"))?,
        })
    }

    /// Local-owner confirmation only. A verified submission alone never calls
    /// this method. Commit the read-only grant, receipt and pending route together.
    pub fn confirm_cloud_peer(
        &mut self,
        verified: &VerifiedCandidate,
        response_key: &[u8; 32],
        now: u64,
        talk: bool,
        reciprocal: Option<Connection>,
    ) -> Result<PairReceipt, RemoteError> {
        let challenge = verified.challenge();
        let candidate = verified.candidate();
        if now >= challenge.expires_at_ms || challenge.generation != 1 {
            return Err(refused());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let owner: Option<String> = tx
            .query_row(
                "SELECT machine_id FROM machine WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(database)?;
        if owner.as_deref() != Some(&challenge.owner) {
            return Err(refused());
        }
        let previous: Option<(Vec<u8>, Vec<u8>, Vec<u8>)> = tx.query_row(
            "SELECT digest,receipt,proof FROM cloud_pair_receipts WHERE offer=?1 AND deadline_ms>?2",
            params![challenge.offer, now as i64], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional().map_err(database)?;
        if let Some((digest, bytes, proof)) = previous {
            if digest.as_slice() != verified.digest() {
                return Err(refused());
            }
            return Ok(PairReceipt {
                bytes,
                proof: proof
                    .try_into()
                    .map_err(|_| database("invalid pairing receipt proof"))?,
            });
        }
        let grant = Grant {
            client_id: candidate.peer.clone(),
            public_key: candidate.public_key,
            kind: "cli".into(),
            origin: "cli".into(),
            name: candidate.name.clone(),
            agents: "all".into(),
            scopes: DEFAULT_SCOPES.map(str::to_owned).into(),
            mode: "direct".into(),
            issued_at_ms: now,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        };
        insert_grant_in(&tx, &grant)?;
        let grant = if talk {
            crate::store::talk_in(&tx, &grant.client_id, true)?.ok_or_else(refused)?
        } else {
            grant
        };
        use sha2::{Digest, Sha256};
        let mut token = super::wire::reroute_token(response_key, challenge, &candidate.peer)?;
        let hash: [u8; 32] = Sha256::digest(token).into();
        let mut receipt: serde_json::Value = serde_json::from_slice(&crate::pairing::receipt(
            &grant,
            &challenge.owner,
            &challenge.owner_key,
        )?)
        .map_err(database)?;
        receipt["rerouteToken"] = serde_json::json!(crate::canonical::base64url(&token));
        token.fill(0);
        let bytes = serde_json::to_vec(&receipt).map_err(database)?;
        let proof = crypto::server_proof(response_key, &bytes).map_err(|_| refused())?;
        tx.execute(
            "INSERT INTO cloud_peers(project,client_id,owner,owner_key,generation,reroute_hash) VALUES(?1,?2,?3,?4,1,?5)",
            params![
                challenge.project,
                candidate.peer,
                challenge.owner,
                challenge.owner_key.as_slice(),
                hash.as_slice()
            ],
        )
        .map_err(database)?;
        tx.execute(
            "INSERT INTO cloud_pair_receipts VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                challenge.offer,
                verified.digest().as_slice(),
                challenge.expires_at_ms as i64,
                bytes,
                serde_json::to_string(challenge).map_err(database)?,
                proof.as_slice()
            ],
        )
        .map_err(database)?;
        tx.execute(
            "INSERT INTO cloud_peer_routes VALUES(?1,?2,?3,1,0,1)",
            params![challenge.project, candidate.uid, candidate.peer],
        )
        .map_err(database)?;
        if let Some(mut record) = reciprocal {
            if record.version != 1
                || !record.request.is_empty()
                || record.owner != candidate.peer
                || record.owner_key != candidate.public_key
                || record.peer != challenge.owner
                || !record.confirmed
                || record.config["projectId"] != challenge.project
                || !super::uid_ok(&record.uid)
                || !crate::remote_catalog::name_ok(&record.name)
            {
                return Err(refused());
            }
            let old: Option<String> = tx
                .query_row(
                    "SELECT connection FROM cloud_remotes WHERE owner=?1",
                    [&record.owner],
                    |row| row.get(0),
                )
                .optional()
                .map_err(database)?;
            if let Some(old) = old {
                let old: Connection = serde_json::from_str(&old).map_err(database)?;
                if old.owner_key != record.owner_key || old.peer != record.peer {
                    return Err(refused());
                }
            } else {
                let occupied: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM cloud_remotes WHERE name=?1)",
                        [&record.name],
                        |row| row.get(0),
                    )
                    .map_err(database)?;
                if occupied {
                    record.name = record.owner.clone();
                }
                record.reroute_token = Some(super::wire::reverse_reroute_token(
                    response_key,
                    challenge,
                    &candidate.peer,
                )?);
                tx.execute(
                    "INSERT INTO cloud_remotes VALUES(?1,?2,?3)",
                    params![
                        record.name,
                        record.owner,
                        serde_json::to_string(&record).map_err(database)?
                    ],
                )
                .map_err(database)?;
            }
        }
        crate::audit::append(
            &tx,
            crate::audit::AuditMetadata {
                time: now,
                client: &candidate.peer,
                request: &challenge.offer,
                operation: "remote.pair",
                operation_id: None,
                resources: std::slice::from_ref(&candidate.peer),
                digest: verified.digest(),
                revision: grant.revision,
                decision: "committed",
                code: "",
            },
        )?;
        tx.commit().map_err(database)?;
        Ok(PairReceipt { bytes, proof })
    }

    /// Reserve before signing a new one-shot reroute. Gaps are harmless; the
    /// receiver accepts any strictly greater generation, never a replay.
    pub fn next_cloud_reroute_generation(&mut self, machine: &str) -> Result<u64, RemoteError> {
        crate::canonical::uuid(machine).map_err(|_| refused())?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let previous: i64 = tx
            .query_row(
                "SELECT generation FROM cloud_reroute_counters WHERE machine_id=?1",
                [machine],
                |r| r.get(0),
            )
            .optional()
            .map_err(database)?
            .unwrap_or(1);
        let next = previous
            .checked_add(1)
            .filter(|n| *n > 1 && *n < 1 << 53)
            .ok_or_else(refused)?;
        tx.execute("INSERT INTO cloud_reroute_counters VALUES(?1,?2) ON CONFLICT(machine_id) DO UPDATE SET generation=excluded.generation", params![machine,next]).map_err(database)?;
        tx.commit().map_err(database)?;
        Ok(next as u64)
    }

    /// Recheck live authority, the owner pin and increasing generation under
    /// the writer fence. Rerouting never changes the sending grant or allowlist.
    pub fn apply_cloud_reroute(
        &mut self,
        project: &str,
        verified: &VerifiedReroute,
        now: u64,
    ) -> Result<(), RemoteError> {
        let request = verified.request();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let grant = live(&tx, &request.peer, now)?;
        let saved: Option<(String, Vec<u8>, i64)> = tx.query_row(
            "SELECT owner,owner_key,generation FROM cloud_peers WHERE project=?1 AND client_id=?2",
            params![project,request.peer], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional().map_err(database)?;
        let Some((owner, owner_key, generation)) = saved else {
            return Err(refused());
        };
        if owner != request.machine_id
            || owner_key != request.owner_key
            || request.generation as i64 <= generation
            || grant.public_key != verified.public_key
        {
            return Err(refused());
        }
        // A uid belonging to another native peer cannot be overwritten, even
        // when revoked: its history is needed for complete revocation cleanup.
        let occupied: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM cloud_peer_routes WHERE project=?1 AND uid=?2 AND client_id!=?3)", params![project,request.uid,request.peer], |r| r.get(0)).map_err(database)?;
        if occupied {
            return Err(refused());
        }
        tx.execute("UPDATE cloud_peer_routes SET revoked=1,generation=?3,pending=1 WHERE project=?1 AND client_id=?2 AND revoked=0", params![project,request.peer,request.generation as i64]).map_err(database)?;
        tx.execute("INSERT INTO cloud_peer_routes VALUES(?1,?2,?3,?4,0,1) ON CONFLICT(project,uid) DO UPDATE SET generation=excluded.generation,revoked=0,pending=1", params![project,request.uid,request.peer,request.generation as i64]).map_err(database)?;
        tx.execute(
            "UPDATE cloud_peers SET generation=?3 WHERE project=?1 AND client_id=?2",
            params![project, request.peer, request.generation as i64],
        )
        .map_err(database)?;
        tx.commit().map_err(database)
    }

    /// Pending desired rows survive provider failure and process restart.
    pub fn cloud_route_changes(&self) -> Result<Vec<RouteChange>, RemoteError> {
        self.connection.prepare("SELECT r.project,r.uid,r.client_id,r.generation,r.revoked,p.reroute_hash FROM cloud_peer_routes r JOIN cloud_peers p ON p.project=r.project AND p.client_id=r.client_id WHERE r.pending=1 ORDER BY r.project,r.client_id,r.uid")
            .and_then(|mut query| query.query_map([], |r| Ok(RouteChange {
                project:r.get(0)?, uid:r.get(1)?, peer:r.get(2)?, generation:r.get::<_, i64>(3)? as u64, revoked:r.get(4)?, reroute_hash:r.get(5)?,
            }))?.collect()).map_err(database)
    }

    pub fn cloud_route_published(&self, row: &RouteChange) -> Result<(), RemoteError> {
        self.connection.execute("UPDATE cloud_peer_routes SET pending=0 WHERE project=?1 AND uid=?2 AND client_id=?3 AND generation=?4 AND revoked=?5",
            params![row.project,row.uid,row.peer,row.generation as i64,row.revoked]).map_err(database)?;
        Ok(())
    }
}
