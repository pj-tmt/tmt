use super::*;
use crate::test_support::TestDirectory;
use tmt_core::identity::{Lifetime, create_or_resolve};

const ROOM: &str = "33333333-3333-4333-8333-333333333333";
struct Fixture {
    _directory: TestDirectory,
    storage: Storage,
    owner: String,
}
impl Fixture {
    fn new() -> Self {
        let directory = TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("office.db")).unwrap();
        // Until reserved predecessor 086 lands this PR's schema is deliberately
        // unregistered. Exercise its real SQL after the supported main chain.
        storage
            .connection()
            .unwrap()
            .execute_batch(include_str!("../../schema/087.sql"))
            .unwrap();
        let owner = create_or_resolve(&mut storage, "Member", Lifetime::Saved)
            .unwrap()
            .identity
            .id;
        Self {
            _directory: directory,
            storage,
            owner,
        }
    }
    fn definition(&self, room: Option<&str>, text: &str) -> JobDefinition {
        JobDefinition {
            owner_id: Some(self.owner.clone()),
            room_id: room.map(str::to_owned),
            title: None,
            message: text.into(),
            schedule: JobSchedule::Every {
                zone: "UTC".into(),
                seconds: 10,
                anchor_ms: 0,
                from: None,
            },
            pause: None,
        }
    }
    fn page(&self, room: Option<&str>, after: u64, limit: u16) -> JobListPage {
        self.storage
            .list_orchestration_jobs(JobListQuery {
                room_id: room,
                after_number: after,
                limit,
                ..Default::default()
            })
            .unwrap()
    }
}
#[test]
fn exact_message_survives_reopen_and_removed_numbers_never_recycle() {
    let mut f = Fixture::new();
    let text = format!("{}\0雪\r\n{}", " ".repeat(300), "λ".repeat(20_000));
    let first = f
        .storage
        .create_orchestration_job(f.definition(None, &text))
        .unwrap();
    let mut reopened = Storage::open(&f.storage.path).unwrap();
    assert_eq!(
        reopened.orchestration_job(&first.id).unwrap(),
        Some(first.clone())
    );
    // A whitespace-only prefix is valid metadata, not a corrupt full body.
    assert_eq!(f.page(None, 0, 1).items[0].message_preview, " ".repeat(256));
    reopened
        .remove_orchestration_job(&first.id, first.revision)
        .unwrap();
    assert!(reopened.orchestration_job(&first.id).unwrap().is_none());
    let second = reopened
        .create_orchestration_job(f.definition(None, "Next"))
        .unwrap();
    assert!(second.number > first.number);
    assert_ne!(second.id, first.id);
    assert_eq!(f.page(None, 0, 1).items[0].id, second.id);
}
#[test]
fn filtered_page_excludes_null_room_before_limit_and_all_includes_it() {
    let mut f = Fixture::new();
    let unscoped = f
        .storage
        .create_orchestration_job(f.definition(None, "No squad"))
        .unwrap();
    let squad = f
        .storage
        .create_orchestration_job(f.definition(Some(ROOM), &"雪".repeat(500)))
        .unwrap();
    let unscoped_two = f
        .storage
        .create_orchestration_job(f.definition(None, "Another"))
        .unwrap();
    let all = f.page(None, 0, 1);
    assert_eq!(all.items[0].id, unscoped.id);
    let next = f.page(None, all.next_after.unwrap(), 1);
    assert_eq!(next.items[0].id, squad.id);
    assert_eq!(next.items[0].message_preview.len(), 255);
    assert_eq!(next.items[0].message_preview, "雪".repeat(85));
    let last = f.page(None, next.next_after.unwrap(), 1);
    assert_eq!(last.items[0].id, unscoped_two.id);
    assert!(last.next_after.is_none());
    let filtered = f.page(Some(ROOM), 0, 1);
    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].id, squad.id);
    assert!(filtered.next_after.is_none());
}
#[test]
fn revision_fence_preserves_pause_and_refused_change_rolls_back() {
    let mut f = Fixture::new();
    let job = f
        .storage
        .create_orchestration_job(f.definition(None, "Keep exact text"))
        .unwrap();
    let paused = f
        .storage
        .change_orchestration_job(&job.id, 1, |current| {
            current.pause = Some(JobPause {
                by: "user".into(),
                at_ms: 100,
            });
            Ok(())
        })
        .unwrap();
    assert_eq!(paused.revision, 2);
    let mut stale = Storage::open(&f.storage.path).unwrap();
    assert!(matches!(
        stale.change_orchestration_job(&job.id, 1, |_| panic!("stale callback must not run")),
        Err(JobStoreError::Job(JobError::Conflict))
    ));
    let refused = f.storage.change_orchestration_job(&job.id, 2, |current| {
        current.message = "Uncommitted".into();
        Err(JobError::NotAllowed.into())
    });
    assert!(matches!(
        refused,
        Err(JobStoreError::Job(JobError::NotAllowed))
    ));
    assert_eq!(
        f.storage.orchestration_job(&job.id).unwrap(),
        Some(paused.clone())
    );
    assert!(matches!(
        f.storage.remove_orchestration_job(&job.id, 1),
        Err(JobStoreError::Job(JobError::Conflict))
    ));
    assert_eq!(f.storage.orchestration_job(&job.id).unwrap(), Some(paused));
}
#[test]
fn missing_owner_is_never_rebound_by_name_and_unowned_requires_pause() {
    let mut f = Fixture::new();
    let mut definition = f.definition(None, "Explicitly unowned");
    definition.owner_id = None;
    assert!(matches!(
        f.storage.create_orchestration_job(definition.clone()),
        Err(JobStoreError::Job(JobError::Invalid))
    ));
    definition.pause = Some(JobPause {
        by: "no-owner".into(),
        at_ms: 100,
    });
    let job = f.storage.create_orchestration_job(definition).unwrap();
    assert!(job.owner_id.is_none());
    assert!(f.page(None, 0, 1).items[0].owner_name.is_none());
    let mut definition = f.definition(None, "Unknown owner");
    definition.owner_id = Some("44444444-4444-4444-8444-444444444444".into());
    assert!(matches!(
        f.storage.create_orchestration_job(definition),
        Err(JobStoreError::Job(JobError::NoOwner))
    ));
    assert_eq!(f.page(None, 0, 128).items.len(), 1);
}

