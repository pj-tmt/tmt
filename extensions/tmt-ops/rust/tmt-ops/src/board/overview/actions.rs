//! Read-only action previews share the same admitted values as their effects.
use crate::{
    action::{Action, Verb},
    effects,
};
use serde_json::Value;

pub(in crate::board) fn host(row: &Value, invoker_tmux: bool) -> Result<&str, &'static str> {
    if !invoker_tmux {
        return Err("standalone · opens the conversation");
    }
    match row["pane"]["location"]["host"].as_str() {
        Some("herdr") => Err("Herdr focus is unavailable"),
        Some("tmux") if row["presence"] == "active" => {
            row["id"].as_str().ok_or("target UUID unavailable")
        }
        Some("tmux") => Err("target is offline or stale"),
        _ => Err("binding host unavailable"),
    }
}

pub(in crate::board) fn host_preview(
    row: &Value,
    invoker_tmux: bool,
    action: Option<(&str, &Action)>,
    ops: Option<&Value>,
) -> String {
    let host_name = row["pane"]["location"]["host"]
        .as_str()
        .unwrap_or("unknown");
    let id = row["id"].as_str().unwrap_or("unavailable");
    let Some((key, action)) = action else {
        return format!("Host: {host_name} · target {id} · no host action bound");
    };
    let command = match action.verb {
        Verb::Jump if !invoker_tmux => "opens conversation · Esc returns".into(),
        Verb::Jump => host(row, true)
            .map(|id| format!("tmt focus {id}"))
            .unwrap_or_else(|reason| format!("Unavailable: {reason}")),
        Verb::Swap => row["id"]
            .as_str()
            .map(|id| {
                format!(
                    "tmt focus {id} --into {} --json",
                    action.args[0].literal().expect("validated direction")
                )
            })
            .unwrap_or_else(|| "Unavailable: target UUID unavailable".into()),
        Verb::Run if action.needs_board_pane() => {
            format!("{} · board_pane resolves when pressed", action.text)
        }
        Verb::Run => action
            .argv_parameters(crate::template::Parameters {
                row,
                ops,
                board_pane: None,
            })
            .map(|argv| format!("argv {argv:?}"))
            .unwrap_or_else(|reason| format!("Unavailable: {reason}")),
        _ => action.description(),
    };
    format!("Host: {host_name} · target {id} · press {key}: {command}")
}

pub(in crate::board) fn link_preview(
    action: &Action,
    row: &Value,
    opener: Option<&[String]>,
) -> Result<String, String> {
    match action.verb {
        Verb::Open => {
            let link = effects::resolve_open(action, row)?;
            let argv = effects::open_argv(&link, opener);
            Ok(format!("{link} · argv {argv:?}"))
        }
        Verb::Run => Ok(format!("argv {:?}", action.argv(row)?)),
        _ => Ok(action.description()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn host_never_inferrs_capability_from_a_name_or_pane() {
        let mut row = json!({"id":"uuid","name":"tmux","pane":{"id":"%9"},"presence":"active"});
        assert!(host(&row, true).is_err());
        row["pane"]["location"] = json!({"host":"herdr"});
        assert!(host(&row, true).is_err());
        row["pane"]["location"] = json!({"host":"tmux"});
        assert_eq!(host(&row, true), Ok("uuid"));
        assert!(host(&row, false).is_err());
        row["presence"] = json!("offline");
        assert!(host(&row, true).is_err());
    }
    #[test]
    fn link_preview_preserves_the_actual_argument_without_shell_authority() {
        let row = json!({"fields":{"pr_link":"https://first.test","review_link":"https://例.test/a?x=$(id)"}});
        let open = Action::parse("open {review_link}").unwrap();
        let text =
            link_preview(&open, &row, Some(&["recorder".into(), "--literal".into()])).unwrap();
        assert!(text.contains("https://例.test/a?x=$(id)"));
        assert!(text.contains("recorder"));
        assert!(!text.contains("https://first.test"));
        let run = Action::parse("run recorder {review_link}").unwrap();
        assert_eq!(
            link_preview(&run, &row, None).unwrap(),
            format!("argv {:?}", run.argv(&row).unwrap())
        );
    }
    #[test]
    fn host_preview_uses_the_effective_custom_argv_without_inventing_a_focus_command() {
        let row = json!({"id":"uuid", "pane":{"location":{"host":"tmux"}}, "presence":"active", "fields":{"task":"literal $(id)"}});
        let action = Action::parse("run recorder {task}").unwrap();
        let preview = host_preview(&row, true, Some(("j", &action)), None);
        assert!(preview.contains(&format!("argv {:?}", action.argv(&row).unwrap())));
        assert!(!preview.contains("tmt focus"));
        let jump = Action::parse("jump").unwrap();
        assert!(
            host_preview(&row, true, Some(("f6", &jump)), None)
                .contains("press f6: tmt focus uuid")
        );
        assert!(
            host_preview(&row, false, Some(("f6", &jump)), None)
                .contains("opens conversation · Esc returns")
        );
    }
    #[test]
    fn host_preview_leaves_dispatch_context_unresolved_and_swap_capability_unclaimed() {
        let row = json!({"id":"uuid", "pane":{"location":{"host":"herdr"}}, "fields":{"board_pane":"%99"}});
        let run = Action::parse("run recorder {board_pane} {pane}").unwrap();
        let preview = host_preview(&row, false, Some(("j", &run)), None);
        assert!(preview.contains("run recorder {board_pane} {pane}"));
        assert!(preview.contains("board_pane resolves when pressed"));
        assert!(!preview.contains("argv") && !preview.contains("%99"));
        assert!(!preview.contains("Unavailable"));
        let swap = Action::parse("swap left").unwrap();
        let preview = host_preview(&row, false, Some(("j", &swap)), None);
        assert!(preview.contains("press j: tmt focus uuid --into left --json"));
        assert!(!preview.contains("Swapped") && !preview.contains("Started"));
        assert!(
            host_preview(&json!({}), false, Some(("j", &swap)), None)
                .contains("target UUID unavailable")
        );
    }
    #[test]
    fn standalone_jump_and_escape_restore_the_shared_reader_without_a_host_effect() {
        use crate::board::{
            app::{App, Effect},
            shell::OpsTab,
        };
        let mut app = crate::board::app::workspace::tests::board();
        crate::board::app::workspace::tests::observe_waiting_member(&mut app);
        app.shell.tab = OpsTab::Agents;
        app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
        app.invoker_tmux = false;
        app.set_body_width(80);
        let selected = app.workspace.selected.clone();
        assert_eq!(app.perform(&Action::parse("jump").unwrap()), Effect::None);
        assert_eq!(app.shell.tab, OpsTab::Agents);
        assert_eq!(app.workspace.selected, selected);
        assert_eq!(app.focus, 1);
        let key = ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Esc,
            ratatui::crossterm::event::KeyModifiers::NONE,
        );
        assert_eq!(app.key(key), Effect::None);
        assert_eq!(app.shell.tab, OpsTab::Agents);
        assert_eq!(app.workspace.selected, selected);
        let _ = &app as &App;
    }
}
