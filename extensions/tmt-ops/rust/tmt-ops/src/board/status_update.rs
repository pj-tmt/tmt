//! Status form state inside the shared row composer and band.
use super::app::{App, Effect, Request};
use crate::membership::status_update::{self, Outcome, Preview, Submit, Target};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Action {
    Apply(Submit),
    Retry {
        preview: Preview,
        intent: crate::send::Intent,
    },
}
#[derive(Default)]
pub(super) struct Draft {
    pub target: Option<Target>,
    pub preview: Option<Preview>,
    pub clear_pending: bool,
    pub replace_state: bool,
    pub state: tmt_tui::components::Editor,
    pub reason: tmt_tui::components::Editor,
    pub focus: usize,
    pub scroll: Option<usize>,
    pub error: Option<String>,
    pub outcome: Option<Outcome>,
}
impl Draft {
    pub(super) fn editable_reason(&self) -> bool {
        self.focus == 2
            && !matches!(
                self.outcome,
                Some(Outcome::Applied { .. } | Outcome::Unknown(_))
            )
    }

    pub fn loaded(&mut self, result: Result<Preview, String>) {
        match result {
            Ok(preview) => {
                self.preview = Some(preview);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub fn finished(&mut self, outcome: Outcome) {
        match &outcome {
            Outcome::Conflict(result) => {
                self.preview = None;
                self.clear_pending = false;
                self.replace_state = false;
                self.focus = 0;
                self.scroll = None;
                self.loaded(result.clone());
                self.error = Some(match &self.error {
                    Some(error) => format!(
                        "Conflict; preview refresh failed: {error}. Reopen before applying."
                    ),
                    None => "Selected values changed; preview refreshed. Review and submit again."
                        .into(),
                });
            }
            Outcome::Refused(error) | Outcome::Unknown(error) => self.error = Some(error.clone()),
            Outcome::Applied { notification, .. } => {
                self.focus = 0;
                self.scroll = None;
                self.error = Some(match notification {
                    Ok(_) => {
                        "Status updated; notification accepted (queued, no reply required).".into()
                    }
                    Err(error) => format!("Status updated; notification failed. {error}"),
                })
            }
        }
        self.outcome = Some(outcome);
    }
}
impl App {
    pub(super) fn start_status(&mut self) {
        let Some(input) = &self.input else { return };
        let Some(send) = &input.row_send else { return };
        let Some(actor) = self.view.as_ref().and_then(|view| view.me_id.clone()) else {
            return;
        };
        let Some(identity) = self
            .target_row(&send.target)
            .and_then(|row| row["id"].as_str())
            .map(str::to_owned)
        else {
            return;
        };
        let target = Target {
            squad: input.squad.clone(),
            identity,
            actor,
        };
        if self
            .status_draft
            .as_ref()
            .is_none_or(|draft| draft.target.as_ref() != Some(&target))
        {
            self.status_draft = Some(Draft {
                target: Some(target),
                ..Draft::default()
            });
        }
    }
    pub(super) fn status_key(&mut self, key: KeyEvent) -> Effect {
        if key.code == KeyCode::Esc {
            self.input = None;
            self.status_draft = None;
            return self.say("Cancelled; no further action.");
        }
        if key.code == KeyCode::Tab {
            return Effect::None;
        }
        if self.input.as_ref().is_some_and(|input| {
            input
                .row_send
                .as_ref()
                .is_none_or(|send| !send.valid(self, input))
        }) {
            let outcome = match self
                .status_draft
                .as_ref()
                .and_then(|draft| draft.outcome.as_ref())
            {
                Some(Outcome::Applied { .. }) => {
                    "Status was already applied; notification intent retained."
                }
                Some(Outcome::Unknown(_)) => "Apply outcome remains unknown.",
                _ => "Reopen Update status.",
            };
            return self.say(format!(
                "The row target or actor changed; no further action taken. {outcome}"
            ));
        }
        let open = self
            .input
            .as_ref()
            .and_then(|input| input.row_send.as_ref())
            .and_then(|send| self.target_row(&send.target))
            .and_then(|row| row["waitingOnYou"].as_array())
            .cloned()
            .unwrap_or_default();
        let Some(draft) = &mut self.status_draft else {
            return Effect::None;
        };
        if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            draft.scroll = Some(if key.code == KeyCode::PageUp {
                draft.scroll.unwrap_or(0).saturating_sub(5)
            } else {
                draft.scroll.unwrap_or(0).saturating_add(5)
            });
            return Effect::None;
        }
        let applied = matches!(draft.outcome, Some(Outcome::Applied { .. }));
        if matches!(draft.outcome, Some(Outcome::Unknown(_))) {
            return Effect::None;
        }
        if applied {
            match key.code {
                KeyCode::Up => draft.focus = draft.focus.saturating_sub(1),
                KeyCode::Down => draft.focus = (draft.focus + 1).min(3),
                KeyCode::End => draft.focus = 3,
                _ => {}
            }
            if key.code == KeyCode::Enter
                && draft.focus == 3
                && let Some(Outcome::Applied {
                    intent,
                    notification: Err(_),
                    ..
                }) = &draft.outcome
                && !intent.accepted
                && let Some(preview) = &draft.preview
            {
                return Effect::Act(Request::Status(Box::new(Action::Retry {
                    preview: preview.clone(),
                    intent: intent.clone(),
                })));
            }
            return Effect::None;
        }
        if draft.focus == 2
            && (key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT)
                || key.code == KeyCode::Char('n') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            draft.reason.insert("\n", super::app::INPUT_LIMIT);
            return Effect::None;
        }
        if draft.focus == 2
            && !matches!(
                key.code,
                KeyCode::Up | KeyCode::Down | KeyCode::Enter | KeyCode::Tab
            )
        {
            let width = self.input_viewport.get().map_or(80, |(area, _)| area.width);
            draft.reason.key(key, width, super::app::INPUT_LIMIT);
            return Effect::None;
        }
        if draft.focus == 1
            && !matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Enter
                    | KeyCode::Tab
            )
        {
            draft.state.key(key, u16::MAX, super::app::INPUT_LIMIT);
            return Effect::None;
        }
        let max = 3 + usize::from(!open.is_empty());
        match key.code {
            KeyCode::Up => {
                draft.focus = draft.focus.saturating_sub(1);
                draft.scroll = None;
            }
            KeyCode::Down => {
                draft.focus = (draft.focus + 1).min(max);
                draft.scroll = None;
            }
            KeyCode::PageUp => draft.scroll = Some(draft.scroll.unwrap_or(0).saturating_sub(5)),
            KeyCode::PageDown => draft.scroll = Some(draft.scroll.unwrap_or(0).saturating_add(5)),
            KeyCode::Enter => match draft.focus {
                0 => draft.clear_pending = !draft.clear_pending,
                1 => draft.replace_state = !draft.replace_state,
                2 => {}
                3 => {
                    let Some(preview) = &draft.preview else {
                        return Effect::None;
                    };
                    return Effect::Act(Request::Status(Box::new(Action::Apply(Submit {
                        preview: preview.clone(),
                        clear_pending: draft.clear_pending,
                        state: draft.replace_state.then(|| draft.state.to_string()),
                        reason: draft.reason.as_str().to_owned(),
                    }))));
                }
                4 if !open.is_empty() => {
                    let Some(send) = self.input.as_ref().and_then(|input| input.row_send.clone())
                    else {
                        return Effect::None;
                    };
                    let squad = self.input.as_ref().unwrap().squad.clone();
                    return self.compose_row(send, crate::action::Verb::Talk, squad);
                }
                _ => {}
            },
            _ => {}
        }
        self.follow = true;
        Effect::None
    }
}

