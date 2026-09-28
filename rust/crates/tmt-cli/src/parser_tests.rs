use crate::invocation::IdentityStatusRequest;
use std::ffi::OsString;

#[test]
fn run_identity_starts_an_opaque_command_tail() {
    use std::os::unix::ffi::OsStringExt;
    let command = vec![
        OsString::from("claude"),
        "--help".into(),
        "--json".into(),
        "--resume".into(),
        "--config".into(),
        "--".into(),
        "".into(),
        OsString::from_vec(vec![0xff, b'a']),
    ];
    let mut argv = args(&["run", "Alice"]);
    argv.extend(command.clone());
    assert_eq!(
        parse(&argv).unwrap(),
        Parsed {
            invocation: Invocation::Run {
                name: "Alice".into(),
                command,
                resume: false,
                save: false,
            },
            mode: OutputMode::default(),
        }
    );
    for misplaced in ["-s", "--save", "--resume", "--help", "-", "--"] {
        let error = parse(&args(&["run", "Alice", misplaced])).unwrap_err();
        assert_eq!(error.code, "USAGE_ERROR");
        assert!(error.message.contains("TMT options go before the name"));
    }
    assert_eq!(
        parsed(&["run", "opus", "claude", "--resume", "x", "--help"]).invocation,
        Invocation::Run {
            name: "opus".into(),
            command: ["claude", "--resume", "x", "--help"]
                .map(OsString::from)
                .to_vec(),
            resume: false,
            save: false,
        }
    );
    assert_eq!(
        parsed(&["run", "--resume", "Alice"]).invocation,
        Invocation::Run {
            name: "Alice".into(),
            command: vec![],
            resume: true,
            save: false,
        }
    );
    assert_eq!(
        parsed(&["run", "--help"]).invocation,
        Invocation::Help(vec!["run".into()])
    );
    assert!(parse(&args(&["run", "--resume", "Alice", "claude"])).is_err());
    assert!(parse(&args(&["run", "--json", "Alice", "claude"])).is_err());
}

#[test]
fn run_save_belongs_only_before_the_identity() {
    for option in ["-s", "--save"] {
        assert_eq!(
            parsed(&["run", option, "Alice", "claude", "-s"]).invocation,
            Invocation::Run {
                name: "Alice".into(),
                command: vec!["claude".into(), "-s".into()],
                resume: false,
                save: true
            }
        );
        assert_eq!(
            parsed(&["run", option, "--resume", "Alice"]).invocation,
            Invocation::Run {
                name: "Alice".into(),
                command: vec![],
                resume: true,
                save: true
            }
        );
    }
}

#[test]
fn whoami_context_is_a_read_mode_not_a_new_command_or_global_option() {
    assert_eq!(parsed(&["whoami"]).invocation, Invocation::Whoami);
    assert_eq!(
        parsed(&["whoami", "--context"]).invocation,
        Invocation::WhoamiContext
    );
    let context = parsed(&["whoami", "--context", "--json"]);
    assert_eq!(context.invocation, Invocation::WhoamiContext);
    assert!(context.mode.json);
    assert!(!crate::skill_reminder::eligible_for_drift(&parsed(&[
        "whoami",
        "--context"
    ])));
    assert!(parse(&args(&["list", "--context"])).is_err());
}

#[test]
fn identity_status_uses_shared_duration_grammar_and_scoped_options() {
    for (duration, ttl_ms) in [
        ("1s", 1000),
        ("1500ms", 1500),
        ("1.5m", 90000),
        ("1440m", 86400000),
    ] {
        assert_eq!(
            parsed(&[
                "identity",
                "status",
                "set",
                "Reviewing",
                "--mood",
                "focused",
                "--for",
                duration,
                "--identity",
                "Alice"
            ])
            .invocation,
            Invocation::Identity(IdentityRequest::Status {
                identity: Some("Alice".into()),
                operation: IdentityStatusRequest::Set {
                    activity: "Reviewing".into(),
                    mood: Some("focused".into()),
                    ttl_ms
                }
            })
        );
    }
    assert_eq!(
        parsed(&["identity", "status", "set", "Reviewing"]).invocation,
        Invocation::Identity(IdentityRequest::Status {
            identity: None,
            operation: IdentityStatusRequest::Set {
                activity: "Reviewing".into(),
                mood: None,
                ttl_ms: 3600000
            }
        })
    );
    for duration in ["0", "999ms", "1441m", "NaN", "1h"] {
        assert_eq!(
            parse_error(&["identity", "status", "set", "Reviewing", "--for", duration]).code,
            "USAGE_ERROR"
        );
    }
    for verb in ["show", "clear"] {
        assert_eq!(
            parse_error(&["identity", "status", verb, "--mood", "happy"]).code,
            "USAGE_ERROR"
        );
    }
}

