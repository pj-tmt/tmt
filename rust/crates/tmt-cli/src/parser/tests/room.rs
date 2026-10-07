use super::*;

#[test]
fn room_commands_share_typed_grammar_and_reject_unrelated_flags() {
    use crate::invocation::RoomOperation;
    use tmt_core::room::MembershipChange;
    assert!(
        matches!(parsed(&["x", "listen", "--room", "Design", "--identity", "Alice"]).invocation,
        Invocation::Exchange { operation: ExchangeOperation::Listen { room: Some(room), .. }, .. } if room == "Design")
    );
    assert_eq!(
        parsed(&["room", "create", "Design"]).invocation,
        Invocation::Room(RoomOperation::Create("Design".into()))
    );
    assert_eq!(
        parsed(&["room", "ls"]).invocation,
        Invocation::Room(RoomOperation::List)
    );
    assert_eq!(
        parsed(&["room", "show", "Design"]).invocation,
        Invocation::Room(RoomOperation::Show("Design".into()))
    );
    for (verb, change) in [
        ("join", MembershipChange::Join),
        ("leave", MembershipChange::Leave),
    ] {
        assert_eq!(
            parsed(&["room", verb, "Design", "--identity", "Alice"]).invocation,
            Invocation::Room(RoomOperation::Membership {
                room: "Design".into(),
                identity: Some("Alice".into()),
                change
            })
        );
        assert_eq!(
            parsed(&["room", verb, "Design"]).invocation,
            Invocation::Room(RoomOperation::Membership {
                room: "Design".into(),
                identity: None,
                change
            })
        );
    }
    assert_eq!(
        parsed(&["ls", "--temp", "--here", "--all"]).invocation,
        Invocation::List {
            target: None,
            room: None,
            scope: crate::invocation::ListScope {
                lifetime: Some(tmt_core::identity::Lifetime::Temporary),
                here: true,
                all: true,
            },
        }
    );
    for input in [
        vec!["ls", "--saved", "--temp"],
        vec!["ls", "Alice", "--all"],
        vec!["ls", "%3", "--here"],
    ] {
        assert_eq!(parse_error(&input).code, "USAGE_ERROR", "{input:?}");
    }
    assert_eq!(
        parsed(&["ls", "--saved"]).invocation,
        Invocation::List {
            target: None,
            room: None,
            scope: crate::invocation::ListScope {
                lifetime: Some(tmt_core::identity::Lifetime::Saved),
                ..Default::default()
            },
        }
    );
    assert_eq!(
        parsed(&["ls", "--room", "Design"]).invocation,
        Invocation::List {
            target: None,
            room: Some("Design".into()),
            scope: Default::default(),
        }
    );
    for command in [
        vec!["room"],
        vec!["room", "join"],
        vec!["room", "ls", "--identity", "Alice"],
        vec!["room", "create", "Design", "--force"],
        vec!["ls", "Alice", "--room", "Design"],
        vec!["room", "send", "Design"],
        vec!["room", "broadcast", "Design", "hello", "--timeout", "1s"],
        vec!["room", "send", "Design", "hello", "--inbox"],
    ] {
        assert!(parse(&args(&command)).is_err(), "{command:?}");
    }
    for action in ["create", "ls", "show", "join", "leave", "send", "broadcast"] {
        for help in ["-h", "--help"] {
            assert!(matches!(
                parsed(&["room", action, help]).invocation,
                Invocation::Help(_)
            ));
        }
    }
}

#[test]
fn room_dispatch_uses_the_shared_request_kind_and_operation_identity() {
    use crate::invocation::RoomOperation;
    use tmt_core::request::RequestKind;
    assert!(matches!(
        parsed(&["talk", "Alice", "Only Alice", "--room", "Design", "--inbox", "--detach"]).invocation,
        Invocation::Talk { options: TalkOptions {
            urgent: false,
            focus_kind: tmt_core::request::focus::FocusKind::Fyi, room: Some(room), inbox: true, .. }, .. } if room == "Design"
    ));
    for (verb, kind) in [
        ("send", RequestKind::Request),
        ("broadcast", RequestKind::Announcement),
    ] {
        assert_eq!(
            parsed(&[
                "room",
                verb,
                "Design",
                "hello",
                "--identity",
                "Alice",
                "--operation-id",
                "11111111-1111-4111-8111-111111111111"
            ])
            .invocation,
            Invocation::Room(RoomOperation::Dispatch {
                room: "Design".into(),
                message: "hello".into(),
                identity: Some("Alice".into()),
                operation_id: Some("11111111-1111-4111-8111-111111111111".into()),
                kind,
            })
        );
    }
}