/// Measurement and paint consume these same wrapped lines. Focus reveals the
/// complete old/new values when a long preview exceeds the visible band.
pub(super) fn lines(app: &App, width: u16) -> Vec<(Option<usize>, String)> {
    let Some(input) = &app.input else {
        return vec![];
    };
    let mut lines = vec![(None, input.header())];
    let Some(draft) = &app.status_draft else {
        return vec![(None, "Loading raw metadata…".into())];
    };
    if let Some(preview) = &draft.preview {
        for (focus, label, old, next) in [
            (
                0,
                format!(
                    "[{}] Clear pending",
                    if draft.clear_pending { "x" } else { " " }
                ),
                &preview.pending,
                if draft.clear_pending {
                    "(empty)".into()
                } else {
                    "(unchanged)".into()
                },
            ),
            (
                1,
                format!(
                    "[{}] Replace state: {}{}{}",
                    if draft.replace_state { "x" } else { " " },
                    &draft.state[..draft.state.cursor()],
                    if draft.focus == 1 { "▏" } else { "" },
                    &draft.state[draft.state.cursor()..]
                ),
                &preview.state,
                if draft.replace_state {
                    format!("{:?}", draft.state.as_str())
                } else {
                    "(unchanged)".into()
                },
            ),
        ] {
            let old = old.as_ref().map_or_else(
                || "(absent)".into(),
                |value| {
                    if status_update::valid_value(value) {
                        format!("{value:?}")
                    } else {
                        format!("{value:?} (unsupported value)")
                    }
                },
            );
            lines.push((Some(focus), label));
            if width >= tmt_cli_style::breakpoint::LG.cells {
                lines.push((Some(focus), format!("  Old: {old} → New: {next}")));
            } else {
                lines.push((Some(focus), format!("  Old: {old}")));
                lines.push((Some(focus), format!("  New: {next}")));
            }
        }
        lines.push((
            Some(2),
            if draft.focus == 2 {
                "Reason:".into()
            } else {
                format!("Reason: {}", draft.reason)
            },
        ));
        let button = match &draft.outcome {
            Some(Outcome::Applied {
                intent,
                notification: Err(_),
                ..
            }) if !intent.accepted => "Retry notification only",
            Some(Outcome::Applied { .. }) => "Status applied",
            Some(Outcome::Unknown(_)) => "Outcome unknown — reopen only after inspecting metadata",
            _ => "Apply and notify",
        };
        lines.push((Some(3), button.into()));
    } else if draft.error.is_none() {
        lines.push((None, "Loading raw metadata…".into()));
    }
    if let Some(error) = &draft.error {
        lines.push((None, error.clone()));
    }
    if !matches!(
        draft.outcome,
        Some(Outcome::Applied { .. }) | Some(Outcome::Unknown(_))
    ) {
        let open = input
            .row_send
            .as_ref()
            .and_then(|send| app.target_row(&send.target))
            .and_then(|row| row["waitingOnYou"].as_array());
        lines.push((
            None,
            "Requires a reply (separate from manual status)".into(),
        ));
        if open.is_some_and(|open| !open.is_empty()) {
            lines.push((
                Some(4),
                "Message member (answers the newest open request)".into(),
            ));
        } else {
            lines.push((
                None,
                "No unanswered requests in the acquired window.".into(),
            ));
        }
    }
    lines
        .into_iter()
        .flat_map(|(focus, text)| {
            let text = super::notes::sanitize(&text);
            let field = focus.is_some();
            let mark = if focus == Some(draft.focus) {
                "› "
            } else {
                "  "
            };
            tmt_tui::text::lines(
                &text,
                width.saturating_sub(if field { 2 } else { 0 }),
                tmt_tui::style::TextFlow::Wrap,
            )
            .into_iter()
            .map(move |line| (focus, if field { format!("{mark}{line}") } else { line }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::app::workspace::tests::{MEMBER, ROOM};
    use super::super::app::{Compose, Input};
    use super::super::workspace::OpsTarget;
    use super::*;
    use serde_json::json;
    fn press(app: &mut App, key: KeyCode) -> Effect {
        app.key(KeyEvent::new(key, KeyModifiers::NONE))
    }
    fn app() -> App {
        super::super::app::tests::workspace_composer()
    }
    fn member_row(app: &mut App) -> &mut serde_json::Value {
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .rows
            .get_mut(&OpsTarget::Member {
                room_id: Some(ROOM.into()),
                identity_id: MEMBER.into(),
            })
            .unwrap()
    }
    fn ready(app: &mut App) -> Preview {
        app.perform(&crate::action::Action::parse("update-status").unwrap());
        let target = app.status_draft.as_ref().unwrap().target.clone().unwrap();
        let preview = Preview {
            target,
            room: ROOM.into(),
            namespace: "squad.product.".into(),
            keys: ["squad.product.pending".into(), "squad.product.state".into()],
            name: "auth-fix".into(),
            actor_name: "Ben".into(),
            pending: Some(" exact raw pending ".into()),
            state: Some("blocked".into()),
        };
        app.status_draft
            .as_mut()
            .unwrap()
            .loaded(Ok(preview.clone()));
        preview
    }
    #[test]
    fn status_state_and_reason_share_editor_keys_without_inserting_alt_letters() {
        let mut app = app();
        ready(&mut app);
        let draft = app.status_draft.as_mut().unwrap();
        draft.focus = 1;
        draft.state = "one 你我".into();
        app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
        app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(app.status_draft.as_ref().unwrap().state.as_str(), "one 我");
        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(app.status_draft.as_ref().unwrap().state.cursor(), 0);
        let draft = app.status_draft.as_mut().unwrap();
        draft.focus = 2;
        draft.reason = "one two".into();
        app.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));
        assert_eq!(app.status_draft.as_ref().unwrap().reason.as_str(), "one ");
        assert!(app.input.is_some());
    }

    #[test]
    fn status_reason_accepts_portable_and_distinct_newlines_without_submitting() {
        let mut app = app();
        ready(&mut app);
        app.status_draft.as_mut().unwrap().focus = 2;
        for (code, modifiers) in [
            (KeyCode::Char('n'), KeyModifiers::CONTROL),
            (KeyCode::Enter, KeyModifiers::SHIFT),
        ] {
            assert_eq!(app.key(KeyEvent::new(code, modifiers)), Effect::None);
        }
        assert_eq!(app.status_draft.as_ref().unwrap().reason.as_str(), "\n\n");
        assert!(app.status_draft.as_ref().unwrap().outcome.is_none());
    }

    #[test]
    fn no_preselected_changes_reason_bound_and_message_draft_are_independent() {
        let mut app = app();
        app.perform(&crate::action::Action::parse("talk").unwrap());
        app.paste("Message draft");
        let opening = ready(&mut app);
        assert_eq!(
            app.input.as_ref().unwrap().header(),
            "Update status → product / auth-fix"
        );
        let draft = app.status_draft.as_ref().unwrap();
        assert!(!draft.clear_pending && !draft.replace_state && draft.reason.is_empty());
        press(&mut app, KeyCode::Enter); // explicit pending choice
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        for c in "Approved".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        // The status reason has its own editor; it never changes the member draft.
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 42)).unwrap();
        terminal
            .draw(|frame| super::super::view::render(frame, &app))
            .unwrap();
        assert!(app.input_band.get().is_some());
        assert!(app.input_viewport.get().is_some());
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .contains("Approved")
        );
        assert!(app.input.as_ref().unwrap().text.is_empty());
        assert_eq!(app.status_draft.as_ref().unwrap().reason, "Approved");
        press(&mut app, KeyCode::Down);
        let Effect::Act(Request::Status(action)) = press(&mut app, KeyCode::Enter) else {
            panic!("must use typed status effect")
        };
        let Action::Apply(submit) = *action else {
            panic!("must apply chosen fields")
        };
        assert_eq!(submit.preview, opening);
        assert!(submit.clear_pending);
        assert_eq!(submit.state, None);
        assert_eq!(submit.reason, "Approved");
        app.status_draft.as_mut().unwrap().reason =
            "x".repeat(super::super::app::INPUT_LIMIT).into();
        app.status_draft.as_mut().unwrap().focus = 2;
        press(&mut app, KeyCode::Char('z'));
        assert_eq!(
            app.status_draft.as_ref().unwrap().reason.chars().count(),
            super::super::app::INPUT_LIMIT
        );
        press(&mut app, KeyCode::Esc);
        assert!(app.input.is_none() && app.status_draft.is_none());
        app.perform(&crate::action::Action::parse("talk").unwrap());
        assert_eq!(app.input.as_ref().unwrap().text, "Message draft");
    }
    #[test]
    fn status_is_explicit_and_its_message_action_uses_the_newest_request() {
        let mut app = app();
        member_row(&mut app)["waitingOnYou"] = json!([
            {"requestId":"q1","preparedAtMs":10,"preview":"First?"},
            {"requestId":"q2","preparedAtMs":20,"preview":"Second?"}
        ]);
        ready(&mut app);
        assert!(matches!(
            app.input.as_ref().unwrap().compose,
            Compose::Status
        ));
        app.status_draft.as_mut().unwrap().focus = 4;
        press(&mut app, KeyCode::Enter);
        assert!(
            matches!(&app.input.as_ref().unwrap().compose, Compose::Talk { to } if to == "auth-fix")
        );
        app.paste("Dismissed explicitly");
        assert!(
            matches!(press(&mut app, KeyCode::Enter), Effect::Act(Request::Reply { request, .. }) if request == "q2")
        );
        assert_eq!(
            member_row(&mut app)["waitingOnYou"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn unknown_outcome_never_submits_again_conflict_requires_new_enter() {
        let mut app = app();
        let mut preview = ready(&mut app);
        app.finished_status(Outcome::Unknown("unknown".into()));
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        preview.pending = Some("intervening update".into());
        app.finished_status(Outcome::Conflict(Ok(preview.clone())));
        assert_eq!(
            app.status_draft.as_ref().unwrap().preview.as_ref(),
            Some(&preview)
        );
        assert!(
            app.status_draft
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("submit again")
        );
        assert!(!app.status_draft.as_ref().unwrap().clear_pending);
        assert_eq!(app.status_draft.as_ref().unwrap().focus, 0);
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::None,
            "queued duplicate Enter cannot apply refreshed values"
        );
    }
    #[test]
    fn selected_field_values_stack_at_narrow_widths_without_losing_raw_spaces() {
        let mut app = app();
        ready(&mut app);
        for width in [160, 100, 80] {
            let text = lines(&app, width)
                .into_iter()
                .map(|(_, line)| line)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("\" exact raw pending \""));
            assert!(text.contains("Requires a reply"));
            if width < tmt_cli_style::breakpoint::LG.cells {
                assert!(text.lines().any(|line| line.trim() == "Old: \"blocked\""));
                assert!(text.lines().any(|line| line.trim().starts_with("New:")));
            }
        }
        // Worker request is exact UUID+actor+namespace source, not a display value.
        let Input { compose, .. } = app.input.as_ref().unwrap();
        assert_eq!(compose, &Compose::Status);
    }

    #[test]
    fn applied_failure_requires_explicit_retry_selection_and_never_marks_sent() {
        let mut app = app();
        let preview = ready(&mut app);
        app.status_draft.as_mut().unwrap().focus = 3;
        assert!(matches!(press(&mut app, KeyCode::Enter), Effect::Act(_)));
        let intent = crate::send::Intent::new(
            &preview.target.actor,
            vec![crate::send::LeadRecipient {
                squad: preview.target.squad.clone(),
                id: preview.target.identity.clone(),
                name: preview.name.clone(),
            }],
            "announcement",
            Some(&preview.room),
            "Frozen notification",
        )
        .unwrap();
        app.finished_status(Outcome::Applied {
            intent: intent.clone(),
            notification: Err("offline".into()),
        });
        assert!(matches!(
            app.status_draft.as_ref().unwrap().outcome,
            Some(Outcome::Applied {
                notification: Err(_),
                ..
            })
        ));
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        press(&mut app, KeyCode::End);
        let Effect::Act(Request::Status(action)) = press(&mut app, KeyCode::Enter) else {
            panic!("explicit selection must retry notification only")
        };
        assert_eq!(*action, Action::Retry { preview, intent });
        for width in [160, 100, 80] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 42)).unwrap();
            terminal
                .draw(|frame| super::super::view::render(frame, &app))
                .unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(
                !text.contains("✓ sent"),
                "retention must not claim delivery"
            );
            assert!(text.contains("Retry notification only"));
        }
        app.finished_status(Outcome::Unknown("Outcome unknown".into()));
        assert!(matches!(
            app.status_draft.as_ref().unwrap().outcome,
            Some(Outcome::Unknown(_))
        ));
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
    }

    #[test]
    fn invalidated_status_target_preserves_known_outcome_without_further_effects() {
        for unknown in [false, true] {
            for actor_changed in [false, true] {
                let mut app = app();
                let preview = ready(&mut app);
                let outcome = if unknown {
                    Outcome::Unknown("Apply outcome unknown".into())
                } else {
                    Outcome::Applied {
                        intent: crate::send::Intent::new(
                            &preview.target.actor,
                            vec![crate::send::LeadRecipient {
                                squad: preview.target.squad.clone(),
                                id: preview.target.identity.clone(),
                                name: preview.name.clone(),
                            }],
                            "announcement",
                            Some(&preview.room),
                            "Frozen notification",
                        )
                        .unwrap(),
                        notification: Err("offline".into()),
                    }
                };
                app.finished_status(outcome.clone());
                if actor_changed {
                    app.view.as_mut().unwrap().me = Some("Another actor".into());
                } else {
                    member_row(&mut app)["name"] = json!("Another target");
                }
                for key in [KeyCode::End, KeyCode::Enter, KeyCode::Enter] {
                    assert_eq!(
                        press(&mut app, key),
                        Effect::None,
                        "invalid context must emit no additional apply or dispatch request"
                    );
                }
                assert_eq!(app.status_draft.as_ref().unwrap().outcome, Some(outcome));
                assert_eq!(app.status_draft.as_ref().unwrap().preview, Some(preview));
                let notice = app.notice.as_ref().unwrap();
                assert!(notice.contains("no further action taken"));
                assert!(!notice.contains("Nothing applied"));
                assert!(notice.contains(if unknown {
                    "Apply outcome remains unknown"
                } else {
                    "Status was already applied; notification intent retained"
                }));
            }
        }
    }

    #[test]
    fn no_color_focus_moves_independently_of_checked_fields_without_effects() {
        fn draw(app: &App, width: u16) -> String {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 42)).unwrap();
            terminal
                .draw(|frame| super::super::view::render(frame, app))
                .unwrap();
            let band = app.input_band.get().unwrap();
            assert!(!app.hits.borrow().iter().any(|hit| {
                !ratatui::layout::Rect::new(hit.x, hit.y, hit.width, 1)
                    .intersection(band)
                    .is_empty()
            }));
            terminal
                .backend()
                .buffer()
                .content()
                .chunks(usize::from(width))
                .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        }
        for width in [80, 100, 160] {
            let mut app = app();
            app.view.as_mut().unwrap().look.depth = tmt_cli_style::Depth::None;
            member_row(&mut app)["waitingOnYou"] = json!([{"requestId":"req_11111111-1111-4111-8111-111111111111","preview":"First question with a wrapping preview that remains identifiable"},
                    {"requestId":"req_22222222-2222-4222-8222-222222222222","preview":"Second question"}]);
            let preview = ready(&mut app);
            assert!(draw(&app, width).contains("› [ ] Clear pending"));
            assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
            assert!(draw(&app, width).contains("› [x] Clear pending"));
            assert_eq!(press(&mut app, KeyCode::Down), Effect::None);
            let text = draw(&app, width);
            assert!(text.contains("[x] Clear pending") && !text.contains("› [x] Clear pending"));
            assert!(text.contains("› [ ] Replace state"));
            let mut previous = text;
            for label in [
                "Reason:",
                "Apply and notify",
                "Message member (answers the newest open request)",
            ] {
                assert_eq!(press(&mut app, KeyCode::Down), Effect::None);
                let text = draw(&app, width);
                assert!(text.contains(&format!("› {label}")), "{width}: {text}");
                assert_ne!(
                    text, previous,
                    "focus movement must remain visible without color"
                );
                previous = text;
            }
            assert!(app.status_draft.as_ref().unwrap().clear_pending);
            assert!(!app.status_draft.as_ref().unwrap().replace_state);
            let intent = crate::send::Intent::new(
                &preview.target.actor,
                vec![crate::send::LeadRecipient {
                    squad: preview.target.squad.clone(),
                    id: preview.target.identity.clone(),
                    name: preview.name.clone(),
                }],
                "announcement",
                Some(&preview.room),
                "Frozen notification",
            )
            .unwrap();
            app.finished_status(Outcome::Applied {
                intent,
                notification: Err("offline".into()),
            });
            assert!(!draw(&app, width).contains("› Retry notification only"));
            assert_eq!(press(&mut app, KeyCode::End), Effect::None);
            assert!(draw(&app, width).contains("› Retry notification only"));
        }
    }
}
