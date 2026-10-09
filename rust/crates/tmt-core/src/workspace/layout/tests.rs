use super::*;

#[test]
fn skipped_session_confers_no_reuse_authority_for_its_shared_window() {
    let mut saved = snapshot();
    let mut missing = saved.sessions[0].clone();
    missing.id = "$2".into();
    missing.name = "missing".into();
    missing.windows[0].index = 9;
    saved.sessions.push(missing);
    let plan = layout_creation(&saved, &["workspace".into()]).unwrap();
    assert!(matches!(
        plan.sessions[0].action,
        WorkspaceLayoutSessionAction::SkipExisting
    ));
    let WorkspaceLayoutSessionAction::Create {
        seed_window,
        bootstrap_index,
        links,
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
