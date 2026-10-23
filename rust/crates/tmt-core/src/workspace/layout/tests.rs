use super::*;

#[test]
fn skipped_session_confers_no_reuse_authority_for_its_shared_window() {
    let mut saved = snapshot();
    let mut missing = saved.sessions[0].clone();
    missing.id = "$2".into();
    missing.name = "missing".into();
    missing.windows[0].index = 9;
    saved.sessions.push(missing);
    let made = LiveSession::Restored {
        name: "workspace".into(),
        marker: restored_marker(&saved, &saved.sessions[0]),
    };
    let plan = layout_creation(&saved, &[made]).unwrap();
    assert!(matches!(
        plan.sessions[0].action,
        WorkspaceLayoutSessionAction::SkipExisting
    ));
    let WorkspaceLayoutSessionAction::Create {
        seed_window,
        bootstrap_index,
        links,
        ..
    } = &plan.sessions[1].action
    else {
        panic!("missing session must create its own window")
    };
    assert_eq!(*seed_window, Some(0));
    assert_eq!(*bootstrap_index, None);
    assert!(links[0].create);
    assert_eq!(links[0].index, 9);
}

#[test]
fn all_shared_session_reserves_an_unused_bootstrap_index() {
    let mut saved = snapshot();
    let mut linked = saved.sessions[0].clone();
    linked.id = "$2".into();
    linked.name = "linked".into();
    linked.windows[0].index = 0;
    saved.sessions.push(linked);
    let plan = layout_creation(&saved, &[]).unwrap();
    let WorkspaceLayoutSessionAction::Create {
        seed_window,
        bootstrap_index,
        links,
        ..
    } = &plan.sessions[1].action
    else {
        panic!("new shared session")
    };
    assert_eq!(*seed_window, None);
    assert_eq!(*bootstrap_index, Some(1));
    assert!(!links[0].create);
    assert_eq!(links[0].window, 0);
    assert!(links[0].active);
}

#[test]
fn session_with_a_new_window_uses_it_as_seed_without_disposable_panes() {
    let mut saved = snapshot();
    let mut new_window = saved.windows[0].clone();
    new_window.id = "@2".into();
    new_window.active_pane = "%9".into();
    new_window.layout = with_checksum("80x24,0,0,9");
    new_window.visible_layout = new_window.layout.clone();
    saved.windows.push(new_window);
    let mut new_pane = saved.panes[0].clone();
    new_pane.id = "%9".into();
    new_pane.window = "@2".into();
    new_pane.left = 0;
    new_pane.width = 80;
    saved.panes.push(new_pane);
    let mut linked = saved.sessions[0].clone();
    linked.id = "$2".into();
    linked.name = "linked".into();
    linked.windows.push(crate::workspace::WindowLink {
        index: 9,
        window: "@2".into(),
        active: false,
    });
    saved.sessions.push(linked);
    let plan = layout_creation(&saved, &[]).unwrap();
    let WorkspaceLayoutSessionAction::Create {
        seed_window,
        bootstrap_index,
        links,
        ..
    } = &plan.sessions[1].action
    else {
        panic!("new shared session")
    };
    assert_eq!(*seed_window, Some(1));
    assert_eq!(*bootstrap_index, None);
    assert!(!links[0].create);
    assert!(links[1].create);
}

#[test]
fn duplicate_window_link_is_refused_before_native_effects() {
    let mut saved = snapshot();
    let mut duplicate = saved.sessions[0].windows[0].clone();
    duplicate.index = 9;
    duplicate.active = false;
    saved.sessions[0].windows.push(duplicate);
    assert!(matches!(
        layout_creation(&saved, &[]),
        Err(WorkspaceLayoutError::Topology)
    ));
}

#[test]
fn splits_unequal_siblings_from_remaining_extent_without_default_halves() {
    let root = parse_layout(&with_checksum(
        "100x24,0,0{5x24,0,0,7,10x24,6,0,8,83x24,17,0,9}",
    ))
    .unwrap();
    let mut splits = Vec::new();
    append_splits(&root, &mut splits).unwrap();
    assert_eq!(
        splits,
        [
            WorkspaceLayoutSplit {
                target_leaf: 7,
                new_leaf: 8,
                axis: WorkspaceSplitAxis::Horizontal,
                remaining_extent: 94,
            },
            WorkspaceLayoutSplit {
                target_leaf: 8,
                new_leaf: 9,
                axis: WorkspaceSplitAxis::Horizontal,
                remaining_extent: 83,
            },
        ]
    );
}

#[test]
fn nested_split_sequence_preserves_native_leaf_order_and_cwd_correspondence() {
    let root = parse_layout(&with_checksum(
        "100x24,0,0{20x24,0,0[20x3,0,0,7,20x20,0,4,8],10x24,21,0,9,68x24,32,0[68x10,32,0,10,68x13,32,11,11]}",
    ))
    .unwrap();
    let mut splits = Vec::new();
    let first = append_splits(&root, &mut splits).unwrap();
    let mut native_order = vec![first];
    for split in &splits {
        let target = native_order
            .iter()
            .position(|id| *id == split.target_leaf)
            .unwrap();
        assert!(!native_order.contains(&split.new_leaf));
        native_order.insert(target + 1, split.new_leaf);
    }
    assert_eq!(native_order, [7, 8, 9, 10, 11]);
    assert_eq!(native_order, root.leaf_ids());
    assert_eq!(
        splits
            .iter()
            .map(|split| split.remaining_extent)
            .collect::<Vec<_>>(),
        [79, 20, 68, 13]
    );
    let saved = snapshot();
    let prepared = prepare_layout(&saved).unwrap();
    assert_eq!(prepared[0].splits[0].target_leaf, 7);
    assert_eq!(prepared[0].splits[0].new_leaf, 8);
    assert_eq!(prepared[0].panes[1].cwd, "/work/8");
}

