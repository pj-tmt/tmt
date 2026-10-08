//! The squad tab's jobs half: composition, focus transfer, scoped keys, row labels.
use super::*;
use crate::board::cronboard::{TEST_NOW, test_cron, test_view, test_views};
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
    document["squad"]["lead"] = json!({"name": "sol"});
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
    let jobs = vec![jobs.remove(0)];
    // The first read may precede the board's own clock; a second one is settled.
    app.cron
        .replace(Ok(test_cron(jobs.clone(), ClockStatus::NoClock)));
    app.cron.replace(Ok(test_cron(jobs, ClockStatus::NoClock)));
    // The cursor starts on the lead; these cases are about the members.
    app.select(1);
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
            .position(|l| l.starts_with("── cron · 1"))
            .expect(&text);
        assert!(screen[..rule].iter().any(|l| l.contains("alpha")), "{text}");
        assert!(screen[..rule].iter().any(|l| l.contains("carol")), "{text}");
        assert!(screen[rule].contains("no clock"), "{text}");
        assert!(screen[rule + 1].contains("every 30m"), "{text}");
        assert!(screen[rule + 1].contains("someone"), "{text}");
    }
}

fn running_in(pane: &str) -> ClockStatus {
    ClockStatus::Running(tmt_squad::cron::Holder {
        pane: Some(pane.into()),
        pid: 7,
        since_ms: TEST_NOW - 60_000,
        expires_ms: TEST_NOW + 5_000,
    })
}

fn rule_of(app: &App) -> String {
    draw(app, 120, 30)
        .into_iter()
        .find(|line| line.starts_with("── cron"))
        .expect("the rule line")
}

#[test]
fn the_half_is_content_sized_and_capped_at_two_fifths_of_the_body() {
    let mut app = squad_tab();
    let below = |app: &App, height: u16| {
        let screen = draw(app, 120, height);
        let rule = screen
            .iter()
            .position(|l| l.starts_with("── cron"))
            .unwrap();
        // The footer is the last line; the half is everything between.
        screen.len() - 1 - rule
    };
    // One job: the rule, the job and the three-line minimum, not half the body.
    assert_eq!(below(&app, 40), 3);
    // Focus leaves the job collapsed; only e reserves its shared detail lines.
    press(&mut app, KeyCode::Tab);
    assert_eq!(below(&app, 40), 3);
    press(&mut app, KeyCode::Char('e'));
    assert_eq!(below(&app, 40), 3);
    let capped = below(&app, 20);
    assert!((3..=7).contains(&capped), "{capped}");
}

#[test]
fn the_clock_reads_checking_until_a_second_read_and_the_holder_shows_its_window() {
    let mut app = board(members());
    app.view.as_mut().unwrap().document["squad"]["roomId"] = json!("room");
    app.cron
        .replace(Ok(test_cron(Vec::new(), ClockStatus::NoClock)));
    assert!(
        rule_of(&app).contains("clock: checking…"),
        "{}",
        rule_of(&app)
    );
    app.cron
        .replace(Ok(test_cron(Vec::new(), ClockStatus::NoClock)));
    let settled = rule_of(&app);
    assert!(
        settled.contains("no clock") && !settled.contains("checking"),
        "{settled}"
    );
    // A holder the worker resolved shows session:window, not the pane id.
    let mut resolved = test_cron(Vec::new(), running_in("%7"));
    resolved.place = Some("team:agents".into());
    app.cron.replace(Ok(resolved));
    let rule = rule_of(&app);
    assert!(
        rule.contains("clock: team:agents ·") && !rule.contains("%7"),
        "{rule}"
    );
    // Unresolved: the pane id.
    app.cron
        .replace(Ok(test_cron(Vec::new(), running_in("%9"))));
    assert!(rule_of(&app).contains("clock: %9"), "{}", rule_of(&app));
}

#[test]
fn the_member_detail_shows_the_next_run_too() {
    let mut app = squad_tab();
    let first = detail_text(&detail_buffer(&app, 80, 12)).join("\n");
    assert!(
        first.contains("cron:") && first.contains("job for u1"),
        "{first}"
    );
    // A member without an active job has no cron line.
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('j'));
    let carol = detail_text(&detail_buffer(&app, 80, 12)).join("\n");
    assert!(
        carol.contains("carol") && !carol.contains("cron:"),
        "{carol}"
    );
}

