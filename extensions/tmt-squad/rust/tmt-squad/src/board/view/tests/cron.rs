//! The squad tab's jobs half: composition, focus transfer, scoped keys, row labels.
use super::*;
use crate::board::cronboard::{TEST_NOW, test_cron, test_view};
use std::time::Instant;
use tmt_squad::cron::ClockStatus;

fn members() -> Value {
    json!([{"rows": [
        row("alpha", "working", "task one", json!({"id": "u1"})),
        row("bravo", "idle", "task two", json!({"id": "u2"})),
        row("carol", "idle", "task three", json!({"id": "u3"})),
    ]}])
}

fn squad_tab() -> App {
    let mut app = board(members());
    let document = &mut app.view.as_mut().unwrap().document;
    document["squad"]["roomId"] = json!("room");
    let jobs = ["u1", "u3"].map(|owner| {
        let mut job = test_view(
            "someone",
            &format!("job for {owner}"),
            Some(TEST_NOW + 3_600_000),
        );
        job.job.squad = "product".into();
        job.job.room_id = "room".into();
        job.job.owner_id = Some(owner.into());
        job
    });
    let mut jobs = jobs.to_vec();
    // Distinct job ids come from the store; the fixture reuses one, so move the room.
    jobs[1].job.room_id = "room".into();
    app.cron
        .replace(Ok(test_cron(vec![jobs.remove(0)], ClockStatus::NoClock)));
    app
}

fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn members_sit_above_the_squads_jobs_with_the_clock_on_the_rule() {
    let app = squad_tab();
    for (width, height) in [(160, 30), (100, 24), (80, 24)] {
        let screen = draw(&app, width, height);
        let text = screen.join("\n");
        let rule = screen
            .iter()
            .position(|l| l.starts_with("── ⏱ cron · 1"))
            .expect(&text);
        assert!(screen[..rule].iter().any(|l| l.contains("alpha")), "{text}");
        assert!(screen[..rule].iter().any(|l| l.contains("carol")), "{text}");
        assert!(screen[rule].contains("no clock"), "{text}");
        assert!(screen[rule + 1].contains("every 30m"), "{text}");
        assert!(screen[rule + 1].contains("someone"), "{text}");
    }
}

#[test]
fn a_member_with_an_active_job_shows_its_next_run_and_the_label_drops_first() {
    let app = squad_tab();
    let wide = draw(&app, 120, 24);
    let alpha = wide.iter().find(|l| l.contains("alpha")).unwrap();
    // The label's clock text depends on the real date; its exact form is
    // covered where time is injected (`cronboard::tests`).
    let label = alpha.split_once("⏱ ").expect(alpha).1;
    assert!(label.contains(':') && alpha.ends_with(label), "{alpha}");
    let carol = wide.iter().find(|l| l.contains("carol")).unwrap();
    assert!(!carol.contains('⏱'), "{carol}");
    // Too narrow for the label: it goes, the row's columns stay.
    let narrow = draw(&app, 30, 24);
    let alpha = narrow.iter().find(|l| l.contains("alpha")).unwrap();
    assert!(!alpha.contains('⏱'), "{alpha}");
}

#[test]
fn tab_enters_the_jobs_half_after_the_last_pane_and_returns() {
    let mut app = squad_tab();
    draw(&app, 100, 24);
    assert!(!app.jobs_focus);
    press(&mut app, KeyCode::Tab);
    assert!(
        app.jobs_focus,
        "Tab after the last pane enters the jobs half"
    );
    let screen = draw(&app, 100, 24);
    let text = screen.join("\n");
    // Focus expands the selected job in place.
    assert!(text.contains("message  job for u1"), "{text}");
    assert!(
        text.contains("next     ") && text.contains("tz Asia/Tokyo"),
        "{text}"
    );
    press(&mut app, KeyCode::Tab);
    assert!(!app.jobs_focus && app.focused_pane() == Some(Pane::Rows));
    assert!(!draw(&app, 100, 24).join("\n").contains("message  job"));
}

#[test]
fn scoped_keys_navigate_jobs_and_never_act_on_a_member_row() {
    let mut app = squad_tab();
    draw(&app, 100, 24);
    press(&mut app, KeyCode::Tab);
    draw(&app, 100, 24);
    // Enter goes to the job's owner, not the selected member row.
    assert_eq!(
        press(&mut app, KeyCode::Enter),
        Effect::Act(crate::board::app::Request::Jump("someone".into()))
    );
    // The preset's member keys refuse while the jobs half has focus.
    for key in ['y', 't', 'r'] {
        assert_eq!(press(&mut app, KeyCode::Char(key)), Effect::None);
        assert!(
            app.notice
                .as_deref()
                .unwrap_or("")
                .contains("acts on a member row"),
            "{key}: {:?}",
            app.notice
        );
    }
    assert_eq!(app.selected, 0, "the member selection never moved");
}

