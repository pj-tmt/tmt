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
        notes_nudge: Default::default(),
        state: RuntimeState::Running,
        key: Some(ObservedSessionKey {
            incarnation: foreground,
            provider_session: Some(thread.clone()),
        }),
        launch_owner: Some(owner),
        last_transition: Some(SessionTransition::Resumed),
    };
    // Startup preserves an admitted channel's stored transition. Consumers must
    // inspect this callback's own kind rather than replay an old compaction.
    let mut compacted = current.clone();
    compacted.last_transition = Some(SessionTransition::Compacted);
    let startup = CodexObservation {
        session: thread.clone(),
        model: None,
        starting: true,
        transition: SessionTransition::Started,
    };
    let next = transition(&startup, &record, &compacted, &server, &|_| {
        RuntimeLiveness::Alive
    })
    .unwrap();
    assert_eq!(next.last_transition, Some(SessionTransition::Compacted));
    let observation = ChannelObservation {
        event: startup,
        record: Some(record.clone()),
    };
    assert_eq!(observation.transition(), Some(SessionTransition::Started));
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

fn pending_record() -> Record {
    let mut record = Record::new(
        "11111111-1111-4111-8111-111111111111",
        &ProcessIncarnation::new(10, "owner").unwrap(),
    )
    .unwrap();
    record.fresh = Some(super::super::record::FreshEndpoint {
        server: Process::of(&ProcessIncarnation::new(11, "server").unwrap()),
        port: 49000,
        cwd: "/tmp".into(),
        thread: None,
    });
    record
}

#[test]
fn admission_gate_unrelated_generation_ready_and_known_auxiliary_never_wait() {
    let session = ProviderSessionId::new("22222222-2222-4222-8222-222222222222").unwrap();
    let pending = pending_record();
    let mut ready = pending.clone();
    ready.ready = Some(Ready {
        server: ready.fresh.as_ref().unwrap().server.clone(),
        port: 49000,
        thread: session.as_str().into(),
    });
    let mut unrelated = pending.clone();
    unrelated.fresh.as_mut().unwrap().thread = Some("33333333-3333-4333-8333-333333333333".into());
    for (generation, record) in [
        ("another-generation", pending.clone()),
        (pending.generation.as_str(), ready),
        (pending.generation.as_str(), unrelated),
    ] {
        let reads = std::cell::Cell::new(0);
        assert!(
            wait_scoped(
                generation,
                &session,
                Instant::now(),
                || {
                    reads.set(reads.get() + 1);
                    Ok(Some(record.clone()))
                },
                |_| panic!("immediate path must not wait or probe")
            )
            .is_ok()
        );
        assert_eq!(reads.get(), 1);
    }
    // Plain/legacy Codex records have no deferred fresh publication.
    let mut plain = pending.clone();
    plain.fresh = None;
    assert!(
        wait_scoped(
            &pending.generation,
            &session,
            Instant::now(),
            || Ok(Some(plain.clone())),
            |_| panic!("plain hook waited")
        )
        .is_ok()
    );
}

#[test]
fn eager_same_generation_hook_waits_for_candidate_then_admission() {
    let pending = pending_record();
    let session = ProviderSessionId::new("22222222-2222-4222-8222-222222222222").unwrap();
    let mut candidate = pending.clone();
    candidate.fresh.as_mut().unwrap().thread = Some(session.as_str().into());
    let mut admitted = candidate.clone();
    admitted.ready = Some(Ready {
        server: candidate.fresh.as_ref().unwrap().server.clone(),
        port: 49000,
        thread: session.as_str().into(),
    });
    let mut records = [pending.clone(), candidate, admitted].into_iter();
    let waits = std::cell::Cell::new(0);
    assert!(
        wait_scoped(
            &pending.generation,
            &session,
            Instant::now() + Duration::from_secs(1),
            || Ok(records.next()),
            |_| waits.set(waits.get() + 1)
        )
        .is_ok()
    );
    assert_eq!(waits.get(), 2);
}

#[test]
fn admission_gate_timeout_leaves_unready_evidence_unchanged() {
    let pending = pending_record();
    let session = ProviderSessionId::new("22222222-2222-4222-8222-222222222222").unwrap();
    assert!(
        wait_scoped(
            &pending.generation,
            &session,
            Instant::now(),
            || Ok(Some(pending.clone())),
            |_| panic!("expired gate waited")
        )
        .is_err()
    );
    assert!(pending.ready.is_none());
    assert!(pending.fresh.unwrap().thread.is_none());
}