#[test]
fn single_pane_requires_no_split_and_invalid_remaining_extent_refuses() {
    let root = parse_layout(&with_checksum("80x24,0,0,7")).unwrap();
    let mut splits = Vec::new();
    assert_eq!(append_splits(&root, &mut splits), Ok(7));
    assert!(splits.is_empty());
    let mut malformed =
        parse_layout(&with_checksum("80x24,0,0{39x24,0,0,7,40x24,40,0,8}")).unwrap();
    malformed.width = 39;
    assert_eq!(
        append_splits(&malformed, &mut splits),
        Err(WorkspaceLayoutError::Geometry)
    );
}

fn with_checksum(body: &str) -> String {
    // Independent checksum oracle, expressed as the upstream shift/add rule.
    let mut checksum = 0u16;
    for byte in body.bytes() {
        checksum = (checksum >> 1) | ((checksum & 1) << 15);
        checksum = checksum.wrapping_add(u16::from(byte));
    }
    format!("{checksum:04x},{body}")
}

#[test]
fn a_fresh_default_session_takes_the_saved_windows_after_its_own() {
    let mut saved = snapshot();
    // The saved session has two windows at 3 and 5; the fresh default's lone window is 0.
    saved.sessions[0].name = "0".into();
    saved.sessions[0]
        .windows
        .push(crate::workspace::WindowLink {
            index: 5,
            window: "@2".into(),
            active: false,
        });
    let body = "80x24,0,0{39x24,0,0,17,40x24,40,0,18}";
    let mut second = saved.windows[0].clone();
    second.id = "@2".into();
    second.layout = with_checksum(body);
    second.visible_layout = with_checksum(body);
    second.active_pane = "%18".into();
    saved.windows.push(second);
    for pane in saved.panes.clone() {
        let mut moved = pane.clone();
        moved.id = format!("%{}", pane.id[1..].parse::<u32>().unwrap() + 10);
        moved.window = "@2".into();
        saved.panes.push(moved);
    }
    let live = [LiveSession::Fresh {
        name: "0".into(),
        window_index: 0,
    }];
    let Ok(plan) = layout_creation(&saved, &live) else {
        panic!("the fixture is a consistent snapshot")
    };
    let WorkspaceLayoutSessionAction::Merge { name, links } = &plan.sessions[0].action else {
        panic!("a fresh default session takes the windows")
    };
    assert_eq!(name, "0");
    // After the live window, in saved order, whatever indexes the windows had before.
    assert_eq!(
        links.iter().map(|link| link.index).collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(links.iter().all(|link| link.create));
    assert!(links[0].active && !links[1].active);
    // A live window above the saved indexes still gets the saved windows after it.
    let high = [LiveSession::Fresh {
        name: "0".into(),
        window_index: 7,
    }];
    let plan = layout_creation(&saved, &high).unwrap();
    let WorkspaceLayoutSessionAction::Merge { links, .. } = &plan.sessions[0].action else {
        panic!("merge")
    };
    assert_eq!(
        links.iter().map(|link| link.index).collect::<Vec<_>>(),
        [8, 9]
    );
}

#[test]
fn a_session_in_use_is_never_skipped_the_saved_one_is_created_beside_it() {
    let mut saved = snapshot();
    let mut other = saved.sessions[0].clone();
    other.id = "$2".into();
    other.name = "other".into();
    other.windows[0].index = 9;
    saved.sessions.push(other);
    // `workspace-restored` is taken by a live session and `other-restored` by a saved name.
    let mut third = saved.sessions[1].clone();
    third.id = "$3".into();
    third.name = "other-restored".into();
    third.windows[0].index = 11;
    saved.sessions.push(third);
    let live = [
        LiveSession::InUse("workspace".into()),
        LiveSession::InUse("workspace-restored".into()),
        LiveSession::InUse("other".into()),
    ];
    let plan = layout_creation(&saved, &live).unwrap();
    let names: Vec<_> = plan
        .sessions
        .iter()
        .map(|planned| match &planned.action {
            WorkspaceLayoutSessionAction::Create { name, .. } => name.as_str(),
            _ => panic!("every session in use is created beside, never skipped"),
        })
        .collect();
    assert_eq!(
        names,
        ["workspace-restored-2", "other-restored-2", "other-restored"]
    );
}

#[test]
fn only_this_snapshots_own_marker_makes_a_session_a_repeat() {
    let saved = snapshot();
    let marker = restored_marker(&saved, &saved.sessions[0]);
    let mut later = saved.clone();
    later.captured_at_ms += 1;
    assert_ne!(marker, restored_marker(&later, &later.sessions[0]));
    let mut renamed = saved.clone();
    renamed.sessions[0].id = "$9".into();
    assert_ne!(marker, restored_marker(&renamed, &renamed.sessions[0]));
}

#[test]
fn a_session_restored_beside_another_is_recognised_under_its_new_name() {
    let saved = snapshot();
    let made = LiveSession::Restored {
        name: "workspace-restored".into(),
        marker: restored_marker(&saved, &saved.sessions[0]),
    };
    let plan = layout_creation(&saved, &[LiveSession::InUse("workspace".into()), made]).unwrap();
    assert!(matches!(
        plan.sessions[0].action,
        WorkspaceLayoutSessionAction::SkipExisting
    ));
    // Another snapshot's session of the same name is only a name in use.
    let mut later = saved.clone();
    later.captured_at_ms += 1;
    let plan = layout_creation(
        &later,
        &[LiveSession::Restored {
            name: "workspace".into(),
            marker: restored_marker(&saved, &saved.sessions[0]),
        }],
    )
    .unwrap();
    let WorkspaceLayoutSessionAction::Create { name, .. } = &plan.sessions[0].action else {
        panic!("a session of another restore is not this one");
    };
    assert_eq!(name, "workspace-restored");
}

fn snapshot() -> WorkspaceSnapshot {
    use crate::endpoint::ProcessIncarnation;
    use crate::workspace::{WindowLink, WorkspaceServer, WorkspaceSession};
    let body = "80x24,0,0{39x24,0,0,7,40x24,40,0,8}";
    WorkspaceSnapshot {
        captured_at_ms: 1,
        server: WorkspaceServer {
            socket: "/private/socket".into(),
            process: ProcessIncarnation::new(10, "old-start").unwrap(),
            id: None,
        },
        sessions: vec![WorkspaceSession {
            id: "$1".into(),
            name: "workspace".into(),
            windows: vec![WindowLink {
                index: 3,
                window: "@1".into(),
                active: true,
            }],
        }],
        windows: vec![WorkspaceWindow {
            id: "@1".into(),
            name: "shells".into(),
            layout: with_checksum(body),
            visible_layout: with_checksum(body),
            width: 80,
            height: 24,
            active_pane: "%8".into(),
        }],
        // Record order is deliberately reversed: native pane assignment
        // uses layout traversal rather than snapshot vector ordering.
        panes: vec![8, 7]
            .into_iter()
            .map(|id| WorkspacePane {
                id: format!("%{id}"),
                window: "@1".into(),
                index: id - 7,
                left: if id == 7 { 0 } else { 40 },
                top: 0,
                width: if id == 7 { 39 } else { 40 },
                height: 24,
                cwd: format!("/work/{id}"),
                identity: None,
                command: None,
            })
            .collect(),
    }
}

#[test]
fn prepares_exact_leaf_cwd_mapping_and_linked_windows_once() {
    let mut saved = snapshot();
    let mut linked = saved.sessions[0].clone();
    linked.id = "$2".into();
    linked.name = "linked".into();
    linked.windows[0].index = 9;
    saved.sessions.push(linked);
    let windows = prepare_layout(&saved).unwrap();
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].root.leaf_ids(), [7, 8]);
    assert_eq!(
        windows[0]
            .panes
            .iter()
            .map(|pane| pane.cwd.as_str())
            .collect::<Vec<_>>(),
        ["/work/7", "/work/8"]
    );
    assert!(!windows[0].zoomed);
}

