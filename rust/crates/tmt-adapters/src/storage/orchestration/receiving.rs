//! Publication of the configured Digest input, inside the canonical SQL owner.
use super::*;
use crate::orchestration::DigestReceivingSettings;

pub(super) fn read(
    db: &Connection,
) -> Result<Option<(u64, DigestReceivingSettings)>, StorageError> {
    let row: Option<(i64, String)> = db
        .query_row(
            "SELECT revision,settings FROM digest_receiving_settings WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| classify(e, "Read configured Digest receiving"))?;
    row.map(|(rev, doc)| {
        if doc.len() > 131_072 {
            return Err(corrupt());
        }
        let mut settings: DigestReceivingSettings =
            serde_json::from_str(&doc).map_err(|_| corrupt())?;
        settings.normalize().map_err(|_| corrupt())?;
        let revision = unsigned(rev)?;
        if revision == 0 {
            return Err(corrupt());
        }
        Ok((revision, settings))
    })
    .transpose()
}
impl Storage {
    pub fn sync_digest_receiving(
        &mut self,
        expected: u64,
        mut settings: DigestReceivingSettings,
    ) -> Result<u64, crate::orchestration::OrchReadFailure> {
        settings.normalize()?;
        if expected > MAX_JS_SAFE_INTEGER {
            return Err(OrchError::Invalid.into());
        }
        let tx = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| classify(e, "Begin configured Digest publication"))?;
        let previous = read(&tx)?;
        let revision = previous.as_ref().map_or(0, |(rev, _)| *rev);
        if expected != revision {
            return Err(OrchError::Conflict.into());
        }
        let next = if previous.as_ref().is_some_and(|(_, old)| *old == settings) {
            revision
        } else {
            let next = revision
                .checked_add(1)
                .filter(|n| *n <= MAX_JS_SAFE_INTEGER)
                .ok_or(OrchError::Limit)?;
            let document = serde_json::to_string(&settings).map_err(|_| OrchError::Invalid)?;
            tx.execute("INSERT INTO digest_receiving_settings(id,revision,settings) VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,settings=excluded.settings",
                params![next as i64,document]).map_err(|e|classify(e,"Publish configured Digest receiving"))?;
            next
        };
        tx.commit()
            .map_err(|e| classify(e, "Finish configured Digest publication"))?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        orchestration::{DigestReceiving, DigestReceivingMember},
        test_support::TestDirectory,
    };
    use tmt_core::identity::{Lifetime, create_or_resolve};
    #[test]
    fn publication_is_cas_idempotent_and_default_applies_to_later_identities() {
        let d = TestDirectory::new();
        let mut s = Storage::open(d.path.join("state.db")).unwrap();
        let settings = DigestReceivingSettings {
            enabled: true,
            default_receiving: DigestReceiving::Batch { interval_ms: 1501 },
            members: vec![],
        };
        assert_eq!(s.sync_digest_receiving(0, settings.clone()).unwrap(), 1);
        let cursor = s
            .connection()
            .unwrap()
            .query_row("SELECT value FROM change_cursor", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(s.sync_digest_receiving(1, settings.clone()).unwrap(), 1);
        assert_eq!(
            s.connection()
                .unwrap()
                .query_row("SELECT value FROM change_cursor", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            cursor
        );
        assert!(matches!(
            s.sync_digest_receiving(0, settings.clone()),
            Err(crate::orchestration::OrchReadFailure::Policy(
                OrchError::Conflict
            ))
        ));
        let id = create_or_resolve(&mut s, "later-seat", Lifetime::Saved)
            .unwrap()
            .identity
            .id;
        let snapshot = s.orchestration_snapshot(None).unwrap().snapshot;
        assert_eq!(
            snapshot.base_receiving[&id].receiving,
            OrchReceiving::Batch(1501)
        );
        assert_eq!(snapshot.digest_revision, 1);
        let mut override_settings = settings;
        override_settings.enabled = false;
        override_settings.members.push(DigestReceivingMember {
            identity_id: id.clone(),
            receiving: DigestReceiving::Auto {},
        });
        assert_eq!(s.sync_digest_receiving(1, override_settings).unwrap(), 2);
        let snapshot = s.orchestration_snapshot(None).unwrap().snapshot;
        assert!(!snapshot.digest_enabled);
        assert_eq!(snapshot.base_receiving[&id].receiving, OrchReceiving::Auto);
    }
}
