//! UUID-owned local Office presentation profile persistence and CAS policy.

use rusqlite::{OptionalExtension, params};
use tmt_office_model::office_profile::{LocalProfile, MAX_REVISION, deterministic_default};

use super::{
    Storage, StorageError, StorageErrorCode, errors::classify,
    identities::with_immediate_transaction,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalProfileSnapshot {
    pub identity_id: String,
    pub identity_name: String,
    pub exists: bool,
    pub revision: u64,
    pub profile: LocalProfile,
    pub updated_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalProfileMutation {
    pub snapshot: LocalProfileSnapshot,
    pub changed: bool,
}

#[derive(Debug)]
pub enum LocalProfileError {
    Storage(StorageError),
    IdentityInactive,
    RevisionConflict,
    RevisionExhausted,
    ProfileInvalid,
    AvatarUnavailable,
    StoredProfileInvalid,
}

impl From<StorageError> for LocalProfileError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

impl std::fmt::Display for LocalProfileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Storage(_) => "Could not access local Office profile storage.",
            Self::IdentityInactive => "The selected identity is not active.",
            Self::RevisionConflict => "The local Office profile changed.",
            Self::RevisionExhausted => "The local Office profile revision is exhausted.",
            Self::ProfileInvalid => "The local Office profile is invalid.",
            Self::AvatarUnavailable => "The selected local Office avatar is unavailable.",
            Self::StoredProfileInvalid => "The stored local Office profile is invalid.",
        })
    }
}
impl std::error::Error for LocalProfileError {}

impl Storage {
    pub fn show_local_profile(
        &self,
        identity_id: &str,
    ) -> Result<LocalProfileSnapshot, LocalProfileError> {
        let connection = self.connection()?;
        let identity_name = connection
            .query_row(
                "SELECT name FROM identities WHERE id = ? AND retired_at_ms IS NULL",
                [identity_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| classify(error, "Read local Office profile identity"))?
            .ok_or(LocalProfileError::IdentityInactive)?;
        let stored = connection.query_row(
            "SELECT revision, profile, updated_at_ms FROM office_local_profiles WHERE identity_id = ?",
            [identity_id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
        ).optional().map_err(|error| classify(error, "Read local Office profile"))?;
        snapshot(identity_id, identity_name, stored)
    }

    pub fn list_active_local_profiles(
        &self,
    ) -> Result<Vec<LocalProfileSnapshot>, LocalProfileError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT i.id, i.name, p.revision, p.profile, p.updated_at_ms FROM identities i \
             LEFT JOIN office_local_profiles p ON p.identity_id = i.id \
             WHERE i.retired_at_ms IS NULL ORDER BY i.canonical_name COLLATE BINARY",
            )
            .map_err(|error| classify(error, "Prepare active local Office profiles"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            })
            .map_err(|error| classify(error, "List active local Office profiles"))?;
        rows.map(|row| {
            let (id, name, revision, profile, updated) =
                row.map_err(|error| classify(error, "Read active local Office profile"))?;
            snapshot(
                &id,
                name,
                revision
                    .zip(profile)
                    .zip(updated)
                    .map(|((r, p), u)| (r, p, u)),
            )
        })
        .collect()
    }

