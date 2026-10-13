//! Current workspace Timers: scoped controls, exact forms, clock and navigation.
use super::*;
use crate::action::Action;
use crate::board::cronboard::{TEST_NOW, test_cron, test_view, test_views};
use crate::board::{
    app::workspace::tests::{MEMBER, ROOM},
    workspace::OpsTarget,
};
use tmt_ops::cron::ClockStatus;

fn squad_tab() -> App {
    let mut app = crate::board::app::workspace::tests::board();
    let home = app.view.as_mut().unwrap().home.as_mut().unwrap();
    home.workspace.roster.members.get_mut(MEMBER).unwrap().name = "alpha".into();
    let target = OpsTarget::Member {
        room_id: Some(ROOM.into()),
        identity_id: MEMBER.into(),
    };
    home.workspace.rows.get_mut(&target).unwrap()["name"] = json!("alpha");
    home.workspace.rows.get_mut(&target).unwrap()["squad"] = json!("product");
    let mut job = test_view("someone", "job for u1", Some(TEST_NOW + 3_600_000));
    job.job.squad = "product".into();
    job.job.room_id = ROOM.into();
    job.job.owner_id = Some(MEMBER.into());
    app.cron
        .replace(Ok(test_cron(vec![job.clone()], ClockStatus::NoClock)));
    app.cron
        .replace(Ok(test_cron(vec![job], ClockStatus::NoClock)));
    app
}

fn open_timers(app: &mut App) {
    assert_eq!(app.perform(&Action::parse("timers").unwrap()), Effect::None);
    assert_eq!(app.cron_list.as_ref().unwrap().scope.as_deref(), Some(ROOM));
    draw(app, 100, 24);
}

fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn running_in(pane: &str) -> ClockStatus {
    ClockStatus::Running(tmt_ops::cron::Holder {
        pane: Some(pane.into()),
        pid: 7,
        since_ms: TEST_NOW - 60_000,
        expires_ms: TEST_NOW + 5_000,
    })
}

fn rule_of(app: &App) -> String {
    draw(app, 120, 30)
        .into_iter()
        .find(|line| line.contains("clock:") || line.contains("no clock"))
        .expect("the rule line")
}