#[test]
fn a_member_with_an_active_job_shows_its_next_run_and_the_label_drops_first() {
    let app = squad_tab();
    let wide = draw(&app, 120, 24);
    let alpha = wide.iter().find(|l| l.contains("alpha")).unwrap();
    // The label's clock text depends on the real date; its exact form is
    // covered where time is injected (`cronboard::tests`).
    let label = alpha.split_once("cron ").expect(alpha).1;
    assert!(label.contains(':') && alpha.ends_with(label), "{alpha}");
    let carol = wide.iter().find(|l| l.contains("carol")).unwrap();
    assert!(!carol.contains("cron "), "{carol}");
    // Too narrow for the label: it goes, the row's columns stay.
    let narrow = draw(&app, 30, 24);
    let alpha = narrow.iter().find(|l| l.contains("alpha")).unwrap();
    assert!(!alpha.contains("cron "), "{alpha}");
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
    assert!(
        !text.contains("prompt "),
        "focus leaves jobs collapsed\n{text}"
    );
    press(&mut app, KeyCode::Char('e'));
    let expanded = draw(&app, 60, 40).join("\n");
    assert!(
        expanded.contains("prompt")
            && expanded.contains("job for u1")
            && !expanded.contains("│when")
            && expanded.contains("│next"),
        "{expanded}"
    );
    press(&mut app, KeyCode::Tab);
    assert!(!app.jobs_focus && app.focused_pane() == Some(Pane::Rows));
    assert!(
        draw(&app, 60, 40).join("\n").contains("prompt"),
        "expansion survives focus changes"
    );
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('e'));
    assert!(!draw(&app, 60, 40).join("\n").contains("prompt"));
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
    assert_eq!(app.selected, 1, "the member selection never moved");
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
    assert!(!draw(&leads, 100, 24).join("\n").contains("── cron"));
}