#[test]
fn zoom_correspondence_uses_full_leaf_ids_not_visible_pane_dimensions() {
    let mut saved = snapshot();
    saved.windows[0].visible_layout = with_checksum("80x24,0,0,8");
    saved.panes[0].left = 0;
    saved.panes[0].width = 80;
    assert!(prepare_layout(&saved).unwrap()[0].zoomed);
    saved.windows[0].visible_layout = with_checksum("80x24,0,0,7");
    assert!(matches!(
        prepare_layout(&saved),
        Err(WorkspaceLayoutError::Topology)
    ));
}

#[test]
fn refuses_unmatched_leaves_names_cwd_and_unlinked_windows_before_effects() {
    let original = snapshot();
    for change in 0..8 {
        let mut saved = original.clone();
        match change {
            0 => saved.panes[0].id = "%9".into(),
            1 => saved.panes[0].cwd = "relative".into(),
            2 => saved.panes[0].cwd = "/work\0other".into(),
            3 => saved.sessions[0].name = "ambiguous:name".into(),
            4 => saved.sessions[0].windows[0].index = i32::MAX as u64 + 1,
            5 => {
                let mut duplicate = saved.sessions[0].clone();
                duplicate.id = "$2".into();
                saved.sessions.push(duplicate);
            }
            6 => saved.windows[0].name = "shell\0other".into(),
            _ => {
                let mut orphan = saved.windows[0].clone();
                orphan.id = "@2".into();
                saved.windows.push(orphan);
            }
        }
        assert!(prepare_layout(&saved).is_err(), "mutation {change}");
    }
}

#[test]
fn validates_nested_geometry_and_preserves_old_leaf_order() {
    let root = parse_layout(&with_checksum(
        "80x24,0,0{39x24,0,0,7,40x24,40,0[40x11,40,0,8,40x12,40,12,9]}",
    ))
    .unwrap();
    assert_eq!((root.width, root.height), (80, 24));
    let WorkspaceLayoutKind::Split(WorkspaceSplitAxis::Horizontal, children) = root.kind else {
        panic!("horizontal root")
    };
    assert_eq!(children[0].kind, WorkspaceLayoutKind::Pane(7));
    let WorkspaceLayoutKind::Split(WorkspaceSplitAxis::Vertical, right) = &children[1].kind else {
        panic!("vertical child")
    };
    assert_eq!(right[0].kind, WorkspaceLayoutKind::Pane(8));
    assert_eq!(right[1].kind, WorkspaceLayoutKind::Pane(9));
}

#[test]
fn refuses_corrupt_checksum_unknown_syntax_and_incomplete_input() {
    assert_eq!(
        parse_layout("0000,80x24,0,0,7"),
        Err(WorkspaceLayoutError::Checksum)
    );
    for body in [
        "",
        "80x24,0,0",
        "80x24,0,0,7 trailing",
        "80x24,0,0(80x24,0,0,7)",
        "80x24,0,0[80x24,0,0,7}",
        "80x24,0,0,4294967296",
    ] {
        assert!(parse_layout(&with_checksum(body)).is_err(), "{body}");
    }
    // A multibyte invalid prefix must refuse before slicing UTF-8 text.
    assert_eq!(
        parse_layout("💥,80x24,0,0,7"),
        Err(WorkspaceLayoutError::Syntax)
    );
}