#[test]
fn timed_out_early_hook_does_not_latch_unready_or_authorize_another_session() {
    let fixture = crate::test_support::TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let mut pending = pending_record();
    let foreground = ProcessIncarnation::new(12, "foreground").unwrap();
    pending.foreground = Foreground::Known(Process::of(&foreground));
    store.create(&pending, |_| RuntimeLiveness::Alive).unwrap();
    let session = ProviderSessionId::new("22222222-2222-4222-8222-222222222222").unwrap();
    assert!(
        wait_scoped(
            &pending.generation,
            &session,
            Instant::now(),
            || store
                .read(&pending.binding_id)
                .map_err(|_| crate::runtime::lifecycle::LifecycleUnavailable),
            |_| panic!("expired gate waited")
        )
        .is_err()
    );
    let candidate = store.fresh_thread(&pending, session.as_str()).unwrap();
    let ready = store
        .admit_fresh(&candidate, &foreground, |_| RuntimeLiveness::Alive)
        .unwrap();
    let current = BindingSessionState {
        notes_nudge: Default::default(),
        state: RuntimeState::Running,
        key: Some(ObservedSessionKey {
            incarnation: foreground,
            provider_session: Some(session.clone()),
        }),
        launch_owner: ready.launch_owner.incarnation(),
        last_transition: Some(SessionTransition::Started),
    };
    let server = ready.ready.as_ref().unwrap().server.incarnation().unwrap();
    for (thread, accepted) in [
        (session, true),
        (
            ProviderSessionId::new("33333333-3333-4333-8333-333333333333").unwrap(),
            false,
        ),
    ] {
        assert!(
            wait_scoped(
                &ready.generation,
                &thread,
                Instant::now(),
                || Ok(Some(ready.clone())),
                |_| panic!("Ready hook waited")
            )
            .is_ok()
        );
        let event = CodexObservation {
            session: thread,
            model: None,
            starting: true,
            transition: SessionTransition::Started,
        };
        let observation = ChannelObservation {
            event,
            record: Some(ready.clone()),
        };
        assert_eq!(observation.verified_binding().is_some(), accepted);
        assert_eq!(
            transition(&observation.event, &ready, &current, &server, &|_| {
                RuntimeLiveness::Alive
            })
            .is_some(),
            accepted
        );
    }
}

fn activity_fixture() -> (
    Record,
    BindingSessionState,
    ProviderSessionId,
    ProcessIncarnation,
) {
    let mut record = pending_record();
    let session = ProviderSessionId::new("22222222-2222-4222-8222-222222222222").unwrap();
    record.fresh.as_mut().unwrap().thread = Some(session.as_str().into());
    record.ready = Some(Ready {
        server: record.fresh.as_ref().unwrap().server.clone(),
        port: 49000,
        thread: session.as_str().into(),
    });
    let foreground = ProcessIncarnation::new(12, "foreground").unwrap();
    record.foreground = Foreground::Known(Process::of(&foreground));
    let current = BindingSessionState {
        notes_nudge: Default::default(),
        state: RuntimeState::Running,
        key: Some(ObservedSessionKey {
            incarnation: foreground,
            provider_session: Some(session.clone()),
        }),
        launch_owner: record.launch_owner.incarnation(),
        last_transition: Some(SessionTransition::Started),
    };
    let server = record.ready.as_ref().unwrap().server.incarnation().unwrap();
    (record, current, session, server)
}

#[test]
fn channel_activity_uses_foreground_for_fresh_and_exact_resume_without_binding_write() {
    use crate::runtime::lifecycle::RuntimeLifecycle;
    let (mut record, current, session, server) = activity_fixture();
    let original = current.clone();
    let lifecycle = super::super::CodexLifecycle;
    for fresh in [true, false] {
        if !fresh {
            record.fresh = None;
        }
        let mapped = activity_for_record(
            Some(&record),
            &current,
            &server,
            &session,
            HostEvidence::Ambiguous {
                runtime_pid: server.pid() as u32,
            },
            Instant::now() + Duration::from_secs(1),
            &|_| RuntimeLiveness::Alive,
        )
        .unwrap();
        assert_eq!(&mapped, &current.key.as_ref().unwrap().incarnation);
        let mut state = None;
        for (event, phase, now) in [("UserPromptSubmit", "working", 1), ("Stop", "idle", 2)] {
            let payload = serde_json::json!({"hook_event_name":event,"session_id":session.as_str(),"turn_id":"turn"}).to_string();
            let activity = lifecycle.decode_activity(payload.as_bytes()).unwrap();
            state = lifecycle.activity_state(&activity, &session, &mapped, state.as_ref(), now);
            let document: serde_json::Value =
                serde_json::from_str(state.as_ref().unwrap().document()).unwrap();
            assert_eq!(document["activity"]["state"], phase);
            assert_eq!(document["activity"]["pid"], mapped.pid());
        }
        assert_eq!(current, original);
    }
}