#[test]
fn a_short_body_keeps_the_rule_only_and_a_pointer_press_focuses_the_half() {
    let app = squad_tab();
    let short = draw(&app, 100, 11);
    assert!(short.iter().any(|l| l.starts_with("── cron · 1")));
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
    assert!(text.contains("cron · 0 · ✗ storage unreachable"), "{text}");
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
        // Member keys cannot perform cron operations.
        for key in ['p', 'x', 'o', 'd', 'n'] {
            let effect = press(&mut app, KeyCode::Char(key));
            assert!(
                !matches!(effect, Effect::Act(Request::Cron(_)))
                    && app.input.is_none()
                    && app.menu.is_none(),
                "{key}"
            );
        }
        let effect = press(&mut app, KeyCode::Char('e'));
        assert!(!matches!(effect, Effect::Act(Request::Cron(_))));
        assert!(app.input.is_none());
        assert!(
            app.row_details
                .contains(&crate::board::row_detail::Target::Row(
                    app.row_target(app.selected).unwrap()
                ))
        );
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
        press(&mut app, KeyCode::Char('E'));
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
        press(&mut app, KeyCode::Char('E'));
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
        press(&mut app, KeyCode::Char('E'));
        typed(&mut app, "x");
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.input.is_none() && app.cron_draft.is_none());
        assert!(app.notice.as_deref().unwrap().contains("nothing changed"));
    }

    #[test]
    fn a_multi_line_message_is_kept_as_stored_and_only_the_schedule_is_edited() {
        let (mut app, _) = focused();
        app.cron.cron.as_mut().unwrap().jobs[0].job.message = "one\ntwo".into();
        press(&mut app, KeyCode::Char('E'));
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
        assert!(
            footer.starts_with("⏎ owner  n new  e expand  E edit"),
            "{footer}"
        );
        assert!(
            footer.contains("q quit") && footer.ends_with("? more"),
            "{footer}"
        );
        let narrow = draw(&app, 40, 24);
        assert!(
            narrow.last().unwrap().ends_with("? more"),
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
        press(&mut app, KeyCode::Char('E'));
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

/// A squad tab with `count` jobs of its own room, `u1` owning them all.
fn many_jobs(count: usize) -> App {
    let mut app = board(members());
    app.view.as_mut().unwrap().document["squad"]["roomId"] = json!("room");
    let jobs = test_views(count, "product", "room");
    app.cron
        .replace(Ok(test_cron(jobs.clone(), ClockStatus::NoClock)));
    app.cron.replace(Ok(test_cron(jobs, ClockStatus::NoClock)));
    app
}

fn walk_ids(count: usize) -> Vec<String> {
    (1..=count).map(|id| format!("room/c{id}")).collect()
}

#[test]
fn walking_the_jobs_half_moves_one_job_per_press_without_auto_expansion() {
    let order = walk_ids(8);
    for (down, up) in [
        (KeyCode::Down, KeyCode::Up),
        (KeyCode::Char('j'), KeyCode::Char('k')),
    ] {
        for (width, height) in [(100, 40), (80, 24)] {
            let mut app = many_jobs(8);
            draw(&app, width, height);
            press(&mut app, KeyCode::Tab);
            draw(&app, width, height);
            assert_eq!(app.jobs_selected().as_deref(), Some(order[0].as_str()));
            let members = app.selected;
            let step = |app: &mut App, code: KeyCode, expected: &str| {
                press(app, code);
                let screen = draw(app, width, height);
                assert_eq!(
                    app.jobs_selected().as_deref(),
                    Some(expected),
                    "{code:?} at {width}x{height}\n{}",
                    screen.join("\n")
                );
                assert_eq!(app.selected, members, "the member selection never moves");
                let cid = expected.rsplit('/').next().unwrap();
                assert!(
                    screen
                        .iter()
                        .any(|line| line.contains(&format!(" {cid} ")) && line.contains("●")),
                    "{cid} painted\n{}",
                    screen.join("\n")
                );
                assert!(
                    app.row_details.expanded.is_empty(),
                    "navigation never auto-expands"
                );
            };
            for expected in &order[1..] {
                step(&mut app, down, expected);
            }
            // A press past either end stays put, and never reaches the member rows.
            step(&mut app, down, &order[7]);
            for expected in order[..order.len() - 1].iter().rev() {
                step(&mut app, up, expected);
            }
            step(&mut app, up, &order[0]);
        }
    }
}

#[test]
fn walking_the_c_list_from_the_board_moves_one_job_per_press() {
    let order = walk_ids(8);
    for (down, up) in [
        (KeyCode::Down, KeyCode::Up),
        (KeyCode::Char('j'), KeyCode::Char('k')),
    ] {
        for (width, height) in [(100, 40), (80, 24)] {
            let mut app = many_jobs(8);
            draw(&app, width, height);
            press(&mut app, KeyCode::Tab);
            draw(&app, width, height);
            press(&mut app, KeyCode::Char('c'));
            draw(&app, width, height);
            let selected = |app: &App| app.cron_list.as_ref().unwrap().selected();
            assert_eq!(selected(&app).as_deref(), Some(order[0].as_str()));
            let step = |app: &mut App, code: KeyCode, expected: &str| {
                press(app, code);
                let screen = draw(app, width, height);
                assert_eq!(
                    selected(app).as_deref(),
                    Some(expected),
                    "{code:?} at {width}x{height}\n{}",
                    screen.join("\n")
                );
                let cid = expected.rsplit('/').next().unwrap();
                assert!(
                    screen.iter().any(|line| line.contains(&format!("›{cid} "))),
                    "{cid} painted\n{}",
                    screen.join("\n")
                );
            };
            for expected in &order[1..] {
                step(&mut app, down, expected);
            }
            // A press past either end stays put, and never reaches the member rows.
            step(&mut app, down, &order[7]);
            for expected in order[..order.len() - 1].iter().rev() {
                step(&mut app, up, expected);
            }
            step(&mut app, up, &order[0]);
        }
    }
}

#[test]
#[ignore = "read-only snapshot inspection before fixture approval"]
fn dump_split_squad_tab_snapshots() {
    let path = std::env::var("SQUAD_SNAPSHOTS_OUT").expect("SQUAD_SNAPSHOTS_OUT");
    std::fs::write(
        path,
        serde_json::to_string_pretty(&snapshots()).unwrap() + "\n",
    )
    .unwrap();
}

#[test]
fn job_expansion_is_explicit_shared_between_surfaces_and_pruned_on_removal() {
    use crate::board::row_detail::Target;
    let mut app = many_jobs(2);
    draw(&app, 160, 40);
    press(&mut app, KeyCode::Tab);
    draw(&app, 160, 40);
    let first = app.jobs_selected().unwrap();
    assert!(app.row_details.expanded.is_empty());
    app.cron.cron.as_mut().unwrap().actor = Err("actor unavailable".into());
    press(&mut app, KeyCode::Char('e'));
    assert!(app.input.is_none() && app.row_details.contains(&Target::Job(first.clone())));
    let shown = draw(&app, 160, 40).join("\n");
    assert!(shown.contains("no details yet"), "{shown}");
    press(&mut app, KeyCode::Down);
    draw(&app, 160, 40);
    let second = app.jobs_selected().unwrap();
    assert_ne!(first, second);
    assert_eq!(app.row_details.expanded, vec![Target::Job(first.clone())]);
    press(&mut app, KeyCode::Char('c'));
    draw(&app, 160, 40);
    assert_eq!(
        app.cron_list.as_ref().unwrap().selected().as_deref(),
        Some(second.as_str())
    );
    press(&mut app, KeyCode::Char('e'));
    let shown = draw(&app, 160, 40).join("\n");
    assert!(app.cron_list.is_some() && app.input.is_none());
    assert_eq!(app.row_details.expanded.len(), 2);
    assert!(
        shown.contains("no details yet") && shown.contains("E edit"),
        "{shown}"
    );
    press(&mut app, KeyCode::Char('e'));
    assert_eq!(app.row_details.expanded, vec![Target::Job(first)]);
    app.cron.cron.as_mut().unwrap().jobs.clear();
    app.reconcile_row_details();
    assert!(app.row_details.expanded.is_empty());
    let shown = draw(&app, 160, 40).join("\n");
    assert!(!shown.contains("E edit"), "{shown}");
}