    pub fn apply_local_profile(
        &mut self,
        identity_id: &str,
        expected_revision: u64,
        profile: &LocalProfile,
    ) -> Result<LocalProfileMutation, LocalProfileError> {
        profile
            .validate()
            .map_err(|_| LocalProfileError::ProfileInvalid)?;
        let encoded =
            tmt_office_model::codec::office_profile_wire::encode_value(profile).to_string();
        with_immediate_transaction(self, "local Office profile", |transaction| {
            let identity_name = transaction
                .query_row(
                    "SELECT name FROM identities WHERE id = ? AND retired_at_ms IS NULL",
                    [identity_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| classify(error, "Revalidate local Office profile identity"))?
                .ok_or(LocalProfileError::IdentityInactive)?;
            let current = transaction.query_row(
                "SELECT revision, profile, updated_at_ms FROM office_local_profiles WHERE identity_id = ?", [identity_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
            ).optional().map_err(|error| classify(error, "Read local Office profile revision"))?;
            match current {
                None if expected_revision == 0 => {
                    admit_avatar_reference(transaction, profile.avatar_ref.as_deref(), None)?;
                    let now = current_time_ms()?;
                    transaction.execute(
                        "INSERT INTO office_local_profiles (identity_id, revision, profile, updated_at_ms) VALUES (?, 1, ?, ?)",
                        params![identity_id, encoded, i64::try_from(now).expect("timestamp fits SQLite")],
                    ).map_err(|error| classify(error, "Create local Office profile"))?;
                    Ok(LocalProfileMutation {
                        snapshot: LocalProfileSnapshot {
                            identity_id: identity_id.into(),
                            identity_name,
                            exists: true,
                            revision: 1,
                            profile: profile.clone(),
                            updated_at_ms: Some(now),
                        },
                        changed: true,
                    })
                }
                None => Err(LocalProfileError::RevisionConflict),
                Some((revision, stored, updated))
                    if expected_revision.checked_add(1) == Some(stored_u64(revision)?)
                        && stored == encoded =>
                {
                    Ok(LocalProfileMutation {
                        snapshot: LocalProfileSnapshot {
                            identity_id: identity_id.into(),
                            identity_name,
                            exists: true,
                            revision: stored_u64(revision)?,
                            profile: profile.clone(),
                            updated_at_ms: Some(stored_u64(updated)?),
                        },
                        changed: false,
                    })
                }
                Some((revision, stored, updated))
                    if stored_u64(revision)? == expected_revision && stored == encoded =>
                {
                    Ok(LocalProfileMutation {
                        snapshot: LocalProfileSnapshot {
                            identity_id: identity_id.into(),
                            identity_name,
                            exists: true,
                            revision: stored_u64(revision)?,
                            profile: profile.clone(),
                            updated_at_ms: Some(stored_u64(updated)?),
                        },
                        changed: false,
                    })
                }
                Some((revision, _, _)) if stored_u64(revision)? != expected_revision => {
                    Err(LocalProfileError::RevisionConflict)
                }
                Some((revision, stored, _)) if stored_u64(revision)? < MAX_REVISION => {
                    let current_profile = decode_stored_profile(&stored)?;
                    admit_avatar_reference(
                        transaction,
                        profile.avatar_ref.as_deref(),
                        current_profile.avatar_ref.as_deref(),
                    )?;
                    let now = current_time_ms()?;
                    let revision = stored_u64(revision)?;
                    let next = revision + 1;
                    transaction.execute(
                        "UPDATE office_local_profiles SET revision = ?, profile = ?, updated_at_ms = ? WHERE identity_id = ? AND revision = ? AND EXISTS (SELECT 1 FROM identities WHERE id = ? AND retired_at_ms IS NULL)",
                        params![i64::try_from(next).unwrap(), encoded, i64::try_from(now).unwrap(), identity_id, i64::try_from(revision).unwrap(), identity_id],
                    ).map_err(|error| classify(error, "Update local Office profile"))?;
                    Ok(LocalProfileMutation {
                        snapshot: LocalProfileSnapshot {
                            identity_id: identity_id.into(),
                            identity_name,
                            exists: true,
                            revision: next,
                            profile: profile.clone(),
                            updated_at_ms: Some(now),
                        },
                        changed: true,
                    })
                }
                Some(_) => Err(LocalProfileError::RevisionExhausted),
            }
        })
    }
}

fn admit_avatar_reference(
    transaction: &rusqlite::Transaction<'_>,
    candidate: Option<&str>,
    current: Option<&str>,
) -> Result<(), LocalProfileError> {
    if candidate == current {
        return Ok(());
    }
    if let Some(value) = candidate
        && !super::office_avatar::reference_available(transaction, value)?
    {
        return Err(LocalProfileError::AvatarUnavailable);
    }
    Ok(())
}

fn decode_stored_profile(encoded: &str) -> Result<LocalProfile, LocalProfileError> {
    serde_json::from_str(encoded)
        .map_err(|_| LocalProfileError::StoredProfileInvalid)
        .and_then(|value| {
            tmt_office_model::codec::office_profile_wire::decode_value(value)
                .map_err(|_| LocalProfileError::StoredProfileInvalid)
        })
}

fn snapshot(
    identity_id: &str,
    identity_name: String,
    stored: Option<(i64, String, i64)>,
) -> Result<LocalProfileSnapshot, LocalProfileError> {
    match stored {
        None => Ok(LocalProfileSnapshot {
            identity_id: identity_id.into(),
            identity_name,
            exists: false,
            revision: 0,
            profile: deterministic_default(identity_id)
                .map_err(|_| LocalProfileError::StoredProfileInvalid)?,
            updated_at_ms: None,
        }),
        Some((revision, encoded, updated)) => {
            let profile = decode_stored_profile(&encoded)?;
            Ok(LocalProfileSnapshot {
                identity_id: identity_id.into(),
                identity_name,
                exists: true,
                revision: stored_u64(revision)?,
                profile,
                updated_at_ms: Some(stored_u64(updated)?),
            })
        }
    }
}

fn stored_u64(value: i64) -> Result<u64, LocalProfileError> {
    value
        .try_into()
        .map_err(|_| LocalProfileError::StoredProfileInvalid)
}
fn current_time_ms() -> Result<u64, LocalProfileError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            StorageError::new(StorageErrorCode::Unknown, "Read system clock failed")
                .caused_by(error)
        })?
        .as_millis()
        .try_into()
        .map_err(|error| {
            LocalProfileError::Storage(
                StorageError::new(StorageErrorCode::Unknown, "System timestamp is too large")
                    .caused_by(error),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;

    fn identity(storage: &Storage, id: &str, name: &str) {
        storage.connection().unwrap().execute(
            "INSERT INTO identities (id, name, canonical_name, lifetime, created_at, updated_at) VALUES (?, ?, lower(?), 'saved', 'now', 'now')",
            params![id, name, name],
        ).unwrap();
    }

    fn avatar_pack() -> tmt_office_model::codec::office_avatar::ValidatedAvatarPack {
        tmt_office_model::codec::office_avatar::validate_pack(include_bytes!(
            "../../../../../extensions/tmt-office/contracts/avatar-pack-v1-sample.tmtavatar.json"
        ))
        .unwrap()
    }

    fn avatar_ref() -> String {
        format!("{}/signal-bot", avatar_pack().digest())
    }

    #[test]
    fn default_is_virtual_and_cas_preserves_noop_and_exact_retry_timestamp() {
        let directory = TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("profile.db")).unwrap();
        let id = "01020304-1111-4111-8111-111111111111";
        identity(&storage, id, "Alice");
        storage.connection().unwrap().execute(
            "INSERT INTO bindings (id, identity_id, transport, pane_id, server_id, socket_path, server_pid, server_start_time, pane_pid, bound_at, last_verified_at) VALUES ('binding', ?, 'tmux', '%1', 'server', '/tmp/server.sock', 1, 'start', 2, 'now', 'now')",
            [id],
        ).unwrap();
        storage.connection().unwrap().execute(
            "INSERT INTO role_profiles (identity_id, content, updated_at) VALUES (?, 'architect', 'now')",
            [id],
        ).unwrap();
        let missing = storage.show_local_profile(id).unwrap();
        assert!(!missing.exists);
        assert_eq!(missing.revision, 0);
        assert_eq!(
            storage
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM office_local_profiles", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );

        let created = storage
            .apply_local_profile(id, 0, &missing.profile)
            .unwrap();
        assert!(created.changed);
        assert_eq!(created.snapshot.revision, 1);
        let created_at = created.snapshot.updated_at_ms;
        assert_eq!(
            storage
                .apply_local_profile(id, 0, &missing.profile)
                .unwrap(),
            LocalProfileMutation {
                changed: false,
                snapshot: created.snapshot.clone(),
            }
        );
        assert_eq!(
            storage
                .apply_local_profile(id, 1, &missing.profile)
                .unwrap(),
            LocalProfileMutation {
                changed: false,
                snapshot: created.snapshot.clone(),
            }
        );
        assert_eq!(created.snapshot.updated_at_ms, created_at);

        let mut changed = missing.profile.clone();
        changed.description = "Architecture review".into();
        let updated = storage.apply_local_profile(id, 1, &changed).unwrap();
        assert!(updated.changed);
        assert_eq!(updated.snapshot.revision, 2);
        assert_eq!(
            storage
                .connection()
                .unwrap()
                .query_row(
                    "SELECT content FROM role_profiles WHERE identity_id = ?",
                    [id],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "architect"
        );
        assert!(matches!(
            storage.apply_local_profile(id, 1, &missing.profile),
            Err(LocalProfileError::RevisionConflict)
        ));
        storage
            .connection()
            .unwrap()
            .execute(
                "UPDATE office_local_profiles SET revision = ? WHERE identity_id = ?",
                params![i64::try_from(MAX_REVISION).unwrap(), id],
            )
            .unwrap();
        let maximum = storage.show_local_profile(id).unwrap();
        assert_eq!(maximum.revision, MAX_REVISION);
        assert_eq!(
            storage
                .apply_local_profile(id, MAX_REVISION, &maximum.profile)
                .unwrap(),
            LocalProfileMutation {
                changed: false,
                snapshot: maximum,
            }
        );
        assert!(matches!(
            storage.apply_local_profile(id, MAX_REVISION, &missing.profile),
            Err(LocalProfileError::RevisionExhausted)
        ));
    }

    #[test]
    fn retirement_hides_but_retains_profile_and_same_name_replacement_inherits_nothing() {
        let directory = TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("retirement.db")).unwrap();
        let old = "11111111-1111-4111-8111-111111111111";
        identity(&storage, old, "Alice");
        let default = storage.show_local_profile(old).unwrap().profile;
        let avatar = avatar_pack();
        storage.install_local_avatar_pack(0, &avatar).unwrap();
        let mut selected = default.clone();
        selected.avatar_ref = Some(avatar_ref());
        storage.apply_local_profile(old, 0, &selected).unwrap();
        storage
            .connection()
            .unwrap()
            .execute(
                "UPDATE identities SET retired_at_ms = 1 WHERE id = ?",
                [old],
            )
            .unwrap();
        assert!(matches!(
            storage.show_local_profile(old),
            Err(LocalProfileError::IdentityInactive)
        ));
        assert_eq!(
            storage
                .connection()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM office_local_profiles WHERE identity_id = ?",
                    [old],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );

        let replacement = "22222222-2222-4222-8222-222222222222";
        identity(&storage, replacement, "Alice");
        let snapshot = storage.show_local_profile(replacement).unwrap();
        assert!(!snapshot.exists);
        assert_ne!(snapshot.identity_id, old);
        assert_eq!(snapshot.profile.avatar_ref, None);
        assert_eq!(
            storage.list_active_local_profiles().unwrap(),
            vec![snapshot]
        );
    }

    #[test]
    fn avatar_admission_retention_removal_and_reinstall_are_atomic_with_profile_cas() {
        let directory = TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("avatar-profile.db")).unwrap();
        let id = "33333333-3333-4333-8333-333333333333";
        identity(&storage, id, "Signal");
        let mut candidate = storage.show_local_profile(id).unwrap().profile;
        let missing = format!("sha256:{}/missing", "0".repeat(64));
        candidate.avatar_ref = Some(missing.clone());
        assert!(matches!(
            storage.apply_local_profile(id, 0, &candidate),
            Err(LocalProfileError::AvatarUnavailable)
        ));
        assert_eq!(
            storage
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM office_local_profiles", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );

        let avatar = avatar_pack();
        storage.install_local_avatar_pack(0, &avatar).unwrap();
        candidate.avatar_ref = Some(format!("{}/missing", avatar.digest()));
        assert!(matches!(
            storage.apply_local_profile(id, 0, &candidate),
            Err(LocalProfileError::AvatarUnavailable)
        ));
        candidate.avatar_ref = Some(avatar_ref());
        let selected = storage.apply_local_profile(id, 0, &candidate).unwrap();
        assert_eq!(selected.snapshot.revision, 1);

        storage
            .connection()
            .unwrap()
            .execute(
                "UPDATE office_avatar_packs SET bytes = ? WHERE digest = ?",
                params![b"{}".as_slice(), avatar.digest()],
            )
            .unwrap();
        let mut retained = selected.snapshot.profile.clone();
        retained.description = "Corrupt fallback remains editable".into();
        let retained = storage.apply_local_profile(id, 1, &retained).unwrap();
        assert_eq!(retained.snapshot.revision, 2);
        storage
            .remove_local_avatar_pack(1, avatar.digest())
            .unwrap();
        let mut missing_retained = retained.snapshot.profile.clone();
        missing_retained.description = "Missing fallback remains editable".into();
        let missing_retained = storage
            .apply_local_profile(id, 2, &missing_retained)
            .unwrap();
        assert_eq!(missing_retained.snapshot.revision, 3);

        let before_reinstall = storage.show_local_profile(id).unwrap();
        storage.install_local_avatar_pack(2, &avatar).unwrap();
        assert_eq!(storage.show_local_profile(id).unwrap(), before_reinstall);

        let mut unavailable_change = before_reinstall.profile.clone();
        unavailable_change.avatar_ref = Some(missing);
        assert!(matches!(
            storage.apply_local_profile(id, 3, &unavailable_change),
            Err(LocalProfileError::AvatarUnavailable)
        ));
        assert_eq!(storage.show_local_profile(id).unwrap(), before_reinstall);

        let mut reset = before_reinstall.profile.clone();
        reset.avatar_ref = None;
        let reset = storage.apply_local_profile(id, 3, &reset).unwrap();
        assert_eq!(reset.snapshot.revision, 4);
        assert_eq!(reset.snapshot.profile.avatar_ref, None);
        let reset_retry = storage
            .apply_local_profile(id, 3, &reset.snapshot.profile)
            .unwrap();
        assert!(!reset_retry.changed);
        assert_eq!(reset_retry.snapshot, reset.snapshot);
    }

    #[test]
    fn omitted_and_null_wire_resets_are_each_exactly_retryable() {
        for (case, explicit_null) in [("omitted", false), ("null", true)] {
            let directory = TestDirectory::new();
            let mut storage =
                Storage::open(directory.path.join(format!("reset-{case}.db"))).unwrap();
            let id = "44444444-4444-4444-8444-444444444444";
            identity(&storage, id, "Resettable");
            let avatar = avatar_pack();
            storage.install_local_avatar_pack(0, &avatar).unwrap();
            let mut selected = storage.show_local_profile(id).unwrap().profile;
            selected.avatar_ref = Some(avatar_ref());
            let selected = storage.apply_local_profile(id, 0, &selected).unwrap();

            let mut reset_wire = tmt_office_model::codec::office_profile_wire::encode_value(
                &selected.snapshot.profile,
            );
            if explicit_null {
                reset_wire["avatarRef"] = serde_json::Value::Null;
            } else {
                reset_wire.as_object_mut().unwrap().remove("avatarRef");
            }
            let reset_profile =
                tmt_office_model::codec::office_profile_wire::decode_value(reset_wire).unwrap();
            let reset = storage.apply_local_profile(id, 1, &reset_profile).unwrap();
            assert!(reset.changed, "{case}");
            assert_eq!(reset.snapshot.profile.avatar_ref, None, "{case}");
            let retry = storage.apply_local_profile(id, 1, &reset_profile).unwrap();
            assert!(!retry.changed, "{case}");
            assert_eq!(retry.snapshot, reset.snapshot, "{case}");
        }
    }

    #[test]
    fn catalog_removal_and_profile_adoption_have_one_ordered_transaction_boundary() {
        let directory = TestDirectory::new();
        let database = directory.path.join("avatar-profile-ordering.db");
        let mut catalog = Storage::open(&database).unwrap();
        let mut profiles = Storage::open(&database).unwrap();
        let id = "55555555-5555-4555-8555-555555555555";
        identity(&catalog, id, "Ordered");
        let avatar = avatar_pack();
        let avatar_ref = avatar_ref();

        catalog.install_local_avatar_pack(0, &avatar).unwrap();
        catalog
            .remove_local_avatar_pack(1, avatar.digest())
            .unwrap();
        let mut candidate = profiles.show_local_profile(id).unwrap().profile;
        candidate.avatar_ref = Some(avatar_ref.clone());
        assert!(matches!(
            profiles.apply_local_profile(id, 0, &candidate),
            Err(LocalProfileError::AvatarUnavailable)
        ));
        assert_eq!(
            profiles
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM office_local_profiles", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );

        catalog.install_local_avatar_pack(2, &avatar).unwrap();
        let adopted = profiles.apply_local_profile(id, 0, &candidate).unwrap();
        assert_eq!(adopted.snapshot.revision, 1);
        catalog
            .remove_local_avatar_pack(3, avatar.digest())
            .unwrap();
        assert_eq!(profiles.show_local_profile(id).unwrap(), adopted.snapshot);

        let retry = profiles.apply_local_profile(id, 0, &candidate).unwrap();
        assert!(!retry.changed);
        assert_eq!(retry.snapshot, adopted.snapshot);
        let noop = profiles.apply_local_profile(id, 1, &candidate).unwrap();
        assert!(!noop.changed);
        assert_eq!(noop.snapshot, adopted.snapshot);

        let mut retained = candidate;
        retained.description = "Retained after catalog removal".into();
        let retained = profiles.apply_local_profile(id, 1, &retained).unwrap();
        assert_eq!(retained.snapshot.revision, 2);
        assert_eq!(retained.snapshot.profile.avatar_ref, Some(avatar_ref));
    }

    #[test]
    fn legacy_stored_profile_decodes_without_rewriting_bytes_revision_or_timestamp() {
        let directory = TestDirectory::new();
        let storage = Storage::open(directory.path.join("legacy-profile.db")).unwrap();
        let id = "44444444-4444-4444-8444-444444444444";
        identity(&storage, id, "Legacy");
        let encoded = r#"{"displayLabel":"","description":"","appearance":{"hairStyle":"short","hairColor":"ink","skinTone":"medium","shirtColor":"blue","shirtMark":""}}"#;
        storage.connection().unwrap().execute(
            "INSERT INTO office_local_profiles (identity_id, revision, profile, updated_at_ms) VALUES (?, 7, ?, 99)",
            params![id, encoded],
        ).unwrap();
        let before = storage.connection().unwrap().query_row(
            "SELECT revision, profile, updated_at_ms FROM office_local_profiles WHERE identity_id = ?",
            [id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
        ).unwrap();
        let shown = storage.show_local_profile(id).unwrap();
        assert_eq!(shown.profile.avatar_ref, None);
        let after = storage.connection().unwrap().query_row(
            "SELECT revision, profile, updated_at_ms FROM office_local_profiles WHERE identity_id = ?",
            [id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
        ).unwrap();
        assert_eq!(after, before);
    }
}