#[test]
fn a_user_binding_wins_over_the_scoped_c_key_and_other_tabs_have_no_half() {
    let mut app = squad_tab();
    draw(&app, 100, 24);
    press(&mut app, KeyCode::Tab);
    app.view
        .as_mut()
        .unwrap()
        .configured_bindings
        .insert("c".into(), crate::action::Action::parse("refresh").unwrap());
    app.view
        .as_mut()
        .unwrap()
        .bindings
        .insert("c".into(), crate::action::Action::parse("refresh").unwrap());
    assert_eq!(press(&mut app, KeyCode::Char('c')), Effect::Refresh);
    let mut leads = squad_tab();
    leads.view.as_mut().unwrap().document["squad"]
        .as_object_mut()
        .unwrap()
        .remove("roomId");
    assert!(!draw(&leads, 100, 24).join("\n").contains("⏱ cron"));
}

#[test]
fn a_short_body_keeps_the_rule_only_and_a_pointer_press_focuses_the_half() {
    let app = squad_tab();
    let short = draw(&app, 100, 11);
    assert!(short.iter().any(|l| l.starts_with("── ⏱ cron · 1")));
    assert!(!short.join("\n").contains("every 30m"));
    let mut app = squad_tab();
    let screen = draw(&app, 100, 24);
    let job = screen.iter().position(|l| l.contains("every 30m")).unwrap();
    let click = |kind| MouseEvent {
        kind,
        column: 10,
        row: job as u16,
        modifiers: KeyModifiers::NONE,
    };
    app.mouse(
        click(MouseEventKind::Down(MouseButton::Left)),
        Instant::now(),
    );
    assert!(app.jobs_focus);
    app.mouse(
        MouseEvent {
            row: 2,
            ..click(MouseEventKind::Down(MouseButton::Left))
        },
        Instant::now(),
    );
    assert!(!app.jobs_focus, "a press elsewhere gives the focus back");
}

#[test]
fn a_failed_read_without_data_says_so_in_the_half() {
    let mut app = board(members());
    app.view.as_mut().unwrap().document["squad"]["roomId"] = json!("room");
    app.cron.replace(Err("storage unreachable".into()));
    let text = draw(&app, 100, 24).join("\n");
    assert!(
        text.contains("⏱ cron · 0 · ✗ storage unreachable"),
        "{text}"
    );
    assert!(text.contains("(jobs unavailable)"), "{text}");
}

mod controls {
    use super::*;
    use crate::board::{
        app::{Choice, Compose, Request},
        cronboard::{CronRequest, Op},
    };
    use crate::cron_service::{CronActor, JobKey};

    fn actor() -> CronActor {
        CronActor {
            id: "user-id".into(),
            name: "Ben".into(),
        }
    }

    /// A squad tab whose jobs half has focus on its single job `c0`.
    fn focused() -> (App, JobKey) {
        let mut app = squad_tab();
        draw(&app, 100, 24);
        press(&mut app, KeyCode::Tab);
        draw(&app, 100, 24);
        let key = JobKey::of(&app.cron.cron.as_ref().unwrap().jobs[0].job);
        (app, key)
    }

    fn typed(app: &mut App, text: &str) {
        for character in text.chars() {
            press(app, KeyCode::Char(character));
        }
    }

    fn existing(key: &JobKey, op: Op) -> Effect {
        Effect::Act(Request::Cron(CronRequest::Existing {
            actor: actor(),
            key: key.clone(),
            revision: 1,
            op,
        }))
    }

    #[test]
    fn keys_are_job_keys_only_while_the_jobs_half_has_focus() {
        let mut app = squad_tab();
        draw(&app, 100, 24);
        // On the members, `n` is the notes preset and `x`, `p`, `e` do nothing.
        for key in ['p', 'x', 'e', 'o', 'd', 'n'] {
            let effect = press(&mut app, KeyCode::Char(key));
            assert!(
                !matches!(effect, Effect::Act(Request::Cron(_)))
                    && app.input.is_none()
                    && app.menu.is_none(),
                "{key}"
            );
        }
    }

