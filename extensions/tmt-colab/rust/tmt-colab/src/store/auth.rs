//! Durable sign-in consumption and hashed sessions. No secret code/token is stored.
//! Caller proof/certificate admission precedes this private transaction boundary.
use super::{Fault, Store, StoreResult};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

pub(crate) struct Enrollment<'a> {
    pub code: &'a str,
    pub space: &'a str,
    pub device: &'a str,
    pub signing_key: &'a [u8; 32],
    pub encryption_key: &'a [u8; 32],
    pub certificate: &'a [u8],
    pub token_hash: &'a [u8; 32],
    pub now: u64,
    pub expires: u64,
}
pub(crate) struct OwnerGenesis<'a> {
    pub space: &'a str,
    pub envelope: &'a [u8],
    pub head: &'a tmt_colab_model::statement::Head,
}
impl Store {
    /// Called once after acquiring the foreground serve lock. Restart invalidates old codes.
    pub(crate) fn start_auth(&mut self) -> StoreResult<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch("
            CREATE TABLE IF NOT EXISTS owner_binding(space TEXT PRIMARY KEY, member TEXT NOT NULL, sign_key BLOB NOT NULL, enc_key BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS membership(revision TEXT PRIMARY KEY, envelope BLOB NOT NULL, hash BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS signin_codes(code TEXT PRIMARY KEY, space TEXT NOT NULL, expires INTEGER NOT NULL,
                consumed INTEGER NOT NULL DEFAULT 0, active INTEGER NOT NULL DEFAULT 1);
            CREATE TABLE IF NOT EXISTS devices(device TEXT PRIMARY KEY, sign_key BLOB NOT NULL,
                enc_key BLOB NOT NULL, certificate BLOB NOT NULL, revoked INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS sessions(token_hash BLOB PRIMARY KEY, space TEXT NOT NULL, device TEXT NOT NULL,
                expires INTEGER NOT NULL, FOREIGN KEY(device) REFERENCES devices(device));
            UPDATE signin_codes SET active=0 WHERE consumed=0;
            PRAGMA user_version=2;
            ")?;
        transaction.commit()?;
        Ok(())
    }
    pub(crate) fn initialize_owner(&mut self, genesis: OwnerGenesis<'_>) -> StoreResult<()> {
        let OwnerGenesis {
            space,
            envelope,
            head,
        } = genesis;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let pinned: Option<(String, Vec<u8>, Vec<u8>)> = transaction
            .query_row(
                "SELECT member,sign_key,enc_key FROM owner_binding WHERE space=?",
                [space],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let owner = &head.owner_member;
        if let Some((id, sign, enc)) = pinned {
            if id != owner.id || sign != owner.signing_key || enc != owner.encryption_key {
                return Err(Fault::Conflict);
            }
            let retained: (Vec<u8>, Vec<u8>) = transaction.query_row(
                "SELECT envelope,hash FROM membership WHERE revision='00000000000000000001'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if retained.0 != envelope || retained.1 != head.hash {
                return Err(Fault::Conflict);
            }
        } else {
            transaction.execute(
                "INSERT INTO owner_binding VALUES (?,?,?,?)",
                params![space, owner.id, owner.signing_key, owner.encryption_key],
            )?;
            transaction.execute(
                "INSERT INTO membership VALUES (?,?,?)",
                params![format!("{:020}", head.revision), envelope, head.hash],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
    pub(crate) fn membership_head(
        &self,
        space: &str,
    ) -> StoreResult<tmt_colab_model::statement::Head> {
        let (revision,hash,id,sign,enc):(String,Vec<u8>,String,Vec<u8>,Vec<u8>)=self.connection.query_row(
            "SELECT revision,hash,member,sign_key,enc_key FROM membership CROSS JOIN owner_binding WHERE space=? ORDER BY revision DESC LIMIT 1",[space],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        )?;
        Ok(tmt_colab_model::statement::Head {
            revision: revision.parse().map_err(|_| Fault::Invalid)?,
            hash: hash.try_into().map_err(|_| Fault::Invalid)?,
            owner_member: tmt_colab_model::statement::OwnerMember {
                id,
                signing_key: sign.try_into().map_err(|_| Fault::Invalid)?,
                encryption_key: enc.try_into().map_err(|_| Fault::Invalid)?,
            },
        })
    }
    pub(crate) fn issue_signin(
        &self,
        code: &str,
        space: &str,
        now: u64,
        expires: u64,
    ) -> StoreResult<()> {
        if expires <= now || expires - now > 600_000 || expires > (1u64 << 53) - 1 {
            return Err(Fault::Invalid);
        }
        self.connection.execute(
            "INSERT INTO signin_codes(code,space,expires) VALUES (?,?,?)",
            params![code, space, millis(expires)?],
        )?;
        Ok(())
    }
    /// Only the verified sign-in owner can call this; a failed transaction consumes nothing.
    pub(crate) fn enroll(&mut self, enrollment: Enrollment<'_>) -> StoreResult<()> {
        let v = enrollment;
        if v.expires <= v.now || v.expires - v.now > 86_400_000 || v.expires > (1u64 << 53) - 1 {
            return Err(Fault::Invalid);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute(
            "UPDATE signin_codes SET consumed=1 WHERE code=? AND space=? AND active=1 AND consumed=0 AND expires>?",
            params![v.code, v.space, millis(v.now)?],
        )? != 1 {
            return Err(Fault::Invalid);
        }
        transaction.execute(
            "INSERT INTO devices(device,sign_key,enc_key,certificate) VALUES (?,?,?,?)",
            params![v.device, v.signing_key, v.encryption_key, v.certificate],
        )?;
        transaction.execute(
            "INSERT INTO sessions(token_hash,space,device,expires) VALUES (?,?,?,?)",
            params![v.token_hash, v.space, v.device, millis(v.expires)?],
        )?;
        transaction.commit()?;
        Ok(())
    }
    pub(crate) fn session(
        &self,
        token_hash: &[u8; 32],
        space: &str,
        now: u64,
    ) -> StoreResult<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT s.device FROM sessions s JOIN devices d USING(device)
             WHERE s.token_hash=? AND s.space=? AND s.expires>? AND d.revoked=0",
                params![token_hash, space, millis(now)?],
                |row| row.get(0),
            )
            .optional()?)
    }
    /// The caller applies an already-verified device.revoke owner statement.
    pub fn revoke_device(&mut self, device: &str) -> StoreResult<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute(
            "UPDATE devices SET revoked=1 WHERE device=? AND revoked=0",
            [device],
        )? != 1
        {
            return Err(Fault::Invalid);
        }
        transaction.execute("DELETE FROM sessions WHERE device=?", [device])?;
        transaction.commit()?;
        Ok(())
    }
}

fn millis(value: u64) -> StoreResult<i64> {
    if value > (1u64 << 53) - 1 {
        return Err(Fault::Invalid);
    }
    Ok(value as i64)
}

#[cfg(test)]
mod tests;
