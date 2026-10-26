//! Join-side adoption consumes the exact original submission in one Store
//! transaction. The shared attempt authenticates the receipt before this scoped
//! callback; no code or raw response key crosses this boundary.
use super::{
    Connection,
    store::{self, PeerRoutes, RouteAdoption},
    wire,
};
use crate::{
    canonical,
    error::RemoteError,
    invitations::{
        Context,
        port::{ReceiptKey, Submission},
        service::check,
    },
    objects::IoBudget,
    store::{DEFAULT_SCOPES, Grant, Store, database, grant_in},
};
use rusqlite::{TransactionBehavior, params};
use sha2::{Digest, Sha256};

/// Temporary capabilities clear on every exit; the admitted Connection owns the
/// deliberately retained forward capability after successful adoption.
struct RerouteSecrets {
    forward: [u8; 32],
    reverse: [u8; 32],
}
impl Drop for RerouteSecrets {
    fn drop(&mut self) {
        self.forward.fill(0);
        self.reverse.fill(0);
    }
}

pub struct ReceiptAdoption<'a> {
    pub context: &'a Context,
    pub submission: &'a Submission,
    pub receipt: &'a [u8],
    pub key: &'a ReceiptKey,
    pub connection: &'a Connection,
    pub owner_name: &'a str,
    /// The trusted local reverse-permission answer, never a wire field.
    pub enable_sending: bool,
}
impl Store {
    pub fn adopt_cloud_receipt(
        &mut self,
        input: ReceiptAdoption<'_>,
        io: &IoBudget<'_>,
    ) -> Result<Connection, RemoteError> {
        check(io)?;
        let ReceiptAdoption {
            context,
            submission,
            receipt,
            key,
            connection,
            owner_name,
            enable_sending,
        } = input;
        wire::v2::validate_context(context)?;
        let candidate = &submission.candidate;
        let project =
            std::str::from_utf8(&context.binding.reference).map_err(|_| wire::invalid())?;
        let owner_uid =
            std::str::from_utf8(&context.binding.owner_principal).map_err(|_| wire::invalid())?;
        let request = wire::v2::submission(context, submission)?;
        let body = crate::wire::strict_json(receipt).ok_or_else(wire::invalid)?;
        let secrets = {
            let (forward, reverse) = wire::v2::reroute_tokens(context, candidate, key)?;
            RerouteSecrets { forward, reverse }
        };
        let receipt_token = body["rerouteToken"]
            .as_str()
            .ok_or_else(wire::missing_reroute_token)?;
        let token_matches = canonical::base64url_bytes(receipt_token, 32)
            .is_ok_and(|value| value.as_slice() == secrets.forward);
        let reverse_hash: [u8; 32] = Sha256::digest(secrets.reverse.as_slice()).into();
        if !token_matches
            || receipt.len() > crate::limits::CLOUD_ENROLLMENT_BYTES
            || body["grant"]["clientId"] != candidate.machine
            || body["grant"]["machineId"] != context.owner
            || body["machinePublicKey"] != canonical::base64url(&context.owner_key)
            || body["grant"]["publicKey"] != canonical::base64url(&candidate.public_key)
            || connection.owner != context.owner
            || connection.owner_key != context.owner_key
            || connection.peer != candidate.machine
            || connection.config["projectId"] != project
            || connection.uid.as_bytes() != candidate.principal
            || !canonical::device_name(owner_name)
            || connection.version != 1
            || !super::uid_ok(&connection.uid)
        {
            return Err(wire::invalid());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        check(io)?;
        let now = crate::pairing::now_ms()?;
        if now >= context.expires_at_ms || crate::store::machine_in(&tx)?.id != candidate.machine {
            return Err(wire::expired());
        }
        let previous =
            store::cloud_connection_in(&tx, &context.owner)?.ok_or_else(wire::invalid)?;
        if previous.owner != context.owner {
            return Err(wire::invalid());
        }
        // A completed original can be observed again, but never restores policy,
        // revoked routes or another member after its claim has been consumed.
        if previous.pending_submission.is_none()
            && previous.confirmed
            && previous.request == request
            && previous.owner_key == context.owner_key
            && previous.peer == candidate.machine
        {
            return Ok(previous);
        }
        let claimed = previous
            .pending_submission
            .as_ref()
            .ok_or_else(wire::invalid)?;
        if claimed.context != *context
            || claimed.request != request
            || previous.peer != candidate.machine
        {
            return Err(wire::invalid());
        }
        if let Some(old) = grant_in(&tx, &context.owner)?.filter(|grant| !grant.disabled) {
            if old.public_key != context.owner_key
                || old.kind != "cli"
                || old.origin != "cli"
                || old.expires_at_ms.is_some_and(|expiry| now >= expiry)
            {
                return Err(wire::invalid());
            }
        } else {
            let grant = Grant {
                client_id: context.owner.clone(),
                public_key: context.owner_key,
                kind: "cli".into(),
                origin: "cli".into(),
                name: owner_name.into(),
                agents: "all".into(),
                scopes: DEFAULT_SCOPES.map(str::to_owned).into(),
                mode: "direct".into(),
                issued_at_ms: now,
                expires_at_ms: None,
                revision: 1,
                disabled: false,
            };
            crate::members::confirm_member_grant_in(&tx, &grant)?;
            if enable_sending {
                crate::members::sending_for_client(&tx, &context.owner, true)?
                    .ok_or_else(wire::invalid)?;
            }
        }
        store::adopt_cloud_routes_in(
            &tx,
            PeerRoutes {
                project,
                owner: &candidate.machine,
                owner_key: &candidate.public_key,
                peer: &context.owner,
                peer_key: &context.owner_key,
                uid: owner_uid,
                reroute_hash: &reverse_hash,
                reciprocal: None,
            },
            RouteAdoption::NewMember,
        )?;
        tx.execute("INSERT INTO cloud_owner_projects(project,config) VALUES(?1,?2) ON CONFLICT(project) DO UPDATE SET config=excluded.config",params![project,connection.config.to_string()]).map_err(database)?;
        let mut confirmed = connection.clone();
        confirmed.name = previous.name;
        confirmed.request = request;
        confirmed.confirmed = true;
        confirmed.reroute_token = Some(secrets.forward);
        confirmed.pending_submission = None;
        // This is the sole consuming writer; ordinary snapshot saves retain it.
        store::write_cloud_connection_in(&tx, &confirmed)?;
        check(io)?;
        if crate::pairing::now_ms()? >= context.expires_at_ms {
            return Err(wire::expired());
        }
        tx.commit().map_err(database)?;
        Ok(confirmed)
    }
}