use super::{
    ContentInput, ExchangeOperation, IdentityFilterRequest, IdentityMetadataRequest,
    IdentityRequest, Invocation, OutputMode, ParseError, Parsed, RoleOperation, TalkOptions, parse,
};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn local_layout_commands_have_one_explicit_revision_fence_and_no_identity_target() {
    use crate::invocation::{OfficeLayoutOperation, OfficeOperation};
    assert_eq!(
        parsed(&["office", "layout", "show"]).invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Layout(OfficeLayoutOperation::Show)
        }
    );
    let basis = "a".repeat(64);
    assert_eq!(
        parsed(&[
            "office",
            "layout",
            "apply",
            "--file",
            "world.json",
            "--if-revision",
            "0",
            "--legacy-basis",
            &basis
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Layout(OfficeLayoutOperation::Apply {
                file: "world.json".into(),
                if_revision: 0,
                legacy_basis: Some(basis),
            })
        }
    );
    for input in [
        vec!["office", "layout", "show", "--identity", "Alice"],
        vec!["office", "layout", "show", "--world", "remote"],
        vec!["office", "layout", "show", "--local"],
        vec!["office", "layout", "apply", "--file", "world.json"],
        vec![
            "office",
            "layout",
            "apply",
            "--file",
            "world.json",
            "--if-revision",
            "-1",
        ],
    ] {
        assert_eq!(parse_error(&input).code, "USAGE_ERROR");
    }
}

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
        parsed(&["ls", "--room", "Design"]).invocation,
        Invocation::List {
            target: None,
            room: Some("Design".into())
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
        Invocation::Talk { options: TalkOptions { room: Some(room), inbox: true, .. }, .. } if room == "Design"
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

#[test]
fn whiteboard_snapshot_commands_require_an_exact_local_reference_and_explicit_output() {
    use crate::invocation::OfficeOperation;
    let reference = "tmt:whiteboard:snapshot:11111111-1111-4111-8111-111111111111";
    for path in [
        vec!["office", "whiteboard"],
        vec!["office", "whiteboard", "snapshot"],
        vec!["office", "whiteboard", "snapshot", "show"],
        vec!["office", "whiteboard", "snapshot", "export"],
    ] {
        for help in ["-h", "--help"] {
            let mut command = path.clone();
            command.push(help);
            assert_eq!(
                parsed(&command).invocation,
                Invocation::Help(path.iter().map(|part| (*part).into()).collect())
            );
        }
    }
    for (action, output) in [("show", None), ("export", Some("/tmp/snapshot.png"))] {
        let mut command = vec!["office", "whiteboard", "snapshot", action, reference];
        if let Some(path) = output {
            command.extend(["--output", path]);
        }
        assert_eq!(
            parsed(&command).invocation,
            Invocation::Office {
                prefix: None,
                operation: OfficeOperation::WhiteboardSnapshot {
                    reference: reference.into(),
                    output: output.map(String::from)
                }
            }
        );
    }
    for command in [
        vec!["office", "whiteboard", "snapshot", "export", reference],
        vec!["office", "whiteboard", "snapshot", "show", "latest"],
        vec![
            "office",
            "whiteboard",
            "snapshot",
            "show",
            reference,
            "--output",
            "/tmp/file",
        ],
        vec![
            "office",
            "whiteboard",
            "snapshot",
            "show",
            reference,
            "--identity",
            "alice",
        ],
    ] {
        assert!(parse(&args(&command)).is_err(), "{command:?}");
    }
}

#[test]
fn office_pairing_has_bounded_typed_options_and_retains_unqualified_status() {
    use crate::invocation::{BoardActorSelection, OfficeBoardOperation, OfficeOperation};
    let world = "https://office.example/worlds/abcdefghijklmnopqrst";
    assert_eq!(
        parsed(&["office", "unpair", "--world", world, "--identity", "Alice"]).invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Unpair {
                world: world.into(),
                identity: Some("Alice".into()),
                emulator: false,
            }
        }
    );
    assert_eq!(
        parsed(&[
            "office",
            "pair",
            "--world",
            world,
            "--identity",
            "Alice",
            "--read-only",
            "--timeout",
            "12"
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Pair {
                world: world.into(),
                identity: Some("Alice".into()),
                emulator: false,
                read_only: true,
                timeout_seconds: 12
            }
        }
    );
    assert_eq!(
        parsed(&[
            "office",
            "board",
            "edit",
            "11111111-1111-4111-8111-111111111111",
            "--owner",
            "--title",
            "new title",
            "--body",
            "new body",
            "--if-revision",
            "2"
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Board(OfficeBoardOperation::Edit {
                entry_id: "11111111-1111-4111-8111-111111111111".into(),
                actor: BoardActorSelection::Owner,
                title: Some("new title".into()),
                body: Some(ContentInput::Inline("new body".into())),
                if_revision: 2,
                operation_id: None
            })
        }
    );
    assert_eq!(
        parsed(&["office", "status", "--world", world]).invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::PairStatus {
                world: world.into(),
                identity: None,
                emulator: false
            }
        }
    );
    assert_eq!(
        parsed(&["office", "inspect", "--world", world, "--emulator"]).invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Inspect {
                world: world.into(),
                identity: None,
                emulator: true
            }
        }
    );
    for timeout in ["0", "301", "1.5", "1s", "-1", "NaN"] {
        assert_eq!(
            parse_error(&["office", "pair", "--world", world, "--timeout", timeout]).code,
            "USAGE_ERROR"
        );
    }
    for input in [
        vec!["office", "status", "--identity", "Alice"],
        vec!["office", "status", "--emulator"],
        vec!["office", "inspect"],
        vec!["office", "unpair"],
        vec!["office", "unpair", "--world", world, "--read-only"],
        vec!["office", "unpair", "--world", world, "--timeout", "5"],
        vec!["office", "status", "--world", world, "--read-only"],
    ] {
        assert_eq!(parse_error(&input).code, "USAGE_ERROR");
    }
}

#[test]
fn office_board_grammar_preserves_exact_inputs_and_actor_category_choices() {
    use crate::invocation::{
        BoardActorSelection, BoardCategorySelection, OfficeBoardOperation, OfficeOperation,
    };
    assert!(
        matches!(parsed(&["office", "board", "list", "--room", "Design review"]).invocation,
        Invocation::Office { operation: OfficeOperation::Board(OfficeBoardOperation::List {
            category: BoardCategorySelection::Room(name), ..
        }), .. } if name == "Design review")
    );
    assert_eq!(
        parsed(&[
            "office",
            "board",
            "post",
            "--repo",
            "origin",
            "--identity",
            "Alice",
            "--title",
            "--literal",
            "--body",
            "line\ttext",
            "--operation-id",
            "11111111-1111-4111-8111-111111111111"
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Board(OfficeBoardOperation::Post {
                category: BoardCategorySelection::Repository("origin".into()),
                actor: BoardActorSelection::Identity(Some("Alice".into())),
                title: "--literal".into(),
                body: ContentInput::Inline("line\ttext".into()),
                operation_id: Some("11111111-1111-4111-8111-111111111111".into())
            })
        }
    );
    assert_eq!(
        parsed(&[
            "office",
            "board",
            "reply",
            "22222222-2222-4222-8222-222222222222",
            "--owner",
            "--file",
            "-"
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Board(OfficeBoardOperation::Reply {
                thread_id: "22222222-2222-4222-8222-222222222222".into(),
                actor: BoardActorSelection::Owner,
                body: ContentInput::Stdin,
                operation_id: None
            })
        }
    );
    for argv in [
        &[
            "office",
            "board",
            "post",
            "--general",
            "--owner",
            "--title",
            "t",
        ] as &[&str],
        &["office", "board", "list", "--room", "Design", "--general"],
        &[
            "office", "board", "list", "--room", "Design", "--repo", "origin",
        ],
        &["office", "board", "list", "--room"],
        &[
            "office",
            "board",
            "post",
            "--general",
            "--repo",
            "origin",
            "--owner",
            "--title",
            "t",
            "--body",
            "b",
        ],
        &[
            "office",
            "board",
            "post",
            "--general",
            "--owner",
            "--identity",
            "Alice",
            "--title",
            "t",
            "--body",
            "b",
        ],
        &[
            "office",
            "board",
            "edit",
            "11111111-1111-4111-8111-111111111111",
            "--owner",
            "--if-revision",
            "1",
        ],
        &[
            "office",
            "board",
            "edit",
            "11111111-1111-4111-8111-111111111111",
            "--owner",
            "--body",
            "b",
            "--file",
            "body.txt",
            "--if-revision",
            "1",
        ],
    ] {
        assert!(parse(&args(argv)).is_err(), "accepted {argv:?}");
    }
}

#[test]
fn office_prefix_is_scoped_to_its_subtree_and_file_inputs_are_paired() {
    use crate::invocation::OfficeOperation;
    for input in [
        vec![
            "office",
            "--prefix",
            "/prefix with spaces",
            "status",
            "--json",
        ],
        vec![
            "office",
            "status",
            "--prefix",
            "/prefix with spaces",
            "--json",
        ],
    ] {
        assert_eq!(
            parsed(&input).invocation,
            Invocation::Office {
                prefix: Some("/prefix with spaces".into()),
                operation: OfficeOperation::Status,
            }
        );
    }
    assert_eq!(
        parsed(&["office", "install", "--yes"]).invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Install {
                yes: true,
                force: false,
                archive: None,
                manifest: None,
                channel: None,
            }
        }
    );
    assert_eq!(
        parsed(&["office", "install", "--yes", "--force"]).invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Install {
                yes: true,
                force: true,
                archive: None,
                manifest: None,
                channel: None,
            }
        }
    );
    assert_eq!(
        parsed(&["office", "upgrade", "--force"]).invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Upgrade {
                force: true,
                channel: None,
            }
        }
    );
    for input in [
        vec!["office", "install", "--archive", "file"],
        vec!["office", "install", "--manifest", "file"],
        vec!["office", "status", "--yes"],
        vec!["office", "upgrade", "--channel", "beta"],
        vec!["office", "pair"],
        vec!["ls", "--prefix", "/prefix"],
    ] {
        assert_eq!(parse_error(&input).code, "USAGE_ERROR");
    }
}

