use super::super::record::{Process, Ready};
use super::*;
use tmt_core::binding::session::{ObservedSessionKey, RuntimeState};
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
