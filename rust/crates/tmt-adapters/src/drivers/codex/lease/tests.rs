use super::*;
use crate::test_support::TestDirectory;
use tmt_core::{binding::session::RuntimeLiveness, endpoint::ProcessIncarnation};

#[test]
fn thread_creation_requires_exact_returned_directory_and_valid_thread() {
    let fixture = TestDirectory::new();
    let cwd = fixture.path.canonicalize().unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    let response = json!({"result":{"cwd":cwd,"thread":{"id":id}}});
    assert_eq!(created_thread(&response, &cwd).unwrap().as_str(), id);
    for response in [
        json!({"error":{"code":-32603},"result":{"cwd":cwd,"thread":{"id":id}}}),
        json!({"result":{"cwd":cwd,"thread":{"id":"not-a-uuid"}}}),
        json!({"result":{"thread":{"id":id}}}),
        json!({"result":{"cwd":"/","thread":{"id":id}}}),
    ] {
        assert!(created_thread(&response, &cwd).is_err());
    }
}

#[test]
fn failed_start_retains_enrollment_unless_cleanup_is_confirmed() {
    for confirmed in [false, true] {
        let fixture = TestDirectory::new();
        let store = Store::open(&fixture.path).unwrap();
        let owner = ProcessIncarnation::new(42, "test-owner").unwrap();
        let record = Record::new("11111111-1111-4111-8111-111111111111", &owner).unwrap();
        store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
        let mut lease = Lease {
            store,
            record: record.clone(),
            server: None,
            cleanup: EnrollmentCleanup::Retire,
            command: RuntimeCommand {
                executable: "/unused".into(),
                args: vec![],
            },
            environment: vec![],
            session: None,
        };
        let cleanup = if confirmed {
            Ok(())
        } else {
            Err(io::Error::other("unconfirmed owned process teardown"))
        };
        let error = StartError::after_cleanup(io::ErrorKind::TimedOut.into(), cleanup);
        assert!(lease.accept_start(Err(error)).is_err());
        assert!(lease.server.is_none());
        if !confirmed {
            assert!(lease.withdraw().is_err());
        }
        drop(lease);
        assert_eq!(
            Store::at(&fixture.path)
                .read(&record.binding_id)
                .unwrap()
                .is_some(),
            !confirmed
        );
    }
}