#[test]
fn learn_selects_exact_bundled_guidance_without_breaking_the_core_flag() {
    assert_eq!(
        parsed(&["learn"]).invocation,
        Invocation::Learn { skill: None }
    );
    assert_eq!(
        parsed(&["learn", "--skill"]).invocation,
        Invocation::Learn {
            skill: Some("tmux-team".into())
        }
    );
    for name in [
        "tmux-team",
        "tmt-inbox",
        "tmt-office",
        "tmt-prop-create",
        "tmt-avatar-create",
    ] {
        assert_eq!(
            parsed(&["learn", "--skill", name]).invocation,
            Invocation::Learn {
                skill: Some(name.into())
            }
        );
    }
    assert_eq!(
        parse_error(&["learn", "--skill", "unknown"]).code,
        "USAGE_ERROR"
    );
}

#[test]
fn office_sync_is_install_scoped_not_active_identity_scoped() {
    use crate::invocation::OfficeOperation;
    assert_eq!(
        parsed(&["office", "sync", "--prefix", "/office", "--json"]).invocation,
        Invocation::Office {
            prefix: Some("/office".into()),
            operation: OfficeOperation::Sync
        }
    );
    for input in [
        vec!["office", "sync", "--identity", "Alice"],
        vec![
            "office",
            "sync",
            "--world",
            "https://office.example/worlds/abcdefghijklmnopqrst",
        ],
        vec!["office", "sync", "--emulator"],
    ] {
        assert_eq!(parse_error(&input).code, "USAGE_ERROR");
    }
}

#[test]
fn office_block_commands_are_typed_and_scoped() {
    use crate::invocation::{OfficeBlockOperation, OfficeBlockTarget, OfficeOperation};
    let world = "https://office.example/worlds/abcdefghijklmnopqrst";
    assert_eq!(
        parsed(&["office", "block", "show", "--world", world]).invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Block {
                target: OfficeBlockTarget {
                    world: world.into(),
                    emulator: false
                },
                identity: None,
                operation: OfficeBlockOperation::Show { block_id: None },
            },
        }
    );
    assert_eq!(
        parsed(&[
            "office",
            "block",
            "show",
            "block-123",
            "--world",
            world,
            "--identity",
            "Alice",
            "--emulator",
            "--json",
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Block {
                target: OfficeBlockTarget {
                    world: world.into(),
                    emulator: true
                },
                identity: Some("Alice".into()),
                operation: OfficeBlockOperation::Show {
                    block_id: Some("block-123".into()),
                },
            },
        }
    );
    assert_eq!(
        parsed(&[
            "office",
            "block",
            "apply",
            "block-123",
            "--world",
            world,
            "--file",
            "layout.json",
            "--if-revision",
            "7",
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Block {
                target: OfficeBlockTarget {
                    world: world.into(),
                    emulator: false
                },
                identity: None,
                operation: OfficeBlockOperation::Apply {
                    block_id: Some("block-123".into()),
                    file: "layout.json".into(),
                    if_revision: 7,
                },
            },
        }
    );
    let max_revision = tmt_core::office_block::MAX_REVISION.to_string();
    let max_minus_one = (tmt_core::office_block::MAX_REVISION - 1).to_string();
    assert_eq!(
        parsed(&[
            "office",
            "block",
            "apply",
            "--world",
            world,
            "--file",
            "layout.json",
            "--if-revision",
            &max_minus_one,
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Block {
                target: OfficeBlockTarget {
                    world: world.into(),
                    emulator: false
                },
                identity: None,
                operation: OfficeBlockOperation::Apply {
                    block_id: None,
                    file: "layout.json".into(),
                    if_revision: tmt_core::office_block::MAX_REVISION - 1,
                },
            },
        }
    );
    for input in [
        vec!["office", "block", "show"],
        vec![
            "office",
            "block",
            "apply",
            "--world",
            world,
            "--file",
            "layout.json",
        ],
        vec![
            "office",
            "block",
            "apply",
            "--world",
            world,
            "--if-revision",
            "7",
        ],
        vec![
            "office",
            "block",
            "apply",
            "--world",
            world,
            "--file",
            "layout.json",
            "--if-revision",
            "-1",
        ],
        vec![
            "office",
            "block",
            "apply",
            "--world",
            world,
            "--file",
            "layout.json",
            "--if-revision",
            &max_revision,
        ],
        vec![
            "office",
            "block",
            "show",
            "--world",
            world,
            "--file",
            "layout.json",
        ],
    ] {
        assert_eq!(
            parse_error(&input).code,
            "USAGE_ERROR",
            "arguments: {input:?}"
        );
    }

    for input in [
        vec!["office", "block", "show", "--local", "--identity", "Alice"],
        vec!["office", "block", "show", "--local", "--lobby"],
        vec!["office", "block", "show", "--lobby"],
        vec![
            "office",
            "block",
            "show",
            "--local",
            "--lobby",
            "--identity",
            "Alice",
        ],
        vec!["office", "block", "show", "--local", "--lobby", "block-id"],
        vec![
            "office", "block", "show", "--local", "--lobby", "--world", world,
        ],
        vec![
            "office",
            "block",
            "show",
            "--local",
            "--lobby",
            "--emulator",
        ],
    ] {
        assert_eq!(
            parse_error(&input).code,
            "USAGE_ERROR",
            "arguments: {input:?}"
        );
    }
    assert_eq!(
        parsed(&["office", "start", "--port", "18457"]).invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Start { port: Some(18457) },
        }
    );
    assert_eq!(
        parsed(&["office", "stop"]).invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Stop
        }
    );
    for input in [
        vec!["office", "block", "show", "--local", "--world", world],
        vec!["office", "block", "show", "block-123", "--local"],
        vec!["office", "block", "show", "--local", "--emulator"],
    ] {
        assert_eq!(
            parse_error(&input).code,
            "USAGE_ERROR",
            "arguments: {input:?}"
        );
    }
}