const NOW: u64 = 1_700_000_000_000;
fn run_input(job: &OrchJob, slot_ms: i64) -> JobRunInput {
    JobRunInput {
        job_id: job.id.clone(),
        expected_revision: job.revision,
        slot_ms,
        manual_operation_id: None,
        originator: tmt_core::request::Originator::Unknown,
    }
}
fn focus(f: &mut Fixture, enabled: bool) {
    f.storage
        .update_orchestration(|before| {
            let mut after = before.snapshot.clone();
            after.revision += 1;
            after.focus = if enabled {
                vec![tmt_core::orchestration::OrchFocus {
                    identity_id: f.owner.clone(),
                    revision: 1,
                    start_ms: NOW - 1,
                    end_ms: NOW + 10_000,
                }]
            } else {
                vec![]
            };
            Ok::<_, StorageError>((after, ()))
        })
        .unwrap();
}
fn count(f: &Fixture, table: &str) -> i64 {
    f.storage
        .connection()
        .unwrap()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}
#[test]
fn held_slots_coalesce_and_acceptance_receipt_prevents_a_second_catchup() {
    let mut f = Fixture::new();
    let job = f
        .storage
        .create_orchestration_job(f.definition(None, "Exact scheduled message\0雪"))
        .unwrap();
    focus(&mut f, true);
    assert!(matches!(
        f.storage
            .run_orchestration_job(run_input(&job, 10_000), 7, || NOW)
            .unwrap(),
        JobRunResult::Held {
            journal_id: Some(_),
            ..
        }
    ));
    assert!(matches!(
        f.storage
            .run_orchestration_job(run_input(&job, 20_000), 7, || NOW)
            .unwrap(),
        JobRunResult::Held {
            journal_id: None,
            ..
        }
    ));
    assert_eq!(count(&f, "request_attempts"), 0);
    assert_eq!(count(&f, "orchestration_decisions"), 1);
    focus(&mut f, false);
    let JobRunResult::Accepted {
        receipt,
        created: true,
        run_id,
    } = f
        .storage
        .run_orchestration_job(run_input(&job, 30_000), 7, || NOW)
        .unwrap()
    else {
        panic!("wake must accept the held slot")
    };
    assert_eq!(receipt.operation_id, job_operation_id(&job, 10_000));
    assert_eq!(receipt.items.len(), 1);
    assert_eq!(
        receipt.items[0].acceptance,
        tmt_core::dispatch::Acceptance::Queued
    );
    assert!(
        f.storage
            .orchestration_job(&job.id)
            .unwrap()
            .unwrap()
            .pending
            .is_none()
    );
    let mut reopened = Storage::open(&f.storage.path).unwrap();
    let JobRunResult::Accepted {
        receipt: replayed,
        created: false,
        run_id: replayed_id,
    } = reopened
        .run_orchestration_job(run_input(&job, 30_000), 7, || {
            panic!("replay must not observe another opportunity")
        })
        .unwrap()
    else {
        panic!("recover original acceptance")
    };
    assert_eq!(replayed, receipt);
    assert_eq!(replayed_id, run_id);
    assert_eq!(count(&f, "request_attempts"), 1);
    assert_eq!(count(&f, "office_dispatch_operations"), 1);
    assert_eq!(count(&f, "orchestration_job_runs"), 1);
    assert_eq!(count(&f, "orchestration_decisions"), 2);
    let message: String = f
        .storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT message_text FROM request_attempts WHERE request_id=?",
            [&receipt.items[0].request_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(message, job.message);
}
#[test]
fn run_record_failure_rolls_back_inbox_receipt_and_held_slot_consumption() {
    let mut f = Fixture::new();
    let job = f
        .storage
        .create_orchestration_job(f.definition(None, "Must survive rollback"))
        .unwrap();
    focus(&mut f, true);
    f.storage
        .run_orchestration_job(run_input(&job, 10_000), 7, || NOW)
        .unwrap();
    let held = f.storage.orchestration_job(&job.id).unwrap().unwrap();
    focus(&mut f, false);
    f.storage.connection().unwrap().execute_batch("CREATE TRIGGER fail_run BEFORE INSERT ON orchestration_job_runs BEGIN SELECT RAISE(ABORT,'injected run record failure'); END;").unwrap();
    assert!(
        f.storage
            .run_orchestration_job(run_input(&job, 30_000), 7, || NOW)
            .is_err()
    );
    assert_eq!(f.storage.orchestration_job(&job.id).unwrap(), Some(held));
    for table in [
        "request_attempts",
        "office_dispatch_operations",
        "orchestration_job_runs",
    ] {
        assert_eq!(count(&f, table), 0, "{table}");
    }
    assert_eq!(
        count(&f, "orchestration_decisions"),
        1,
        "no explanation for an uncommitted admission"
    );
}
#[test]
fn explicit_send_preserves_manual_pause_and_retired_owner_never_transfers() {
    let mut f = Fixture::new();
    let mut definition = f.definition(None, "Paused but explicitly sent");
    definition.pause = Some(JobPause {
        by: "user".into(),
        at_ms: 100,
    });
    let job = f.storage.create_orchestration_job(definition).unwrap();
    assert!(matches!(
        f.storage
            .run_orchestration_job(run_input(&job, 10_000), 7, || NOW)
            .unwrap(),
        JobRunResult::Paused
    ));
    let mut input = run_input(&job, 20_000);
    input.manual_operation_id = Some("55555555-5555-4555-8555-555555555555".into());
    assert!(matches!(
        f.storage.run_orchestration_job(input, 7, || NOW).unwrap(),
        JobRunResult::Accepted { created: true, .. }
    ));
    assert_eq!(
        f.storage.orchestration_job(&job.id).unwrap(),
        Some(job.clone())
    );
    let mut changed_sender = run_input(&job, 20_000);
    changed_sender.manual_operation_id = Some("55555555-5555-4555-8555-555555555555".into());
    changed_sender.originator = tmt_core::request::Originator::Explicit(f.owner.clone());
    assert!(matches!(
        f.storage.run_orchestration_job(changed_sender, 7, || NOW),
        Err(JobStoreError::Dispatch(
            crate::storage::DispatchError::IdempotencyConflict
        ))
    ));
    assert_eq!(count(&f, "request_attempts"), 1);
    f.storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms=? WHERE id=?",
            params![NOW as i64, f.owner],
        )
        .unwrap();
    let successor = create_or_resolve(&mut f.storage, "Member", Lifetime::Saved)
        .unwrap()
        .identity
        .id;
    assert_ne!(successor, f.owner);
    assert!(matches!(
        f.storage
            .run_orchestration_job(run_input(&job, 30_000), 7, || NOW)
            .unwrap(),
        JobRunResult::NoOwner
    ));
    assert_eq!(count(&f, "request_attempts"), 1);
}

