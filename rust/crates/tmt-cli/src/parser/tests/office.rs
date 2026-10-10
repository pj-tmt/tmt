use super::*;

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
    let max_revision = tmt_office_model::office_block::MAX_REVISION.to_string();
    let max_minus_one = (tmt_office_model::office_block::MAX_REVISION - 1).to_string();
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
                    if_revision: tmt_office_model::office_block::MAX_REVISION - 1,
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
            legacy_hook: false,
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
    let maximum = tmt_office_model::office_profile::MAX_REVISION.to_string();
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
                    if_revision: tmt_office_model::office_profile::MAX_REVISION,
                },
            },
            prefix: None,
        }
    );
    let above_maximum = (tmt_office_model::office_profile::MAX_REVISION + 1).to_string();
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