#[test]
fn office_profile_commands_are_local_typed_and_revision_bounded() {
    use crate::invocation::{OfficeOperation, OfficeProfileOperation};
    assert_eq!(
        parse(&args(&[
            "office",
            "profile",
            "show",
            "--local",
            "--identity",
            "Alice",
            "--json"
        ]))
        .unwrap(),
        Parsed {
            invocation: Invocation::Office {
                operation: OfficeOperation::Profile {
                    identity: Some("Alice".into()),
                    operation: OfficeProfileOperation::Show
                },
                prefix: None
            },
            mode: OutputMode { json: true }
        }
    );
    assert_eq!(
        parse(&args(&[
            "office",
            "profile",
            "apply",
            "--local",
            "--identity",
            "Alice",
            "--file",
            "profile.json",
            "--if-revision",
            "0",
            "--json"
        ]))
        .unwrap()
        .invocation,
        Invocation::Office {
            operation: OfficeOperation::Profile {
                identity: Some("Alice".into()),
                operation: OfficeProfileOperation::Apply {
                    file: "profile.json".into(),
                    if_revision: 0
                }
            },
            prefix: None
        }
    );
    let maximum = tmt_core::office_profile::MAX_REVISION.to_string();
    assert_eq!(
        parsed(&[
            "office",
            "profile",
            "apply",
            "--local",
            "--file",
            "profile.json",
            "--if-revision",
            &maximum,
        ])
        .invocation,
        Invocation::Office {
            operation: OfficeOperation::Profile {
                identity: None,
                operation: OfficeProfileOperation::Apply {
                    file: "profile.json".into(),
                    if_revision: tmt_core::office_profile::MAX_REVISION,
                },
            },
            prefix: None,
        }
    );
    let above_maximum = (tmt_core::office_profile::MAX_REVISION + 1).to_string();
    for invalid in [
        vec!["office", "profile", "show"],
        vec![
            "office",
            "profile",
            "show",
            "--world",
            "https://example.test",
        ],
        vec![
            "office",
            "profile",
            "apply",
            "--local",
            "--file",
            "p.json",
            "--if-revision",
            &above_maximum,
        ],
    ] {
        assert!(parse(&args(&invalid)).is_err());
    }
}

