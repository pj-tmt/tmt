//! Join an already host-verified launch fence with the canonical checklist claim.
use super::{
    IdentityContextSnapshot, Storage, StorageError, bindings::BindingRows,
    identities::with_immediate_transaction, requests::TransactionRequests,
};
use tmt_core::{
    binding::BindingRecords,
    request::{
        RequestError, RequestService,
        focus::{FocusChecklist, FocusOpportunity},
    },
};

impl Storage {
    pub fn claim_focus_for_launch(
        &mut self,
        expected: &IdentityContextSnapshot,
        id: String,
        token: String,
    ) -> Result<Option<FocusChecklist>, RequestError<StorageError>> {
        with_immediate_transaction(self, "launch checklist", |transaction| {
            let rows = BindingRows(transaction);
            let identity = &expected.entry.identity.id;
            let current = rows.entry_by_id(identity)?;
            let preferences = rows.session_preferences(identity)?;
            // Telemetry may change in a simultaneous observation hook. Admission
            // only depends on the exact binding/session/owner and remembered pair.
            let remembered = |prefs: &tmt_core::binding::session::SessionPreferences| {
                prefs
                    .remembered
                    .as_ref()
                    .map(|r| (r.harness.clone(), r.provider_session.clone()))
            };
            if current
                .as_ref()
                .is_none_or(|entry| entry.binding != expected.entry.binding)
                || remembered(&preferences) != remembered(&expected.preferences)
            {
                return Ok(None);
            }
            let mut requests = TransactionRequests(transaction);
            RequestService::new(&mut requests, crate::request_runtime::wall_time_ms)
                .claim_focus_checklist(identity, id, token, FocusOpportunity::TurnBoundary)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use tmt_core::identity::{Lifetime, create_or_resolve};

    fn fixture() -> (TestDirectory, std::path::PathBuf, String) {
        let directory = TestDirectory::new();
        let path = directory.path.join("focus.db");
        let mut storage = Storage::open(&path).unwrap();
        let id = create_or_resolve(&mut storage, "Focused", Lifetime::Saved)
            .unwrap()
            .identity
            .id;
        let db = storage.connection().unwrap();
        db.execute("INSERT INTO bindings(id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at,runtime_state,runtime_pid,runtime_start_identity,observed_provider_session_id,launch_owner_pid,launch_owner_start_identity)
            VALUES('binding',?1,'tmux','%1','server','/fixture',10,'server-start',11,'original','original','running',12,'runtime-start','session',13,'owner-start')", [&id]).unwrap();
        db.execute("INSERT INTO identity_session_preferences(identity_id,remembered_harness,runtime_mode,provider_session_id) VALUES(?1,'claude','default','session')", [&id]).unwrap();
        let request = format!("req_{}", tmt_core::operation::new_operation_id());
        db.execute("INSERT INTO request_attempts(attempt_id,request_id,route_kind,wait_active,status,inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms,originator_kind,recipient_identity_id)
            VALUES(?1,?1,'inbox',0,'queued',0,0,1,9007199254740991,9007199254740991,'unknown',?2)", rusqlite::params![request,id]).unwrap();
        db.execute("INSERT INTO focus_items(identity_id,request_id,kind,source,created_at_ms) VALUES(?1,?2,'review','incoming',1)", rusqlite::params![id,request]).unwrap();
        storage.close().unwrap();
        (directory, path, id)
    }

    fn claim(storage: &mut Storage, expected: &IdentityContextSnapshot) -> Option<FocusChecklist> {
        storage
            .claim_focus_for_launch(
                expected,
                tmt_core::operation::new_operation_id(),
                tmt_core::operation::new_operation_id(),
            )
            .unwrap()
    }

    #[test]
    fn each_changed_authority_refuses_before_sealing_and_telemetry_does_not() {
        for update in [
            "UPDATE bindings SET id='replacement'",
            "UPDATE bindings SET launch_owner_start_identity='other'",
            "UPDATE bindings SET runtime_start_identity='other'",
            "UPDATE bindings SET observed_provider_session_id='other'",
            "UPDATE bindings SET runtime_state='unknown'",
            "UPDATE identity_session_preferences SET provider_session_id='other'",
            "UPDATE identity_session_preferences SET remembered_harness='codex'",
            "UPDATE identities SET retired_at_ms=1",
        ] {
            let (_directory, path, id) = fixture();
            let expected = Storage::context_by_identity(&path, &id, 1)
                .unwrap()
                .unwrap();
            let mut storage = Storage::open(&path).unwrap();
            storage.connection().unwrap().execute(update, []).unwrap();
            assert!(claim(&mut storage, &expected).is_none(), "{update}");
            assert_eq!(
                storage
                    .connection()
                    .unwrap()
                    .query_row("SELECT COUNT(*) FROM focus_checklists", [], |row| row
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(
                storage
                    .connection()
                    .unwrap()
                    .query_row(
                        "SELECT COUNT(*) FROM focus_items WHERE checklist_id IS NULL",
                        [],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                1
            );
            storage.close().unwrap();
        }
        let (_directory, path, id) = fixture();
        let expected = Storage::context_by_identity(&path, &id, 1)
            .unwrap()
            .unwrap();
        let mut storage = Storage::open(&path).unwrap();
        storage.connection().unwrap().execute("UPDATE identity_session_preferences SET driver_state_version=1,driver_state='{\"activity\":\"idle\"}'",[]).unwrap();
        assert!(claim(&mut storage, &expected).is_some());
        storage.close().unwrap();
    }

    #[test]
    fn concurrent_launch_claims_have_one_winner_and_one_sealed_inventory() {
        let (_directory, path, id) = fixture();
        let operations = [0, 1].map(|_| {
            let expected = Storage::context_by_identity(&path, &id, 1)
                .unwrap()
                .unwrap();
            Box::new(move |storage: &mut Storage| claim(storage, &expected))
                as super::super::test_support::Operation<Option<FocusChecklist>>
        });
        let results = super::super::test_support::concurrent_pair(&path, operations);
        assert_eq!(results.iter().filter(|value| value.is_some()).count(), 1);
        let mut storage = Storage::open(&path).unwrap();
        assert_eq!(
            storage
                .connection()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM focus_checklists WHERE state='claimed'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            storage
                .connection()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM focus_items WHERE checklist_id IS NOT NULL",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        storage.close().unwrap();
    }
}
