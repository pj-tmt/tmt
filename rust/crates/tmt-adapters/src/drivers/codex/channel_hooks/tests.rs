use super::super::record::{Process, Ready};
use super::*;
use tmt_core::binding::session::{ObservedSessionKey, RuntimeState};
#[test]
fn private_locator_refuses_missing_record_and_other_generation() {
    let directory = crate::test_support::TestDirectory::new();
    let store = Store::open(&directory.path).unwrap();
    let owner = ProcessIncarnation::new(10, "owner").unwrap();
    let record = Record::new("11111111-1111-4111-8111-111111111111", &owner).unwrap();
    assert!(scoped_record(&directory.path, &record.binding_id, &record.generation).is_none());
    store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
    assert!(scoped_record(&directory.path, &record.binding_id, &record.generation).is_some());
    assert!(scoped_record(&directory.path, &record.binding_id, "wrong-generation").is_none());
}
#[test]
fn binding_locator_requires_private_ready_thread_and_known_foreground() {
    let event = super::super::decode_hook(br#"{"hook_event_name":"SessionStart","source":"resume","session_id":"22222222-2222-4222-8222-222222222222"}"#).unwrap();
    let mut observation = ChannelObservation {
        event,
        record: None,
    };
    assert_eq!(observation.verified_binding(), None);
    let owner = ProcessIncarnation::new(10, "owner").unwrap();
    let mut record = Record::new("11111111-1111-4111-8111-111111111111", &owner).unwrap();
    observation.record = Some(record.clone());
    assert_eq!(observation.verified_binding(), None);
    record.ready = Some(Ready {
        server: Process::of(&owner),
        port: 49000,
        thread: observation.event.session.as_str().into(),
    });
    observation.record = Some(record.clone());
    assert_eq!(observation.verified_binding(), None);
    record.foreground = Foreground::Known(Process::of(
        &ProcessIncarnation::new(12, "foreground").unwrap(),
    ));
    observation.record = Some(record.clone());
    assert_eq!(
        observation.verified_binding(),
        Some(record.binding_id.as_str())
    );
    record.ready.as_mut().unwrap().thread = "foreign".into();
    observation.record = Some(record);
    assert_eq!(observation.verified_binding(), None);
}
#[test]
fn channel_server_resume_and_end_preserve_foreground_but_require_exact_live_proof() {
    let owner = ProcessIncarnation::new(10, "owner").unwrap();
    let server = ProcessIncarnation::new(11, "server").unwrap();
    let foreground = ProcessIncarnation::new(12, "foreground").unwrap();
    let thread = ProviderSessionId::new("22222222-2222-4222-8222-222222222222").unwrap();
    let mut record = Record::new("11111111-1111-4111-8111-111111111111", &owner).unwrap();
    record.ready = Some(Ready {
        server: Process::of(&server),
        port: 49000,
        thread: thread.as_str().into(),
    });
    record.foreground = Foreground::Known(Process::of(&foreground));
    let current = BindingSessionState {
        state: RuntimeState::Running,
        key: Some(ObservedSessionKey {
            incarnation: foreground,
            provider_session: Some(thread.clone()),
        }),
        launch_owner: Some(owner),
        last_transition: Some(SessionTransition::Resumed),
    };
    for kind in [
        SessionTransition::Resumed,
        SessionTransition::Compacted,
        SessionTransition::Ended,
    ] {
        let event = CodexObservation {
            session: thread.clone(),
            model: None,
            starting: kind != SessionTransition::Ended,
            transition: kind,
        };
        let next = transition(&event, &record, &current, &server, &|_| {
            RuntimeLiveness::Alive
        })
        .unwrap();
        assert_eq!(next.key, current.key);
        assert_eq!(next.launch_owner, current.launch_owner);
        for pid in [10, 11, 12] {
            assert!(
                transition(&event, &record, &current, &server, &|p| if p.pid() == pid {
                    RuntimeLiveness::Unknown
                } else {
                    RuntimeLiveness::Alive
                })
                .is_none()
            );
        }
        for foreground in [
            Foreground::Unknown,
            Foreground::Known(Process {
                pid: 12,
                start: "different-incarnation".into(),
            }),
        ] {
            let mut unpublished = record.clone();
            unpublished.foreground = foreground;
            assert!(
                transition(&event, &unpublished, &current, &server, &|_| {
                    RuntimeLiveness::Alive
                })
                .is_none()
            );
        }
        let wrong = CodexObservation {
            session: ProviderSessionId::new("other").unwrap(),
            ..event.clone()
        };
        assert!(
            transition(&wrong, &record, &current, &server, &|_| {
                RuntimeLiveness::Alive
            })
            .is_none()
        );
        assert!(
            transition(
                &event,
                &record,
                &current,
                &ProcessIncarnation::new(13, "other-server").unwrap(),
                &|_| RuntimeLiveness::Alive
            )
            .is_none()
        );
    }
    // The existing ordinary shared-resume policy remains deliberately separate.
    let event = CodexObservation {
        session: thread,
        model: None,
        starting: true,
        transition: SessionTransition::Resumed,
    };
    let ordinary = event
        .propose_with_resume(&current, &server, RuntimeLiveness::Alive, true, true)
        .unwrap();
    assert_eq!(ordinary.key.unwrap().incarnation, server);
}