#[test]
fn office_prop_commands_have_exact_local_and_revision_grammar() {
    use crate::invocation::{OfficeOperation, OfficePropOperation};
    assert_eq!(
        parsed(&[
            "office",
            "prop",
            "install",
            "--local",
            "--file",
            "pack.tmtprop.json",
            "--if-revision",
            "7",
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Prop(OfficePropOperation::Install {
                file: "pack.tmtprop.json".into(),
                if_revision: 7,
            }),
        }
    );
    assert_eq!(
        parsed(&[
            "office", "prop", "list", "--local", "--limit", "1", "--cursor", "opaque",
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Prop(OfficePropOperation::List {
                limit: 1,
                cursor: Some("opaque".into()),
            }),
        }
    );
    assert_eq!(
        parsed(&["office", "prop", "preview", "--file", "pack.tmtprop.json",]).invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Prop(OfficePropOperation::Preview {
                file: "pack.tmtprop.json".into(),
            }),
        }
    );
    for invalid in [
        vec![
            "office",
            "prop",
            "install",
            "--file",
            "p",
            "--if-revision",
            "0",
        ],
        vec!["office", "prop", "show", "sha256:x"],
        vec!["office", "prop", "list", "--local", "--limit", "0"],
        vec!["office", "prop", "list", "--local", "--limit", "21"],
    ] {
        assert!(parse(&args(&invalid)).is_err(), "{invalid:?}");
    }
}

#[test]
fn extension_preflight_requires_both_named_files_and_rejects_execution_flags() {
    use crate::invocation::OfficeOperation;
    assert_eq!(
        parsed(&[
            "office",
            "extension",
            "validate",
            "--file",
            "definition.json",
            "--instance",
            "instance.json"
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::ExtensionValidate {
                file: "definition.json".into(),
                instance: "instance.json".into()
            }
        }
    );
    for invalid in [
        vec![
            "office",
            "extension",
            "validate",
            "--file",
            "definition.json",
        ],
        vec![
            "office",
            "extension",
            "validate",
            "--instance",
            "instance.json",
        ],
        vec![
            "office",
            "extension",
            "validate",
            "--file",
            "definition.json",
            "--instance",
            "instance.json",
            "--execute",
        ],
        vec![
            "office",
            "extension",
            "install",
            "--file",
            "definition.json",
        ],
    ] {
        assert!(parse(&args(&invalid)).is_err(), "{invalid:?}");
    }
}

#[test]
fn office_avatar_commands_have_exact_local_and_revision_grammar() {
    use crate::invocation::{OfficeAvatarOperation, OfficeOperation};
    assert_eq!(
        parsed(&[
            "office",
            "avatar",
            "install",
            "--local",
            "--file",
            "bot.tmtavatar.json",
            "--if-revision",
            "7",
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Avatar(OfficeAvatarOperation::Install {
                file: "bot.tmtavatar.json".into(),
                if_revision: 7,
            }),
        }
    );
    assert_eq!(
        parsed(&[
            "office", "avatar", "list", "--local", "--limit", "1", "--cursor", "opaque",
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Avatar(OfficeAvatarOperation::List {
                limit: 1,
                cursor: Some("opaque".into()),
            }),
        }
    );
    assert_eq!(
        parsed(&[
            "office",
            "avatar",
            "preview",
            "--file",
            "bot.tmtavatar.json"
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::Avatar(OfficeAvatarOperation::Preview {
                file: "bot.tmtavatar.json".into(),
            }),
        }
    );
    for invalid in [
        vec![
            "office",
            "avatar",
            "install",
            "--file",
            "p",
            "--if-revision",
            "0",
        ],
        vec!["office", "avatar", "show", "sha256:x"],
        vec!["office", "avatar", "list", "--local", "--limit", "0"],
        vec!["office", "avatar", "list", "--local", "--limit", "21"],
    ] {
        assert!(parse(&args(&invalid)).is_err(), "{invalid:?}");
    }
}

#[test]
fn native_upgrade_alias_and_selection_share_one_typed_contract() {
    for command in ["upgrade", "update"] {
        let invocation = parsed(&[
            command,
            "--channel",
            "alpha",
            "--to",
            "5.0.0-alpha.3",
            "--json",
        ]);
        assert!(invocation.mode.json);
        assert_eq!(
            invocation.invocation,
            Invocation::Upgrade {
                channel: Some(tmt_core::native_install::Channel::Alpha),
                exact: Some("5.0.0-alpha.3".into()),
                unpin: false,
            }
        );
        assert_eq!(
            parsed(&[command, "--unpin"]).invocation,
            Invocation::Upgrade {
                channel: None,
                exact: None,
                unpin: true
            }
        );
        assert_eq!(
            parse_error(&[command, "--to", "5.0.0", "--unpin", "--json"]).code,
            "USAGE_ERROR"
        );
        assert_eq!(
            parse_error(&[command, "--channel", "beta", "--json"]).code,
            "USAGE_ERROR"
        );
    }
}

#[test]
fn internal_native_install_requires_explicit_inputs_and_typed_pin_policy() {
    use tmt_core::native_install::{Channel, PinAction};
    let input = [
        "__native-install",
        "--archive",
        "archive.tar.gz",
        "--manifest",
        "manifest.json",
        "--prefix",
        "/prefix with spaces",
        "--channel",
        "alpha",
        "--json",
    ];
    let parsed = parsed(&input);
    assert!(parsed.mode.json);
    assert_eq!(
        parsed.invocation,
        Invocation::NativeInstall {
            product: tmt_core::native_install::Product::Cli,
            archive: "archive.tar.gz".into(),
            manifest: "manifest.json".into(),
            prefix: "/prefix with spaces".into(),
            channel: Channel::Alpha,
            pin: PinAction::Preserve,
        }
    );
    for pin in ["--pin", "--unpin"] {
        let mut args = input.to_vec();
        args.push(pin);
        let result = super::parse(&self::args(&args)).unwrap();
        assert!(
            matches!(result.invocation, Invocation::NativeInstall { pin: actual, .. }
            if actual == if pin == "--pin" { PinAction::PinCandidate } else { PinAction::Clear })
        );
    }
    let mut office = input.to_vec();
    office.extend(["--product", "office"]);
    assert!(matches!(
        super::parse(&self::args(&office)).unwrap().invocation,
        Invocation::NativeInstall {
            product: tmt_core::native_install::Product::Office,
            ..
        }
    ));
    let mut invalid_product = input.to_vec();
    invalid_product.extend(["--product", "third-party"]);
    assert_eq!(parse_error(&invalid_product).code, "USAGE_ERROR");
    let mut conflict = input.to_vec();
    conflict.extend(["--pin", "--unpin"]);
    assert_eq!(parse_error(&conflict).code, "USAGE_ERROR");
    assert_eq!(parse_error(&["__native-install"]).code, "USAGE_ERROR");
    let help = crate::grammar::public_grammar(&crate::grammar::grammar(), true);
    assert!(help.find_subcommand("__native-install").is_none());
}

#[test]
fn negative_config_values_reach_setting_validation_without_accepting_unknown_flags() {
    let invocation = parsed(&["config", "set", "preambleEvery", "-1", "--json"]);
    assert_eq!(
        invocation.invocation,
        Invocation::Config(crate::invocation::ConfigRequest::Set {
            key: "preambleEvery".into(),
            value: "-1".into(),
            global: false,
        })
    );
    assert!(invocation.mode.json);
    assert_eq!(
        parse_error(&["config", "set", "preambleEvery", "--unknown", "--json"]).code,
        "USAGE_ERROR"
    );
}

#[test]
fn notes_path_has_one_typed_identity_selector_and_json_mode() {
    assert_eq!(
        parsed(&["notes", "path", "--identity", "Research & QA", "--json"]),
        Parsed {
            invocation: Invocation::NotesPath {
                identity: Some("Research & QA".into()),
            },
            mode: OutputMode { json: true },
        }
    );
    assert_eq!(
        parsed(&["notes", "path"]).invocation,
        Invocation::NotesPath { identity: None }
    );
    assert_eq!(parse_error(&["notes"]).code, "USAGE_ERROR");
    assert_eq!(
        parse_error(&["notes", "path", "unexpected"]).code,
        "USAGE_ERROR"
    );
    assert_eq!(
        parse_error(&["notes", "path", "--force"]).code,
        "USAGE_ERROR"
    );
}

fn parsed(values: &[&str]) -> Parsed {
    parse(&args(values)).unwrap_or_else(|error| {
        panic!("expected {:?} to parse, got {error:?}", values);
    })
}

#[test]
fn preamble_operands_join_without_reparsing_literal_content() {
    use crate::invocation::PreambleRequest;
    for (args, content) in [
        (
            vec!["preamble", "set", "Alice", "first", "second"],
            "first second",
        ),
        (
            vec!["preamble", "set", "Alice", "--", "--json", "literal"],
            "--json literal",
        ),
    ] {
        let parsed = parsed(&args);
        assert_eq!(
            parsed.invocation,
            Invocation::Preamble(PreambleRequest::Set {
                name: "Alice".into(),
                content: content.into()
            })
        );
        assert!(!parsed.mode.json);
    }
    assert_eq!(
        parsed(&["preamble"]).invocation,
        Invocation::Preamble(PreambleRequest::Show(None))
    );
    assert_eq!(
        parsed(&["preamble", "show", "Alice"]).invocation,
        Invocation::Preamble(PreambleRequest::Show(Some("Alice".into())))
    );
}

fn parse_error(values: &[&str]) -> ParseError {
    parse(&args(values)).expect_err("expected arguments to be rejected")
}

fn assert_usage_error(values: &[&str], message: &str, mode: OutputMode) {
    let error = parse_error(values);
    assert_eq!(error.code, "USAGE_ERROR", "arguments: {values:?}");
    assert_eq!(error.mode, mode, "arguments: {values:?}");
    assert!(
        error.message.contains(message),
        "arguments: {values:?}; expected message containing {message:?}, got {:?}",
        error.message
    );
}

#[test]
fn command_aliases_preserve_typed_invocations() {
    let cases = [
        (
            "ls alias",
            vec!["ls"],
            Invocation::List {
                target: None,
                room: None,
            },
        ),
        (
            "this alias",
            vec!["this", "Alice"],
            Invocation::Bind {
                pane: None,
                name: "Alice".into(),
                save: false,
            },
        ),
        (
            "remove alias",
            vec!["remove", "Alice"],
            Invocation::Remove {
                name: "Alice".into(),
                force: false,
            },
        ),
        (
            "send alias",
            vec!["send", "peer", "hello"],
            Invocation::Talk {
                target: "peer".into(),
                message: "hello".into(),
                originator: None,
                options: TalkOptions {
                    room: None,
                    inbox: false,
                    force: false,
                    detach: false,
                    delay_seconds: None,
                    timeout_seconds: None,
                    no_preamble: false,
                },
            },
        ),
        (
            "read alias",
            vec!["read", "peer", "0"],
            Invocation::Check {
                target: "peer".into(),
                lines: Some(0),
            },
        ),
    ];

    for (name, argv, invocation) in cases {
        let actual = parsed(&argv);
        assert_eq!(actual.invocation, invocation, "{name}");
        assert_eq!(actual.mode, OutputMode::default(), "{name} mode");
    }
}

#[test]
fn save_and_force_flags_have_false_defaults_and_short_aliases() {
    assert_eq!(
        parsed(&["add", "%1", "Alice"]).invocation,
        Invocation::Bind {
            pane: Some("%1".into()),
            name: "Alice".into(),
            save: false,
        }
    );
    assert_eq!(
        parsed(&["add", "%1", "Alice", "-s"]).invocation,
        Invocation::Bind {
            pane: Some("%1".into()),
            name: "Alice".into(),
            save: true,
        }
    );
    assert_eq!(
        parsed(&["name", "Alice", "--save"]).invocation,
        Invocation::Bind {
            pane: None,
            name: "Alice".into(),
            save: true,
        }
    );
    assert_eq!(
        parsed(&["marked", "Alice"]).invocation,
        Invocation::BindMarked {
            name: "Alice".into(),
            save: false,
        }
    );
    assert_eq!(
        parsed(&["marked", "Alice", "-s"]).invocation,
        Invocation::BindMarked {
            name: "Alice".into(),
            save: true,
        }
    );
    assert_eq!(
        parsed(&["rm", "Alice"]).invocation,
        Invocation::Remove {
            name: "Alice".into(),
            force: false,
        }
    );
    assert_eq!(
        parsed(&["remove", "Alice", "-f"]).invocation,
        Invocation::Remove {
            name: "Alice".into(),
            force: true,
        }
    );
}

#[test]
fn literal_option_words_remain_data_when_the_grammar_requires_values() {
    assert_eq!(
        parsed(&["talk", "peer", "--", "--json --debug"]).invocation,
        Invocation::Talk {
            target: "peer".into(),
            message: "--json --debug".into(),
            originator: None,
            options: TalkOptions {
                room: None,
                inbox: false,
                force: false,
                detach: false,
                delay_seconds: None,
                timeout_seconds: None,
                no_preamble: false,
            },
        }
    );
    assert_eq!(
        parsed(&[
            "reply",
            "request-1",
            "--receipt",
            "receipt",
            "--message=--json"
        ])
        .invocation,
        Invocation::Reply {
            request_id: "request-1".into(),
            receipt: "receipt".into(),
            input: ContentInput::Inline("--json".into()),
        }
    );
}

#[test]
fn root_and_command_local_options_work_before_and_after_the_command() {
    assert_eq!(
        parsed(&["--json", "talk", "peer", "hello", "--detach"]).mode,
        OutputMode { json: true }
    );
    assert_eq!(
        parsed(&["talk", "peer", "hello", "--json", "--no-preamble"]),
        Parsed {
            invocation: Invocation::Talk {
                target: "peer".into(),
                message: "hello".into(),
                originator: None,
                options: TalkOptions {
                    room: None,
                    inbox: false,
                    force: false,
                    detach: false,
                    delay_seconds: None,
                    timeout_seconds: None,
                    no_preamble: true,
                },
            },
            mode: OutputMode { json: true },
        }
    );
    assert_eq!(
        parsed(&["--timeout", "2", "send", "peer", "hello"]).invocation,
        Invocation::Talk {
            target: "peer".into(),
            message: "hello".into(),
            originator: None,
            options: TalkOptions {
                room: None,
                inbox: false,
                force: false,
                detach: false,
                delay_seconds: None,
                timeout_seconds: Some(2.0),
                no_preamble: false,
            },
        }
    );
}

#[test]
fn removed_output_flags_are_rejected_but_remain_literal_payload_data() {
    for option in ["--verbose", "-v", "--debug"] {
        for argv in [
            vec![option, "list", "--json"],
            vec!["--json", "list", option],
            vec!["identity", "create", "Agent", option, "--json"],
            vec!["--json", "--version", option],
        ] {
            assert_usage_error(&argv, option, OutputMode { json: true });
        }
    }
    assert_eq!(
        parsed(&["talk", "peer", "--", "--verbose --debug -v"]).invocation,
        Invocation::Talk {
            target: "peer".into(),
            message: "--verbose --debug -v".into(),
            originator: None,
            options: TalkOptions {
                room: None,
                inbox: false,
                force: false,
                detach: false,
                delay_seconds: None,
                timeout_seconds: None,
                no_preamble: false,
            },
        }
    );
}

#[test]
fn command_local_options_are_rejected_at_the_wrong_boundary() {
    for (argv, text) in [
        (
            &["list", "--force"][..],
            "Unknown option or argument 'force'",
        ),
        (
            &["role", "show", "--force"][..],
            "Unknown option or argument 'force'",
        ),
        (
            &["list", "--delay", "1"][..],
            "Unknown option or argument 'delay'",
        ),
        (
            &["role", "show", "--no-preamble"][..],
            "Unknown option or argument 'no-preamble'",
        ),
        (
            &["read", "peer", "--timeout", "2"][..],
            "Unknown option or argument 'timeout'",
        ),
    ] {
        assert_usage_error(argv, text, OutputMode::default());
    }
}

#[test]
fn missing_arguments_keep_json_mode_before_or_after_the_command() {
    for argv in [
        &["--json", "name"][..],
        &["name", "--json"][..],
        &["--json", "talk", "peer"][..],
        &["talk", "peer", "--json"][..],
    ] {
        assert_usage_error(argv, "required", OutputMode { json: true });
    }
}

#[test]
fn local_missing_values_do_not_swallow_root_presentation_flags() {
    for argv in [
        &["talk", "peer", "--identity", "--json", "hello"][..],
        &["talk", "peer", "--identity", "--json", "hello", "--nope"][..],
        &[
            "no-command",
            "talk",
            "peer",
            "--identity",
            "--json",
            "--nope",
        ][..],
    ] {
        let error = if argv[0] == "no-command" {
            // A missing external executable falls back to this core diagnostic;
            // an installed extension owns its entire tail instead.
            crate::parser::parse_core(&args(argv)).unwrap_err()
        } else {
            parse_error(argv)
        };
        assert_eq!(error.code, "USAGE_ERROR", "{argv:?}");
        assert!(error.mode.json, "{argv:?}: {error:?}");
    }
    let result = parsed(&["talk", "peer", "--identity=--json", "hello"]);
    assert!(!result.mode.json);
    let Invocation::Talk {
        originator,
        message,
        ..
    } = result.invocation
    else {
        panic!("expected talk request");
    };
    assert_eq!(originator.as_deref(), Some("--json"));
    assert_eq!(message, "hello");
    let error = parse_error(&["learn", "--config", "--json"]);
    assert!(!error.mode.json);
    assert!(error.message.contains("config"));
}

#[test]
fn retired_wait_and_team_paths_have_distinct_errors() {
    let wait = parse_error(&["talk", "peer", "hello", "--wait"]);
    assert_eq!(wait.code, "USAGE_ERROR");
    assert_eq!(wait.mode, OutputMode::default());
    assert!(wait.message.contains("--wait option is retired"));

    let team = parse_error(&["team", "legacy"]);
    assert_eq!(team.code, "UNSUPPORTED_TEAM");
    assert_eq!(team.mode, OutputMode::default());
    assert_eq!(team.message, "Team workflows are not supported.");

    let scoped = parse_error(&["--json", "--team", "legacy", "list"]);
    assert_eq!(scoped.code, "UNSUPPORTED_TEAM");
    assert_eq!(scoped.mode, OutputMode { json: true });
}

#[test]
fn identity_show_selects_a_name_or_the_verified_caller() {
    assert_eq!(
        parsed(&["identity", "show"]).invocation,
        Invocation::Identity(IdentityRequest::Show(None))
    );
    assert_eq!(
        parsed(&["identity", "show", "Alice"]).invocation,
        Invocation::Identity(IdentityRequest::Show(Some("Alice".into())))
    );
}

#[test]
fn rejected_placement_option_reports_public_usage() {
    let rejected_option = parse_error(&["role", "show", "--timeout", "1s"]);
    assert_eq!(rejected_option.code, "USAGE_ERROR");
    assert!(rejected_option.message.contains("Usage: tmt role show"));
    assert!(rejected_option.message.contains("tmt help role show"));
}

#[test]
fn identity_metadata_and_repeated_filters_have_typed_requests() {
    assert_eq!(
        parsed(&[
            "identity",
            "meta",
            "set",
            "--identity",
            "alice",
            "project",
            "tmt",
        ])
        .invocation,
        Invocation::Identity(IdentityRequest::Metadata {
            identity: Some("alice".into()),
            operation: IdentityMetadataRequest::Set {
                key: "project".into(),
                value: "tmt".into(),
            },
        })
    );
    assert_eq!(
        parsed(&[
            "identity",
            "list",
            "--where",
            "project=tmt=alpha",
            "--has",
            "capability.review",
            "--where",
            "department=engineering",
        ])
        .invocation,
        Invocation::Identity(IdentityRequest::List(vec![
            IdentityFilterRequest::Equals {
                key: "project".into(),
                value: "tmt=alpha".into(),
            },
            IdentityFilterRequest::Equals {
                key: "department".into(),
                value: "engineering".into(),
            },
            IdentityFilterRequest::Has("capability.review".into()),
        ]))
    );
    assert_usage_error(
        &["identity", "list", "--where", "project"],
        "KEY=VALUE",
        OutputMode::default(),
    );
    let Invocation::Identity(IdentityRequest::Metadata {
        operation: IdentityMetadataRequest::Set { value, .. },
        ..
    }) = parsed(&[
        "identity",
        "meta",
        "set",
        "--identity",
        "alice",
        "key",
        "--",
        "--literal-value",
    ])
    .invocation
    else {
        panic!("expected metadata set request")
    };
    assert_eq!(value, "--literal-value");
}

#[test]
fn timing_values_accept_exact_boundaries_and_reject_invalid_values() {
    let timeout = parsed(&["talk", "peer", "hello", "--timeout", "86400s"]);
    assert_eq!(
        timeout.invocation,
        Invocation::Talk {
            target: "peer".into(),
            message: "hello".into(),
            originator: None,
            options: TalkOptions {
                room: None,
                inbox: false,
                force: false,
                detach: false,
                delay_seconds: None,
                timeout_seconds: Some(86_400.0),
                no_preamble: false,
            },
        }
    );
    let delay = parsed(&["talk", "peer", "hello", "--delay", "2147483647ms"]);
    assert_eq!(
        delay.invocation,
        Invocation::Talk {
            target: "peer".into(),
            message: "hello".into(),
            originator: None,
            options: TalkOptions {
                room: None,
                inbox: false,
                force: false,
                detach: false,
                delay_seconds: Some(2_147_483.647),
                timeout_seconds: None,
                no_preamble: false,
            },
        }
    );

    for (argv, text) in [
        (
            &["talk", "peer", "hello", "--timeout", "86400.001s"][..],
            "24 hours",
        ),
        (
            &["talk", "peer", "hello", "--delay", "2147483648ms"][..],
            "timer limit",
        ),
        (
            &["talk", "peer", "hello", "--timeout", "-1"][..],
            "Invalid time format",
        ),
        (
            &["talk", "peer", "hello", "--timeout", "NaN"][..],
            "Invalid time format",
        ),
    ] {
        assert_usage_error(argv, text, OutputMode::default());
    }

    let detached = parsed(&["talk", "peer", "hello", "--detach"]);
    assert_eq!(
        detached.invocation,
        Invocation::Talk {
            target: "peer".into(),
            message: "hello".into(),
            originator: None,
            options: TalkOptions {
                room: None,
                inbox: false,
                force: false,
                detach: true,
                delay_seconds: None,
                timeout_seconds: None,
                no_preamble: false,
            },
        }
    );
    assert_usage_error(
        &["talk", "peer", "hello", "--detach", "--timeout", "1s"],
        "either --timeout or --detach",
        OutputMode::default(),
    );
}

#[test]
fn inbox_and_listener_options_have_typed_defaults_and_minutes() {
    let talk = parsed(&["talk", "Receiver", "hello", "--inbox", "--detach"]);
    let Invocation::Talk { options, .. } = talk.invocation else {
        panic!("talk invocation")
    };
    assert!(options.inbox);

    assert_eq!(
        parsed(&["x", "listen", "--identity", "Receiver"]).invocation,
        Invocation::Exchange {
            identity: Some("Receiver".into()),
            operation: ExchangeOperation::Listen {
                room: None,
                timeout_seconds: 900.0,
                debounce_seconds: 10.0
            },
        }
    );
    assert_eq!(
        parsed(&["x", "listen", "--timeout", "1.5m", "--debounce", "250MS"]).invocation,
        Invocation::Exchange {
            identity: None,
            operation: ExchangeOperation::Listen {
                room: None,
                timeout_seconds: 90.0,
                debounce_seconds: 0.25
            },
        }
    );
    for value in ["0", "86400.1s"] {
        assert_usage_error(
            &["x", "listen", "--timeout", value],
            "Listen timeout",
            OutputMode::default(),
        );
    }
    for value in ["NaN", "1h"] {
        assert_usage_error(
            &["x", "listen", "--timeout", value],
            "Invalid time format",
            OutputMode::default(),
        );
    }
}

#[test]
fn capture_and_exchange_integers_accept_exact_boundaries() {
    assert_eq!(
        parsed(&["check", "peer", "0"]).invocation,
        Invocation::Check {
            target: "peer".into(),
            lines: Some(0),
        }
    );
    assert_eq!(
        parsed(&["check", "peer", "--lines", "2147483647"]).invocation,
        Invocation::Check {
            target: "peer".into(),
            lines: Some(2_147_483_647),
        }
    );
    assert_eq!(
        parsed(&["x", "list", "--limit", "200", "--after", "9007199254740991"]).invocation,
        Invocation::Exchange {
            identity: None,
            operation: ExchangeOperation::List {
                limit: Some(200),
                after: Some(9_007_199_254_740_991),
            },
        }
    );
    assert_eq!(
        parsed(&["x", "ack", "request-1", "--revision", "9007199254740991"]).invocation,
        Invocation::Exchange {
            identity: None,
            operation: ExchangeOperation::Ack {
                request_id: "request-1".into(),
                revision: 9_007_199_254_740_991,
                incoming: false,
            },
        }
    );

    for argv in [
        &["check", "peer", "2147483648"][..],
        &["check", "peer", "--lines", "2147483648"][..],
        &["check", "peer", "--lines", "1.5"][..],
        &["check", "peer", "--lines", "-1"][..],
        &["--lines", "2147483648", "check", "peer", "0"][..],
        &["check", "peer", "0", "--lines", "1.5"][..],
        &["check", "peer", "2147483648", "--lines", "0"][..],
        &["x", "list", "--after", "9007199254740992"][..],
        &["x", "ack", "request-1", "--revision", "0"][..],
    ] {
        assert_eq!(parse_error(argv).code, "USAGE_ERROR", "arguments: {argv:?}");
    }
    assert_eq!(
        parsed(&["--lines", "12", "check", "peer", "0"]).invocation,
        Invocation::Check {
            target: "peer".into(),
            lines: Some(0)
        }
    );
}

#[test]
fn input_sources_are_mutually_exclusive_and_required() {
    assert_eq!(
        parsed(&["reply", "request-1", "--receipt", "receipt", "--stdin",]).invocation,
        Invocation::Reply {
            request_id: "request-1".into(),
            receipt: "receipt".into(),
            input: ContentInput::Stdin,
        }
    );
    assert_eq!(
        parsed(&["role", "set", "profile text", "--identity", "Alice",]).invocation,
        Invocation::Role {
            identity: Some("Alice".into()),
            operation: RoleOperation::Set(ContentInput::Inline("profile text".into())),
        }
    );

    for (argv, text) in [
        (
            &["reply", "request-1", "--receipt", "receipt"][..],
            "exactly one inline content",
        ),
        (
            &[
                "reply",
                "request-1",
                "--receipt",
                "receipt",
                "--message",
                "inline",
                "--file",
                "/tmp/response.txt",
            ][..],
            "exactly one inline content",
        ),
        (
            &["role", "set", "inline", "--file", "/tmp/profile.txt"][..],
            "exactly one inline content",
        ),
    ] {
        assert_usage_error(argv, text, OutputMode::default());
    }
}

#[test]
fn nested_exchange_and_role_options_stay_at_their_own_boundaries() {
    assert_eq!(
        parsed(&["x", "--identity", "Alice", "list", "--limit", "2"]).invocation,
        Invocation::Exchange {
            identity: Some("Alice".into()),
            operation: ExchangeOperation::List {
                limit: Some(2),
                after: None,
            },
        }
    );
    assert_eq!(
        parsed(&["x", "list", "--identity", "Alice", "--after", "0"]).invocation,
        Invocation::Exchange {
            identity: Some("Alice".into()),
            operation: ExchangeOperation::List {
                limit: None,
                after: Some(0),
            },
        }
    );
    assert_eq!(
        parsed(&["role", "--identity", "Alice", "show"]).invocation,
        Invocation::Role {
            identity: Some("Alice".into()),
            operation: RoleOperation::Show,
        }
    );
    assert_eq!(
        parsed(&["role", "show", "--identity", "Alice"]).invocation,
        Invocation::Role {
            identity: Some("Alice".into()),
            operation: RoleOperation::Show,
        }
    );

    for (argv, text) in [
        (
            &["x", "list", "--force"][..],
            "Unknown option or argument 'force'",
        ),
        (
            &["role", "show", "--limit", "2"][..],
            "unexpected argument '--limit'",
        ),
        (
            &["role", "show", "--file", "/tmp/profile.txt"][..],
            "unexpected argument '--file'",
        ),
    ] {
        assert_usage_error(argv, text, OutputMode::default());
    }
}