#[test]
fn refuses_duplicate_leaves_and_geometry_that_would_reassign_panes() {
    assert_eq!(
        parse_layout(&with_checksum("80x24,0,0{39x24,0,0,7,40x24,40,0,7}")),
        Err(WorkspaceLayoutError::DuplicatePane)
    );
    for body in [
        "0x24,0,0,7",
        "65536x24,0,0,7",
        "80x24,1,0,7",
        "80x24,0,0{80x24,0,0,7}",
        "80x24,0,0{39x24,0,0,7,40x24,39,0,8}",
        "80x24,0,0{39x24,0,0,7,39x24,40,0,8}",
        "80x24,0,0{39x23,0,0,7,40x24,40,0,8}",
    ] {
        assert_eq!(
            parse_layout(&with_checksum(body)),
            Err(WorkspaceLayoutError::Geometry),
            "{body}"
        );
    }
}

#[test]
fn bounds_recursion_before_any_effect_or_unbounded_allocation() {
    let mut body = "1x1,0,0,7".to_owned();
    for _ in 0..MAX_DEPTH {
        body = format!("1x1,0,0[{body}]");
    }
    assert_eq!(
        parse_layout(&with_checksum(&body)),
        Err(WorkspaceLayoutError::Limit)
    );
    assert_eq!(
        parse_layout(&"a".repeat(MAX_LAYOUT_BYTES + 1)),
        Err(WorkspaceLayoutError::Limit)
    );
}

#[test]
fn physical_geometry_restores_window_local_border_status_and_refuses_mismatch() {
    for status in ["off", "top", "bottom"] {
        let mut saved = snapshot();
        for pane in &mut saved.panes {
            if status == "top" {
                pane.top += 1;
            }
            if status != "off" {
                pane.height -= 1;
            }
        }
        assert_eq!(prepare_layout(&saved).unwrap()[0].border_status, status);
        let mut overflowing = saved.clone();
        overflowing.panes[0].height = u64::MAX;
        assert!(matches!(
            prepare_layout(&overflowing),
            Err(WorkspaceLayoutError::Geometry)
        ));

        saved.panes[0].left += 1;
        assert!(matches!(
            prepare_layout(&saved),
            Err(WorkspaceLayoutError::Geometry)
        ));
    }
}

// Exact layout captured by the private tmux 3.8 #4189 proof, before #4402.
// This is producer evidence, not a string generated by the codec under test.
const TMUX_38_LAYOUT: &str = r#"{"V":2,"L":{"t":"v","w":120,"h":40,"x":0,"y":0,"c":[{"t":"h","w":120,"h":19,"x":0,"y":0,"c":[{"t":"p","w":59,"h":19,"x":0,"y":0,"a":true,"i":0,"I":"%0"},{"t":"p","w":60,"h":19,"x":60,"y":0,"i":1,"I":"%3"}]},{"t":"h","w":120,"h":20,"x":0,"y":20,"c":[{"t":"p","w":59,"h":20,"x":0,"y":20,"i":2,"I":"%2"},{"t":"p","w":60,"h":20,"x":60,"y":20,"i":3,"I":"%1"}]}]}}"#;

#[test]
fn retained_tmux_38_layout_admits_the_original_four_panes() {
    let root = parse_layout(TMUX_38_LAYOUT).unwrap();
    assert_eq!(
        (root.width, root.height, root.left, root.top),
        (120, 40, 0, 0)
    );
    assert_eq!(root.leaf_ids(), [0, 3, 2, 1]);
    for (id, rectangle) in [
        (0, (59, 19, 0, 0)),
        (3, (60, 19, 60, 0)),
        (2, (59, 20, 0, 20)),
        (1, (60, 20, 60, 20)),
    ] {
        let cell = leaf_cell(&root, id).unwrap();
        assert_eq!((cell.width, cell.height, cell.left, cell.top), rectangle);
    }
    let mut splits = Vec::new();
    append_splits(&root, &mut splits).unwrap();
    assert_eq!(
        splits
            .iter()
            .map(|s| (s.target_leaf, s.new_leaf, s.remaining_extent))
            .collect::<Vec<_>>(),
        [(0, 2, 20), (0, 3, 60), (2, 1, 60)]
    );
}