#[test]
fn job_definitions_share_configuration_revision_and_noops_write_nothing() {
    let mut f = Fixture::new();
    assert_eq!(
        f.storage
            .orchestration_snapshot(None)
            .unwrap()
            .snapshot
            .revision,
        0
    );
    let job = f
        .storage
        .create_orchestration_job(f.definition(None, "Configuration edit"))
        .unwrap();
    assert_eq!(
        f.storage
            .orchestration_snapshot(None)
            .unwrap()
            .snapshot
            .revision,
        1
    );
    let unchanged = f
        .storage
        .change_orchestration_job(&job.id, 1, |_| Ok(()))
        .unwrap();
    assert_eq!(unchanged, job);
    assert_eq!(
        f.storage
            .orchestration_snapshot(None)
            .unwrap()
            .snapshot
            .revision,
        1
    );
    f.storage.connection().unwrap().execute_batch("CREATE TRIGGER refuse_job_configuration BEFORE UPDATE ON orchestration_settings BEGIN SELECT RAISE(ABORT,'injected configuration failure'); END;").unwrap();
    assert!(
        f.storage
            .change_orchestration_job(&job.id, 1, |current| {
                current.title = Some("Uncommitted".into());
                Ok(())
            })
            .is_err()
    );
    assert_eq!(
        f.storage.orchestration_job(&job.id).unwrap(),
        Some(job.clone())
    );
    assert_eq!(
        f.storage
            .orchestration_snapshot(None)
            .unwrap()
            .snapshot
            .revision,
        1
    );
    assert!(f.storage.remove_orchestration_job(&job.id, 1).is_err());
    assert_eq!(
        f.storage.orchestration_job(&job.id).unwrap(),
        Some(job.clone())
    );
    f.storage
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_job_configuration")
        .unwrap();
    f.storage.remove_orchestration_job(&job.id, 1).unwrap();
    assert_eq!(
        f.storage
            .orchestration_snapshot(None)
            .unwrap()
            .snapshot
            .revision,
        2
    );
    assert_eq!(
        count(&f, "orchestration_decisions"),
        0,
        "configuration is not an execution outcome"
    );
}