    #[test]
    fn pause_resume_and_send_carry_actor_key_and_the_viewed_revision() {
        let (mut app, key) = focused();
        assert_eq!(
            press(&mut app, KeyCode::Char('p')),
            existing(&key, Op::Pause)
        );
        assert_eq!(
            press(&mut app, KeyCode::Char('x')),
            existing(&key, Op::Send)
        );
        app.cron.cron.as_mut().unwrap().jobs[0].job.pause = Some(tmt_squad::cron::Pause {
            by: "u".into(),
            at_ms: 0,
        });
        assert_eq!(
            press(&mut app, KeyCode::Char('p')),
            existing(&key, Op::Resume)
        );
        app.cron.cron.as_mut().unwrap().jobs[0].job.owner_id = None;
        press(&mut app, KeyCode::Char('p'));
        assert!(app.notice.as_deref().unwrap().contains("no owner"));
    }

    #[test]
    fn delete_asks_first_and_the_default_keeps_the_job() {
        let (mut app, key) = focused();
        assert_eq!(press(&mut app, KeyCode::Char('d')), Effect::None);
        let menu = app.menu.as_ref().expect("a confirmation");
        assert!(menu.title.contains("delete product c0"));
        assert_eq!(menu.entries[0].choice, Choice::Dismiss);
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::None,
            "Enter keeps the job"
        );
        assert!(app.menu.is_none());
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(
            press(&mut app, KeyCode::Char('y')),
            existing(&key, Op::Remove)
        );
    }

    #[test]
    fn edit_prefills_writes_only_changed_fields_and_cancel_writes_nothing() {
        let (mut app, key) = focused();
        press(&mut app, KeyCode::Char('e'));
        let input = app.input.as_ref().unwrap();
        assert_eq!(
            (input.text.as_str(), &input.compose),
            ("job for u1", &Compose::Cron)
        );
        assert!(input.prompt.contains("edit product c0 · message"));
        let footer = draw(&app, 100, 24).pop().unwrap();
        assert!(
            footer.starts_with("edit product c0 · message › job for u1▏"),
            "{footer}"
        );
        // Nothing changed at either step: nothing is written.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.input.as_ref().unwrap().text, "every 30m");
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert!(app.input.is_none() && app.notice.as_deref().unwrap().contains("nothing written"));
        // An invalid schedule keeps the step, the text and says why.
        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Enter);
        for _ in 0..9 {
            press(&mut app, KeyCode::Backspace);
        }
        typed(&mut app, "soon");
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        let input = app.input.as_ref().unwrap();
        assert_eq!(input.text, "soon");
        assert!(input.hint.as_ref().unwrap().error);
        assert!(input.prompt.ends_with("schedule"));
        // Fixing it submits the changed fields only, with the message untouched.
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        typed(&mut app, "daily 09:00");
        let Effect::Act(Request::Cron(request)) = press(&mut app, KeyCode::Enter) else {
            panic!("a changed schedule is submitted");
        };
        assert_eq!(
            request,
            CronRequest::Existing {
                actor: actor(),
                key: key.clone(),
                revision: 1,
                op: Op::Edit {
                    message: None,
                    schedule: Some("daily 09:00".into()),
                    zone: "Asia/Tokyo".into()
                }
            }
        );
        // Esc at any step discards the draft.
        press(&mut app, KeyCode::Char('e'));
        typed(&mut app, "x");
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.input.is_none() && app.cron_draft.is_none());
        assert!(app.notice.as_deref().unwrap().contains("nothing changed"));
    }

    #[test]
    fn a_multi_line_message_is_kept_as_stored_and_only_the_schedule_is_edited() {
        let (mut app, _) = focused();
        app.cron.cron.as_mut().unwrap().jobs[0].job.message = "one\ntwo".into();
        press(&mut app, KeyCode::Char('e'));
        let input = app.input.as_ref().unwrap();
        assert!(input.prompt.ends_with("schedule"), "{}", input.prompt);
        assert!(input.hint.as_ref().unwrap().text.contains("several lines"));
    }

    #[test]
    fn new_and_reassign_collect_owner_message_and_schedule_exactly() {
        let (mut app, key) = focused();
        // A member row is selected on the members; Tab back and use `n` there.
        press(&mut app, KeyCode::Tab);
        draw(&app, 100, 24);
        press(&mut app, KeyCode::Tab);
        draw(&app, 100, 24);
        assert!(app.jobs_focus);
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(
            app.input.as_ref().unwrap().text,
            "alpha",
            "the selected member is the default owner"
        );
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert!(
            app.input.as_ref().unwrap().hint.as_ref().unwrap().error,
            "an empty message is refused"
        );
        typed(&mut app, "  spaced  ");
        press(&mut app, KeyCode::Enter);
        typed(&mut app, "  weekdays 09:00  ");
        let Effect::Act(Request::Cron(request)) = press(&mut app, KeyCode::Enter) else {
            panic!("a complete form is submitted");
        };
        let CronRequest::Add {
            squad,
            room_id,
            owner,
            message,
            schedule,
            ..
        } = request
        else {
            panic!("an add");
        };
        assert_eq!((squad.as_str(), room_id.as_str()), ("product", "room"));
        assert_eq!(
            (owner.as_str(), message.as_str(), schedule.as_str()),
            ("alpha", "  spaced  ", "weekdays 09:00")
        );
        // Reassign asks only for the owner, prefilled with the current one.
        press(&mut app, KeyCode::Char('o'));
        assert_eq!(app.input.as_ref().unwrap().text, "someone");
        for _ in 0..7 {
            press(&mut app, KeyCode::Backspace);
        }
        typed(&mut app, "carol");
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            existing(
                &key,
                Op::Reassign {
                    owner: "carol".into()
                }
            )
        );
    }

    #[test]
    fn controls_need_a_known_actor_and_a_selected_job() {
        let (mut app, _) = focused();
        app.cron.cron.as_mut().unwrap().actor =
            Err("Record yourself with tmt squad me <name>".into());
        press(&mut app, KeyCode::Char('p'));
        assert!(
            app.notice.as_deref().unwrap().contains("known user"),
            "{:?}",
            app.notice
        );
        app.cron.cron.as_mut().unwrap().jobs.clear();
        press(&mut app, KeyCode::Char('x'));
        assert!(
            app.notice.as_deref().unwrap().contains("Select a job"),
            "{:?}",
            app.notice
        );
    }

    #[test]
    fn help_and_the_footer_show_the_scoped_keys_while_jobs_have_focus() {
        let (mut app, _) = focused();
        let screen = draw(&app, 120, 24);
        let footer = screen.last().unwrap();
        assert!(footer.starts_with("⏎ owner  n new  e edit"), "{footer}");
        assert!(
            footer.contains("? more") && footer.ends_with("q quit"),
            "{footer}"
        );
        let narrow = draw(&app, 40, 24);
        assert!(
            narrow.last().unwrap().ends_with("q quit"),
            "{:?}",
            narrow.last()
        );
        press(&mut app, KeyCode::Char('?'));
        let text = draw(&app, 120, 40).join("\n");
        assert!(
            text.contains("cron jobs") && text.contains("go to the job's owner"),
            "{text}"
        );
        assert!(
            text.contains("delete the job after a confirmation"),
            "{text}"
        );
    }

    #[test]
    fn the_c_list_runs_the_same_controls_and_forms_close_it() {
        let (mut app, key) = focused();
        press(&mut app, KeyCode::Char('c'));
        assert!(app.cron_list.is_some());
        draw(&app, 100, 24);
        // A pause keeps the list open to show the result.
        assert_eq!(
            press(&mut app, KeyCode::Char('p')),
            existing(&key, Op::Pause)
        );
        assert!(app.cron_list.is_some());
        // A form needs the input line, so it closes the list.
        press(&mut app, KeyCode::Char('e'));
        assert!(app.cron_list.is_none() && app.input.is_some());
    }
}

fn snapshots() -> Value {
    let mut frames = Vec::new();
    for (scenario, focused) in [("members above jobs", false), ("jobs focused", true)] {
        for (width, height) in [(160u16, 24u16), (100, 24), (80, 24)] {
            let mut app = squad_tab();
            draw(&app, width, height);
            if focused {
                press(&mut app, KeyCode::Tab);
            }
            frames.push(json!({
                "scenario": scenario, "width": width, "height": height,
                "lines": draw(&app, width, height),
                "hits": format!("{:?}", app.hits.borrow()),
            }));
        }
    }
    json!(frames)
}

#[test]
fn split_squad_tab_snapshots_at_each_width() {
    assert_eq!(
        snapshots(),
        serde_json::from_str::<Value>(include_str!("cron_snapshots.json")).unwrap()
    );
}

#[test]
#[ignore = "explicit initial captures of the split squad tab; frozen parity is separate"]
fn record_split_squad_tab_snapshots() {
    std::fs::write(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/board/view/tests/cron_snapshots.json"
        ),
        serde_json::to_string_pretty(&snapshots()).unwrap() + "\n",
    )
    .unwrap();
}
