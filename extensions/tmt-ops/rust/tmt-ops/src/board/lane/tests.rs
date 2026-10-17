use super::*;
use crate::{action::Action, config::OpsKeyScope};
use serde_json::json;

struct Fixture {
    root: std::path::PathBuf,
    core: Core,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("ops-host-opening-{}", crate::id::new_v4().unwrap()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("caller"), r#"{"bound":false,"pane":"%9"}"#).unwrap();
        let executable = root.join("core");
        crate::test_support::write_ready_executable(
            &executable,
            r#"#!/bin/sh
root=${0%/*}
printf '%s\n' "$*" >> "$root/calls"
case "$1:$2" in
  config:show) printf '{"paths":{"global":"%s/config.json"},"resolved":{}}\n' "$root" ;;
  api:*) cat > /dev/null; printf '{"dataRoot":"%s/data"}\n' "$root" ;;
  whoami:*) cat "$root/caller" ;;
  *) printf '%s\n' '{"error":{"code":"UNEXPECTED","message":"Unexpected fake-Core call."}}'; exit 1 ;;
esac
"#,
        );
        Self {
            root,
            core: Core::at(executable),
        }
    }
    fn whoami_count(&self) -> usize {
        std::fs::read_to_string(self.root.join("calls"))
            .unwrap()
            .lines()
            .filter(|call| call.starts_with("whoami "))
            .count()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

fn selection() -> super::super::settings::host_action::Selection {
    super::super::settings::host_action::Selection {
        row: Some(json!({"id":"identity", "name":"alpha", "fields":{"board_pane":"%99"}})),
        recipient: Some(json!({"id":"identity", "name":"alpha", "fields":{"board_pane":"%99"}})),
        default_action: Some(Action::parse("jump").unwrap()),
        squad: None,
        scope: OpsKeyScope::Member,
        invoker_tmux: true,
    }
}

#[test]
fn only_host_action_opening_reads_public_caller_context_and_keeps_null_unavailable() {
    let f = Fixture::new();
    for opening in [Opening::Settings, Opening::ThemePicker(None)] {
        match run(&f.core, None, Job::Open(opening)) {
            BoardEvent::Opened {
                config, board_pane, ..
            } => {
                assert!(config.is_ok());
                assert!(board_pane.is_none());
            }
            _ => panic!("expected config opening"),
        }
    }
    assert_eq!(f.whoami_count(), 0);
    match run(&f.core, None, Job::Open(Opening::HostAction(selection()))) {
        BoardEvent::Opened {
            config,
            board_pane,
            opening: Opening::HostAction(held),
        } => {
            assert!(config.is_ok());
            assert_eq!(board_pane.as_deref(), Some("%9"));
            assert_eq!(held, selection());
        }
        _ => panic!("expected host action opening"),
    }
    assert_eq!(f.whoami_count(), 1);
    std::fs::write(f.root.join("caller"), r#"{"bound":false}"#).unwrap();
    match run(&f.core, None, Job::Open(Opening::HostAction(selection()))) {
        BoardEvent::Opened {
            config, board_pane, ..
        } => {
            assert!(config.is_ok());
            assert!(board_pane.is_none());
        }
        _ => panic!("expected host action opening"),
    }
    assert_eq!(f.whoami_count(), 2);
}

#[test]
fn prepared_run_refuses_missing_context_without_starting_the_program() {
    let f = Fixture::new();
    std::fs::write(f.root.join("caller"), r#"{"bound":false}"#).unwrap();
    let marker = f.root.join("unexpected");
    let action = Action::parse(&format!("run touch {} {{board_pane}}", marker.display())).unwrap();
    let request = Request::PreparedRun {
        action,
        row: json!({"id":"identity", "fields":{"board_pane":"%99"}}),
    };
    match run(&f.core, None, Job::Act(request)) {
        BoardEvent::Acted {
            outcome: Err(error),
            jump: false,
            sends: false,
        } => assert!(error.contains("board_pane is empty")),
        _ => panic!("unavailable context must refuse"),
    }
    assert_eq!(f.whoami_count(), 1);
    assert!(!marker.exists());
}