#[test]
fn channel_activity_refuses_each_missing_malformed_and_replaced_proof() {
    let (record, current, session, server) = activity_fixture();
    let host = HostEvidence::Ambiguous {
        runtime_pid: server.pid() as u32,
    };
    let deadline = Instant::now() + Duration::from_secs(1);
    let verify = |record: Option<&Record>,
                  state: &BindingSessionState,
                  observed: &ProcessIncarnation,
                  session: &ProviderSessionId| {
        activity_for_record(record, state, observed, session, host, deadline, &|_| {
            RuntimeLiveness::Alive
        })
    };
    assert!(verify(None, &current, &server, &session).is_none());
    for changed in [
        {
            let mut r = record.clone();
            r.ready = None;
            r
        },
        {
            let mut r = record.clone();
            r.foreground = Foreground::Unknown;
            r
        },
        {
            let mut r = record.clone();
            r.launch_owner.start.clear();
            r
        },
        {
            let mut r = record.clone();
            r.foreground = Foreground::Known(Process {
                pid: 12,
                start: String::new(),
            });
            r
        },
        {
            let mut r = record.clone();
            r.ready.as_mut().unwrap().server.start.clear();
            r
        },
        {
            let mut r = record.clone();
            r.ready.as_mut().unwrap().thread = "auxiliary".into();
            r
        },
        {
            let mut r = record.clone();
            r.foreground = Foreground::Known(Process {
                pid: 12,
                start: "replacement".into(),
            });
            r
        },
        {
            let mut r = record.clone();
            r.launch_owner.start = "replacement".into();
            r
        },
        {
            let mut r = record.clone();
            r.ready.as_mut().unwrap().server.start = "replacement".into();
            r
        },
    ] {
        assert!(verify(Some(&changed), &current, &server, &session).is_none());
    }
    for changed in [
        BindingSessionState::default(),
        {
            let mut c = current.clone();
            c.key = None;
            c
        },
        {
            let mut c = current.clone();
            c.launch_owner = None;
            c
        },
        {
            let mut c = current.clone();
            c.key.as_mut().unwrap().provider_session = None;
            c
        },
        {
            let mut c = current.clone();
            c.state = RuntimeState::Ended;
            c
        },
    ] {
        assert!(verify(Some(&record), &changed, &server, &session).is_none());
    }
    assert!(
        verify(
            Some(&record),
            &current,
            &server,
            &ProviderSessionId::new("auxiliary").unwrap()
        )
        .is_none()
    );
    assert!(
        verify(
            Some(&record),
            &current,
            &ProcessIncarnation::new(11, "replacement").unwrap(),
            &session
        )
        .is_none()
    );
    for pid in [10, 11, 12] {
        for liveness in [RuntimeLiveness::Unknown, RuntimeLiveness::Gone] {
            assert!(
                activity_for_record(
                    Some(&record),
                    &current,
                    &server,
                    &session,
                    host,
                    deadline,
                    &|p| if p.pid() == pid {
                        liveness
                    } else {
                        RuntimeLiveness::Alive
                    }
                )
                .is_none()
            );
        }
    }
    assert!(
        activity_for_record(
            Some(&record),
            &current,
            &server,
            &session,
            host,
            Instant::now(),
            &|_| panic!("expired activity probed")
        )
        .is_none()
    );
    assert!(
        activity_for_record(
            Some(&record),
            &current,
            &server,
            &session,
            HostEvidence::Unsupported,
            deadline,
            &|_| panic!("unrelated activity probed")
        )
        .is_none()
    );
}

#[test]
fn activity_locator_refuses_replaced_generation_and_malformed_record_before_probes() {
    let fixture = crate::test_support::TestDirectory::new();
    let store = Store::open(&fixture.path).unwrap();
    let (record, current, session, server) = activity_fixture();
    store.create(&record, |_| RuntimeLiveness::Alive).unwrap();
    for generation in ["replacement", record.generation.as_str()] {
        if generation == record.generation {
            std::fs::write(store.path(&record.binding_id).unwrap(), b"malformed").unwrap();
        }
        let selected = scoped_record(&fixture.path, &record.binding_id, generation);
        assert!(
            activity_for_record(
                selected.as_ref(),
                &current,
                &server,
                &session,
                HostEvidence::Ambiguous { runtime_pid: 11 },
                Instant::now() + Duration::from_secs(1),
                &|_| panic!("invalid locator probed")
            )
            .is_none()
        );
    }
}