#[test]
fn the_clock_reads_checking_until_a_second_read_and_the_holder_shows_its_window() {
    let mut app = squad_tab();
    app.cron = Default::default();
    app.cron
        .replace(Ok(test_cron(Vec::new(), ClockStatus::NoClock)));
    open_timers(&mut app);
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

    /// The public workspace Timers action selects its scoped job `c0`.
    fn focused() -> (App, JobKey) {
        let mut app = squad_tab();
        open_timers(&mut app);
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
        app.cron.cron.as_mut().unwrap().jobs[0].job.pause = Some(tmt_ops::cron::Pause {
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
        open_timers(&mut app);
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
            footer.starts_with("edit product c0 · message › job for u1"),
            "{footer}"
        );
        // Nothing changed at either step: nothing is written.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.input.as_ref().unwrap().text, "every 30m");
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert!(app.input.is_none() && app.notice.as_deref().unwrap().contains("nothing written"));
        // An invalid schedule keeps the step, the text and says why.
        open_timers(&mut app);
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
        open_timers(&mut app);
        press(&mut app, KeyCode::Char('E'));
        typed(&mut app, "x");
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.input.is_none() && app.cron_draft.is_none());
        assert!(app.notice.as_deref().unwrap().contains("nothing changed"));
    }

    #[test]
    fn multiline_message_is_editable_and_unchanged_submission_preserves_it() {
        let (mut app, _) = focused();
        app.cron.cron.as_mut().unwrap().jobs[0].job.message = "one\ntwo".into();
        press(&mut app, KeyCode::Char('E'));
        let input = app.input.as_ref().unwrap();
        assert!(input.prompt.ends_with("message"), "{}", input.prompt);
        assert_eq!(input.text, "one\ntwo");
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert!(app.input.as_ref().unwrap().prompt.ends_with("schedule"));
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert!(app.notice.as_deref().unwrap().contains("nothing written"));
    }

    #[test]
    fn new_and_reassign_collect_owner_message_and_schedule_exactly() {
        let (mut app, key) = focused();
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
        assert_eq!((squad.as_str(), room_id.as_str()), ("product", ROOM));
        assert_eq!(
            (owner.as_str(), message.as_str(), schedule.as_str()),
            ("alpha", "  spaced  ", "weekdays 09:00")
        );
        // Reassign asks only for the owner, prefilled with the current one.
        open_timers(&mut app);
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
    fn an_empty_scoped_list_can_create_a_job_for_the_selected_member() {
        let mut app = squad_tab();
        app.cron.cron.as_mut().unwrap().jobs.clear();
        open_timers(&mut app);
        assert!(app.cron_list.as_ref().unwrap().selected().is_none());
        press(&mut app, KeyCode::Char('n'));
        assert!(app.cron_list.is_none());
        assert_eq!(app.input.as_ref().unwrap().text, "alpha");
        press(&mut app, KeyCode::Enter);
        typed(&mut app, "Review the queue");
        press(&mut app, KeyCode::Enter);
        typed(&mut app, "every 30m");
        let Effect::Act(Request::Cron(CronRequest::Add {
            actor: submitted_actor,
            squad,
            room_id,
            owner,
            message,
            schedule,
            ..
        })) = press(&mut app, KeyCode::Enter)
        else {
            panic!("a complete new job");
        };
        assert_eq!(submitted_actor, actor());
        assert_eq!((squad.as_str(), room_id.as_str()), ("product", ROOM));
        assert_eq!(
            (owner.as_str(), message.as_str(), schedule.as_str()),
            ("alpha", "Review the queue", "every 30m")
        );
    }

    #[test]
    fn controls_need_a_known_actor_and_a_selected_job() {
        let (mut app, _) = focused();
        app.cron.cron.as_mut().unwrap().actor =
            Err("Record yourself with tmt ops squad me <name>".into());
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
    fn timers_controls_keep_the_list_and_forms_close_it() {
        let (mut app, key) = focused();
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
    for (scenario, focused) in [("workspace tree", false), ("scoped Timers", true)] {
        for (width, height) in [(160u16, 24u16), (100, 24), (80, 24)] {
            let mut app = squad_tab();
            draw(&app, width, height);
            if focused {
                open_timers(&mut app);
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

/// Current workspace jobs with store-assigned c-ids in the actual room UUID.
fn many_jobs(count: usize) -> App {
    let mut app = squad_tab();
    app.cron = Default::default();
    let jobs = test_views(count, "product", ROOM);
    app.cron
        .replace(Ok(test_cron(jobs.clone(), ClockStatus::NoClock)));
    app.cron.replace(Ok(test_cron(jobs, ClockStatus::NoClock)));
    app
}

fn walk_ids(count: usize) -> Vec<String> {
    (1..=count).map(|id| format!("{ROOM}/c{id}")).collect()
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
            open_timers(&mut app);
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
    open_timers(&mut app);
    draw(&app, 160, 40);
    let first = app.cron_list.as_ref().unwrap().selected().unwrap();
    assert!(app.row_details.expanded.is_empty());
    app.cron.cron.as_mut().unwrap().actor = Err("actor unavailable".into());
    press(&mut app, KeyCode::Char('e'));
    assert!(app.input.is_none() && app.row_details.contains(&Target::Job(first.clone())));
    let shown = draw(&app, 160, 40).join("\n");
    assert!(shown.contains("no details yet"), "{shown}");
    press(&mut app, KeyCode::Down);
    draw(&app, 160, 40);
    let second = app.cron_list.as_ref().unwrap().selected().unwrap();
    assert_ne!(first, second);
    assert_eq!(app.row_details.expanded, vec![Target::Job(first.clone())]);
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