#[test]
fn tmux_v2_refuses_untrusted_metadata_and_invalid_geometry() {
    let valid = r#"{"V":2,"L":{"t":"p","w":80,"h":24,"x":0,"y":0,"i":0,"I":"%7"}}"#;
    assert!(parse_layout(valid).is_ok());
    for (from, to) in [
        ("\"V\":2", "\"V\":3"),
        ("\"V\":2", "\"V\":2,\"V\":2"),
        ("\"w\":80", "\"w\":80,\"w\":80"),
        ("\"w\":80", "\"w\":80.0"),
        ("\"w\":80", "\"w\":8e1"),
        ("\"w\":80", "\"w\":080"),
        ("\"w\":80", "\"w\":0"),
        ("\"w\":80", "\"w\":65536"),
        ("\"x\":0", "\"x\":-1"),
        ("\"x\":0", "\"x\":1"),
        ("\"t\":\"p\"", "\"t\":\"z\""),
        ("\"i\":0", "\"z\":0,\"i\":0"),
        ("\"i\":0", "\"i\":2147483648"),
        ("\"i\":0", "\"a\":true,\"l\":0,\"i\":0"),
        ("\"i\":0", "\"a\":1,\"i\":0"),
        ("\"i\":0", "\"c\":[],\"i\":0"),
        ("\"i\":0,", ""),
        ("\"I\":\"%7\"", "\"I\":\"%07\""),
        ("\"I\":\"%7\"", "\"I\":\"%+7\""),
        ("\"I\":\"%7\"", "\"I\":\"%4294967296\""),
        ("\"I\":\"%7\"", "\"I\":\"%\\u0037\""),
        ("\"V\":2", "\"unknown\":2"),
        ("\"V\":2", "\"V\":2,"),
        ("\"V\":2", "\"V\":2,\"L\":null"),
    ] {
        let invalid = valid.replace(from, to);
        assert!(parse_layout(&invalid).is_err(), "admitted {invalid}");
    }
    assert!(parse_layout(&format!("{valid} trailing")).is_err());
    for invalid in [
        TMUX_38_LAYOUT.replace("\"I\":\"%3\"", "\"I\":\"%0\""),
        TMUX_38_LAYOUT.replace("\"i\":1", "\"i\":0"),
        TMUX_38_LAYOUT.replace("\"i\":1", "\"a\":true,\"i\":1"),
        TMUX_38_LAYOUT
            .replace("\"i\":1", "\"l\":0,\"i\":1")
            .replace("\"i\":2", "\"l\":0,\"i\":2"),
        TMUX_38_LAYOUT.replace("\"x\":60", "\"x\":59"),
        TMUX_38_LAYOUT.replacen("\"t\":\"v\"", "\"t\":\"h\"", 1),
        TMUX_38_LAYOUT.replacen("\"t\":\"v\"", "\"t\":\"v\",\"i\":8", 1),
    ] {
        assert!(parse_layout(&invalid).is_err(), "admitted {invalid}");
    }
    assert_eq!(
        parse_layout(&" ".repeat(MAX_LAYOUT_BYTES + 1)),
        Err(WorkspaceLayoutError::Limit)
    );
    let mut nested = r#"{"t":"p","w":80,"h":24,"x":0,"y":0,"i":0,"I":"%7"}"#.to_owned();
    for _ in 0..MAX_DEPTH {
        nested = format!(r#"{{"t":"h","w":80,"h":24,"x":0,"y":0,"c":[{nested}]}}"#);
    }
    assert_eq!(
        parse_layout(&format!(r#"{{"V":2,"L":{nested}}}"#)),
        Err(WorkspaceLayoutError::Limit)
    );
}

#[test]
fn live_server_encoding_adapts_geometry_and_preserves_same_encoding_bytes() {
    let json = " { \"L\": { \"I\":\"%7\", \"i\":0, \"l\":0, \"y\":0,\"x\":0,\"h\":24,\"w\":80,\"t\":\"p\" }, \"V\":2 } ";
    let legacy = with_checksum("80x24,0,0,7");
    let mut saved = snapshot();
    saved.windows[0].layout = json.into();
    let root = parse_layout(json).unwrap();
    let prepared = WorkspaceLayoutWindow {
        window: &saved.windows[0],
        root: root.clone(),
        panes: vec![],
        floating: vec![],
        zoomed: false,
        border_status: "off",
        splits: vec![],
    };
    assert_eq!(prepared.layout_for_server(json).unwrap(), json);
    assert_eq!(prepared.layout_for_server(&legacy).unwrap(), legacy);
    assert!(prepared.layout_for_server("unrecognized").is_err());
    saved.windows[0].layout = legacy.clone();
    let prepared = WorkspaceLayoutWindow {
        window: &saved.windows[0],
        root: root.clone(),
        panes: vec![],
        floating: vec![],
        zoomed: false,
        border_status: "off",
        splits: vec![],
    };
    assert_eq!(prepared.layout_for_server(&legacy).unwrap(), legacy);
    assert_eq!(
        parse_layout(&prepared.layout_for_server(json).unwrap()).unwrap(),
        root
    );
    let root = parse_layout(TMUX_38_LAYOUT).unwrap();
    let mut legacy = String::new();
    legacy_body(&root, &mut legacy);
    assert_eq!(
        legacy,
        "120x40,0,0[120x19,0,0{59x19,0,0,0,60x19,60,0,3},120x20,0,20{59x20,0,20,2,60x20,60,20,1}]"
    );
    assert_eq!(parse_layout(&with_checksum(&legacy)).unwrap(), root);
    assert_eq!(parse_layout(&json::serialize(&root)).unwrap(), root);
}

#[test]
fn json_snapshot_keeps_legacy_geometry_and_correspondence_admission() {
    let mut saved = snapshot();
    let legacy = prepare_layout(&saved).unwrap();
    let expected_panes: Vec<_> = legacy[0].panes.iter().map(|p| p.id.clone()).collect();
    let expected_splits = legacy[0].splits.clone();
    let expected_root = legacy[0].root.clone();
    saved.windows[0].layout = json::serialize(&expected_root);
    saved.windows[0].visible_layout = saved.windows[0].layout.clone();
    let prepared = prepare_layout(&saved).unwrap();
    assert_eq!(prepared[0].root, expected_root);
    assert_eq!(prepared[0].splits, expected_splits);
    assert_eq!(
        prepared[0]
            .panes
            .iter()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>(),
        expected_panes
    );
    saved.panes[0].id = "%99".into();
    assert!(prepare_layout(&saved).is_err());
}

#[test]
fn reversed_v2_indexes_keep_historical_panes_with_their_geometry() {
    // Exact producer layout from an isolated tmux 3.8 select-layout/capture.
    // tmux ignores I and assigns native panes by sorted i, unlike legacy DFS.
    let producer = r#"{"V":2,"L":{"t":"h","w":80,"h":24,"x":0,"y":0,"c":[{"t":"p","w":29,"h":24,"x":0,"y":0,"i":1,"I":"%1"},{"t":"p","w":50,"h":24,"x":30,"y":0,"a":true,"i":0,"I":"%0"}]}}"#;
    let mut saved = snapshot();
    saved.windows[0].layout = producer.into();
    let root = parse_layout(producer).unwrap();
    let prepared = WorkspaceLayoutWindow {
        window: &saved.windows[0],
        root: root.clone(),
        panes: vec![],
        floating: vec![],
        zoomed: false,
        border_status: "off",
        splits: vec![],
    };
    let applied = prepared.layout_for_server(producer).unwrap();
    let expected = r#"{"V":2,"L":{"t":"h","w":80,"h":24,"x":0,"y":0,"c":[{"t":"p","w":29,"h":24,"x":0,"y":0,"i":0,"I":"%1"},{"t":"p","w":50,"h":24,"x":30,"y":0,"i":1,"I":"%0"}]}}"#;
    assert_eq!(applied, expected);
    assert_eq!(parse_layout(&applied).unwrap(), root);
    assert_eq!(saved.windows[0].layout, producer);
}

// Exact `window_layout` / `window_visible_layout` strings from an isolated tmux 3.8 server (its
// own -S socket, never the host's), captured for #4405 by the two scripts and outputs kept in
// /private/tmp/core-ops-4405-evidence/. They are producer evidence, not output of the codec under
// test. The panes were made with `new-pane`; the plain split is the same window without floats.
const TMUX_38_SINGLE: &str =
    r#"{"V":2,"L":{"t":"p","w":120,"h":40,"x":0,"y":0,"a":true,"i":0,"I":"%0"}}"#;
const TMUX_38_SPLIT: &str = r#"{"V":2,"L":{"t":"h","w":120,"h":40,"x":0,"y":0,"c":[{"t":"p","w":60,"h":40,"x":0,"y":0,"a":true,"i":0,"I":"%0"},{"t":"p","w":59,"h":40,"x":61,"y":0,"i":1,"I":"%1"}]}}"#;
const TMUX_38_ONE_FLOAT: &str = r#"{"V":2,"L":{"t":"h","w":120,"h":40,"x":0,"y":0,"c":[{"t":"p","w":60,"h":40,"x":0,"y":0,"a":true,"i":0,"I":"%0"},{"t":"p","w":60,"h":10,"x":4,"y":2,"i":2,"z":0,"I":"%2"},{"t":"p","w":59,"h":40,"x":61,"y":0,"i":1,"I":"%1"}]}}"#;
const TMUX_38_TWO_FLOATS: &str = r#"{"V":2,"L":{"t":"h","w":120,"h":40,"x":0,"y":0,"c":[{"t":"p","w":60,"h":40,"x":0,"y":0,"a":true,"i":0,"I":"%0"},{"t":"p","w":28,"h":8,"x":6,"y":4,"i":3,"z":0,"I":"%3"},{"t":"p","w":60,"h":10,"x":4,"y":2,"i":2,"z":1,"I":"%2"},{"t":"p","w":59,"h":40,"x":61,"y":0,"i":1,"I":"%1"}]}}"#;
const TMUX_38_THREE_FLOATS: &str = r#"{"V":2,"L":{"t":"h","w":120,"h":40,"x":0,"y":0,"c":[{"t":"p","w":60,"h":40,"x":0,"y":0,"a":true,"i":0,"I":"%0"},{"t":"p","w":18,"h":6,"x":61,"y":21,"i":4,"z":0,"I":"%4"},{"t":"p","w":28,"h":8,"x":6,"y":4,"i":3,"z":1,"I":"%3"},{"t":"p","w":60,"h":10,"x":4,"y":2,"i":2,"z":2,"I":"%2"},{"t":"p","w":59,"h":40,"x":61,"y":0,"i":1,"I":"%1"}]}}"#;
const TMUX_38_ACTIVE_FLOAT: &str = r#"{"V":2,"L":{"t":"h","w":120,"h":40,"x":0,"y":0,"c":[{"t":"p","w":60,"h":40,"x":0,"y":0,"l":0,"i":0,"I":"%0"},{"t":"p","w":23,"h":4,"x":8,"y":4,"a":true,"i":5,"z":0,"I":"%5"},{"t":"p","w":18,"h":6,"x":61,"y":21,"i":4,"z":1,"I":"%4"},{"t":"p","w":28,"h":8,"x":6,"y":4,"i":3,"z":2,"I":"%3"},{"t":"p","w":60,"h":10,"x":4,"y":2,"i":2,"z":3,"I":"%2"},{"t":"p","w":59,"h":40,"x":61,"y":0,"i":1,"I":"%1"}]}}"#;
const TMUX_38_SINGLE_PLUS_FLOAT: &str = r#"{"V":2,"L":{"t":"v","w":120,"h":40,"x":0,"y":0,"c":[{"t":"p","w":120,"h":40,"x":0,"y":0,"a":true,"i":0,"I":"%0"},{"t":"p","w":28,"h":8,"x":6,"y":4,"i":1,"z":0,"I":"%1"}]}}"#;
const TMUX_38_NESTED_PLUS_FLOAT: &str = r#"{"V":2,"L":{"t":"h","w":120,"h":40,"x":0,"y":0,"c":[{"t":"p","w":60,"h":40,"x":0,"y":0,"a":true,"i":0,"I":"%2"},{"t":"p","w":28,"h":8,"x":6,"y":4,"i":3,"z":0,"I":"%5"},{"t":"v","w":59,"h":40,"x":61,"y":0,"c":[{"t":"p","w":59,"h":20,"x":61,"y":0,"i":1,"I":"%3"},{"t":"p","w":59,"h":19,"x":61,"y":21,"i":2,"I":"%4"}]}]}}"#;
const TMUX_38_ZOOMED: &str = r#"{"V":2,"L":{"t":"h","w":120,"h":40,"x":0,"y":0,"c":[{"t":"p","w":60,"h":40,"x":0,"y":0,"a":true,"i":0,"I":"%6"},{"t":"p","w":18,"h":6,"x":11,"y":11,"i":3,"z":0,"I":"%9"},{"t":"p","w":18,"h":6,"x":61,"y":21,"i":2,"z":1,"I":"%8"},{"t":"p","w":59,"h":40,"x":61,"y":0,"i":1,"I":"%7"}]}}"#;
const TMUX_38_ZOOMED_VISIBLE: &str = r#"{"V":2,"L":{"t":"v","w":120,"h":40,"x":0,"y":0,"c":[{"t":"p","w":120,"h":40,"x":0,"y":0,"a":true,"i":0,"I":"%6"},{"t":"p","w":18,"h":6,"x":61,"y":21,"i":2,"z":0,"I":"%8"}]}}"#;

#[test]
fn floating_panes_project_to_the_tiled_tree_they_floated_over() {
    let tiled = parse_layout(TMUX_38_SPLIT).unwrap();
    for (layout, floating) in [
        (TMUX_38_ONE_FLOAT, vec![2]),
        (TMUX_38_TWO_FLOATS, vec![3, 2]),
        (TMUX_38_THREE_FLOATS, vec![4, 3, 2]),
        // A floating pane may be the active one; the tiled pane it replaced is then `last`.
        (TMUX_38_ACTIVE_FLOAT, vec![5, 4, 3, 2]),
    ] {
        let saved = parse_saved_layout(layout).unwrap();
        assert_eq!(saved.tiled, tiled, "{layout}");
        assert_eq!(saved.floating, floating, "{layout}");
        // A live layout is tiled only.
        assert_eq!(parse_layout(layout), Err(WorkspaceLayoutError::Syntax));
    }
    let pure = parse_saved_layout(TMUX_38_SPLIT).unwrap();
    assert_eq!((pure.tiled, pure.floating), (tiled, vec![]));
}

#[test]
fn a_root_left_with_one_tiled_pane_collapses_to_it() {
    let saved = parse_saved_layout(TMUX_38_SINGLE_PLUS_FLOAT).unwrap();
    assert_eq!(saved.tiled, parse_layout(TMUX_38_SINGLE).unwrap());
    assert_eq!(saved.floating, [1]);
}

#[test]
fn floats_sit_in_the_root_even_beside_a_nested_split() {
    let saved = parse_saved_layout(TMUX_38_NESTED_PLUS_FLOAT).unwrap();
    assert_eq!(saved.floating, [5]);
    assert_eq!(saved.tiled.leaf_ids(), [2, 3, 4]);
    let WorkspaceLayoutKind::Split(WorkspaceSplitAxis::Horizontal, children) = &saved.tiled.kind
    else {
        panic!("the root stays the tiled horizontal split")
    };
    assert_eq!(children.len(), 2);
    assert!(matches!(
        children[1].kind,
        WorkspaceLayoutKind::Split(WorkspaceSplitAxis::Vertical, _)
    ));
}

#[test]
fn a_zoom_keeps_every_float_in_the_layout_and_only_the_above_ones_in_the_visible_layout() {
    let full = parse_saved_layout(TMUX_38_ZOOMED).unwrap();
    assert_eq!(
        (full.tiled.leaf_ids(), full.floating),
        (vec![6, 7], vec![9, 8])
    );
    let visible = parse_saved_layout(TMUX_38_ZOOMED_VISIBLE).unwrap();
    assert_eq!(
        visible.tiled,
        parse_layout(TMUX_38_ZOOMED_VISIBLE_PANE).unwrap()
    );
    assert_eq!(visible.floating, [8]);
}

// The zoomed pane of the capture above on its own: what is left of its visible layout.
const TMUX_38_ZOOMED_VISIBLE_PANE: &str =
    r#"{"V":2,"L":{"t":"p","w":120,"h":40,"x":0,"y":0,"a":true,"i":0,"I":"%6"}}"#;

#[test]
fn floating_cells_outside_the_producer_grammar_are_refused() {
    for invalid in [
        // Two floats cannot share a stacking order.
        TMUX_38_TWO_FLOATS.replace("\"z\":1", "\"z\":0"),
        TMUX_38_ONE_FLOAT.replace("\"z\":0", "\"z\":65536"),
        TMUX_38_ONE_FLOAT.replace("\"z\":0", "\"z\":-1"),
        // Only a pane cell directly under the root floats.
        TMUX_38_ONE_FLOAT.replacen("\"t\":\"h\"", "\"t\":\"h\",\"z\":0", 1),
        TMUX_38_NESTED_PLUS_FLOAT.replace("\"i\":1,\"I\":\"%3\"", "\"i\":1,\"z\":1,\"I\":\"%3\""),
        TMUX_38_SINGLE.replace("\"i\":0", "\"z\":0,\"i\":0"),
        // A window of floats only, a lone tiled pane that does not fill the root, and a
        // tiled remainder that is not contiguous.
        TMUX_38_TWO_FLOATS
            .replace("\"a\":true,\"i\":0", "\"z\":7,\"i\":0")
            .replace("\"i\":1,\"I\":\"%1\"", "\"z\":8,\"i\":1,\"I\":\"%1\""),
        TMUX_38_SINGLE_PLUS_FLOAT.replace(
            "\"w\":120,\"h\":40,\"x\":0,\"y\":0,\"a\"",
            "\"w\":119,\"h\":40,\"x\":0,\"y\":0,\"a\"",
        ),
        TMUX_38_ONE_FLOAT.replace("\"x\":61", "\"x\":60"),
    ] {
        assert!(parse_saved_layout(&invalid).is_err(), "admitted {invalid}");
    }
}

fn floating_snapshot(
    layout: &str,
    visible: &str,
    active: &str,
    panes: &[u32],
) -> WorkspaceSnapshot {
    let mut saved = snapshot();
    saved.windows[0].layout = layout.into();
    saved.windows[0].visible_layout = visible.into();
    saved.windows[0].width = 120;
    saved.windows[0].height = 40;
    saved.windows[0].active_pane = active.into();
    let template = saved.panes[0].clone();
    saved.panes = panes
        .iter()
        .map(|id| WorkspacePane {
            id: format!("%{id}"),
            index: u64::from(*id),
            cwd: format!("/work/{id}"),
            ..template.clone()
        })
        .collect();
    saved
}

fn place(saved: &mut WorkspaceSnapshot, id: &str, rectangle: (u64, u64, u64, u64)) {
    let pane = saved.panes.iter_mut().find(|pane| pane.id == id).unwrap();
    (pane.left, pane.top, pane.width, pane.height) = rectangle;
}

#[test]
fn restore_plans_the_tiled_layout_and_names_each_float_it_leaves_out() {
    let mut saved = floating_snapshot(TMUX_38_TWO_FLOATS, TMUX_38_TWO_FLOATS, "%0", &[0, 1, 2, 3]);
    for (id, rectangle) in [
        ("%0", (0, 0, 60, 40)),
        ("%1", (61, 0, 59, 40)),
        ("%2", (4, 2, 60, 10)),
        ("%3", (6, 4, 28, 8)),
    ] {
        place(&mut saved, id, rectangle);
    }
    let windows = prepare_layout(&saved).unwrap();
    assert_eq!(windows[0].root.leaf_ids(), [0, 1]);
    let ids =
        |panes: &[&WorkspacePane]| panes.iter().map(|pane| pane.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&windows[0].panes), ["%0", "%1"]);
    assert_eq!(ids(&windows[0].floating), ["%3", "%2"]);
    assert_eq!(
        windows[0].splits.len(),
        1,
        "only the tiled panes are carved out"
    );
    assert!(!windows[0].zoomed);
    assert_eq!(
        floating_pane_ids(&saved),
        HashSet::from(["%2".to_owned(), "%3".to_owned()])
    );
    // The plan creates the tiled panes only, so no float is ever a creation target.
    let plan = layout_creation(&saved, &[]).unwrap();
    assert_eq!(plan.windows[0].panes.len(), 2);

    // Every float needs its saved pane, and every saved pane of the window a cell.
    let mut missing = saved.clone();
    missing.panes.retain(|pane| pane.id != "%3");
    assert!(matches!(
        prepare_layout(&missing),
        Err(WorkspaceLayoutError::Topology)
    ));
    let mut extra = saved.clone();
    extra.panes.push(WorkspacePane {
        id: "%9".into(),
        ..saved.panes[0].clone()
    });
    assert!(matches!(
        prepare_layout(&extra),
        Err(WorkspaceLayoutError::Topology)
    ));
}

#[test]
fn a_zoomed_window_with_floats_restores_its_tiled_panes() {
    let mut saved = floating_snapshot(TMUX_38_ZOOMED, TMUX_38_ZOOMED_VISIBLE, "%6", &[6, 7, 8, 9]);
    for (id, rectangle) in [
        ("%6", (0, 0, 120, 40)),
        ("%7", (61, 0, 59, 40)),
        ("%8", (61, 21, 18, 6)),
        ("%9", (11, 11, 18, 6)),
    ] {
        place(&mut saved, id, rectangle);
    }
    let windows = prepare_layout(&saved).unwrap();
    assert!(windows[0].zoomed);
    assert_eq!(windows[0].root.leaf_ids(), [6, 7]);
    assert_eq!(
        windows[0]
            .floating
            .iter()
            .map(|pane| pane.id.as_str())
            .collect::<Vec<_>>(),
        ["%9", "%8"]
    );
    // The visible layout may keep only floats the layout has.
    let mut foreign = saved.clone();
    foreign.windows[0].visible_layout = TMUX_38_ZOOMED_VISIBLE.replace("%8", "%7");
    assert!(prepare_layout(&foreign).is_err());
}

#[test]
fn a_window_with_floats_is_applied_as_its_tiled_tree_never_as_the_saved_bytes() {
    let saved = floating_snapshot(TMUX_38_ONE_FLOAT, TMUX_38_ONE_FLOAT, "%0", &[0, 1, 2]);
    let root = parse_saved_layout(TMUX_38_ONE_FLOAT).unwrap().tiled;
    let prepared = WorkspaceLayoutWindow {
        window: &saved.windows[0],
        root: root.clone(),
        panes: vec![],
        floating: vec![&saved.panes[2]],
        zoomed: false,
        border_status: "off",
        splits: vec![],
    };
    let applied = prepared.layout_for_server(TMUX_38_SPLIT).unwrap();
    assert!(!applied.contains("\"z\""), "{applied}");
    assert_eq!(parse_layout(&applied).unwrap(), root);
    // The same window without floats keeps passing its original bytes through.
    let plain_saved = floating_snapshot(TMUX_38_SPLIT, TMUX_38_SPLIT, "%0", &[0, 1]);
    let plain = WorkspaceLayoutWindow {
        window: &plain_saved.windows[0],
        root,
        panes: vec![],
        floating: vec![],
        zoomed: false,
        border_status: "off",
        splits: vec![],
    };
    assert_eq!(
        plain.layout_for_server(TMUX_38_SPLIT).unwrap(),
        TMUX_38_SPLIT
    );
}
