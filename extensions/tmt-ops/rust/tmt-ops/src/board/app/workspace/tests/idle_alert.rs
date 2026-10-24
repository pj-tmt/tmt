use super::*;
use crate::board::{
    activity::{ObservedActivity, Phase},
    workspace::IdleAlertSetting,
};

const MINUTE: u64 = 60_000;

/// One squad whose only member has been idle for `idle_for`, with the alert
/// marking it `over` and the squad's setting `after` (None = never read).
fn idle_board(idle_for: u64, over: bool, setting: Option<IdleAlertSetting>) -> App {
    let mut app = board();
    app.workspace.filter = crate::board::workspace::AgentsFilter::All;
    let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let member = workspace.roster.members.get_mut(MEMBER).unwrap();
    member.state = Some("idle".into());
    member.activity = ObservedActivity {
        phase: Phase::Idle,
        since_ms: Some(crate::status::now_ms() - idle_for),
        last_ms: None,
    };
    let squad = &mut workspace.roster.squads[0];
    squad.idle_alert = setting;
    let state = squad.member_states.get_mut(MEMBER).unwrap();
    state.state = Some("idle".into());
    state.idle_over = over;
    app.overview_rows.borrow_mut().clear();
    app
}

fn alert(after_minutes: u64) -> Option<IdleAlertSetting> {
    Some(IdleAlertSetting {
        enabled: true,
        after_ms: after_minutes * MINUTE,
    })
}

fn screen(app: &App, width: u16) -> String {
    paint(app, width, 30).join("\n")
}

#[test]
fn an_over_member_shows_the_cron_glyph_before_the_idle_word_and_time() {
    let app = idle_board(32 * MINUTE, true, alert(30));
    let row = painted_member_row(&app, 160, 30);
    assert!(row.contains("◷ idle 32m"), "{row}");
    let plain = painted_member_row(&idle_board(32 * MINUTE, false, alert(30)), 160, 30);
    assert!(plain.contains("◌ idle 32m"), "{plain}");
    assert!(!plain.contains("◷ idle"), "{plain}");
}

#[test]
fn only_a_member_shown_as_idle_keeps_the_over_mark() {
    let mut app = idle_board(32 * MINUTE, true, alert(30));
    let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    workspace
        .roster
        .members
        .get_mut(MEMBER)
        .unwrap()
        .activity
        .phase = Phase::Working;
    app.overview_rows.borrow_mut().clear();
    let row = painted_member_row(&app, 160, 30);
    assert!(!row.contains("◷ idle"), "{row}");
}

#[test]
fn the_squad_row_names_the_alert_and_counts_the_members_over_only_while_it_is_on() {
    let on = screen(&idle_board(32 * MINUTE, true, alert(30)), 160);
    // The MODEL cell is 12 wide: the full words do not fit, the value stays.
    assert!(on.contains("◷ 30m"), "{on}");
    assert!(on.contains("◷ 1 idle"), "{on}");
    let excepted = screen(&idle_board(32 * MINUTE, false, alert(30)), 160);
    assert!(excepted.contains("◷ 30m"), "{excepted}");
    assert!(!excepted.contains("1 idle"), "{excepted}");
    for off in [
        None,
        Some(IdleAlertSetting {
            enabled: false,
            after_ms: 30 * MINUTE,
        }),
    ] {
        let screen = screen(&idle_board(32 * MINUTE, false, off), 160);
        assert_eq!(screen.contains("○ off"), off.is_some(), "{screen}");
        // The ◷ column caption stays; no row or count carries the idle mark.
        assert!(
            !screen.contains("◷ idle") && !screen.contains("1 idle"),
            "{screen}"
        );
    }
}

fn menu_labels(app: &App) -> Vec<String> {
    app.menu
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|entry| entry.label.clone())
        .collect()
}

fn settings_config(name: &str, text: &str) -> (std::path::PathBuf, Config) {
    let path = std::env::temp_dir().join(format!(
        "ops-idle-settings-{name}-{}.toml",
        std::process::id()
    ));
    std::fs::write(&path, text).unwrap();
    let config = Config::read(path.clone()).unwrap();
    (path, config)
}

#[test]
fn the_workspace_menu_ends_with_a_rule_and_the_selected_squads_settings_only_when_one_is_selected()
{
    let mut app = board();
    app.workspace_settings();
    let labels = menu_labels(&app);
    assert_eq!(labels.last().unwrap(), "Squad settings: product");
    assert!(labels[labels.len() - 2].chars().all(|ch| ch == '─'));
    assert!(matches!(
        app.menu.as_ref().unwrap().entries.last().unwrap().choice,
        Choice::SquadSettings(ref squad) if squad == "product"
    ));
    // Down saturates on the link; Up steps over the rule to the entry before it.
    paint(&app, 160, 36);
    for _ in 0..40 {
        key(&mut app, KeyCode::Down);
    }
    let menu = app.menu.as_ref().unwrap();
    assert_eq!(menu.entries[menu.selected].label, "Squad settings: product");
    key(&mut app, KeyCode::Up);
    let menu = app.menu.as_ref().unwrap();
    assert_eq!(menu.entries[menu.selected].label, "Panels");
    app.menu = None;
    app.workspace.selected = Some(OpsTarget::Pool);
    app.workspace_settings();
    let labels = menu_labels(&app);
    assert!(
        labels
            .iter()
            .all(|label| !label.starts_with("Squad settings") && !label.starts_with('─')),
        "{labels:?}"
    );
}

#[test]
fn choosing_squad_settings_opens_the_overlay_for_that_squad_and_escape_returns_to_the_menu() {
    let mut app = board();
    app.workspace_settings();
    let last = app.menu.as_ref().unwrap().entries.len() - 1;
    app.menu.as_mut().unwrap().selected = last;
    assert_eq!(
        key(&mut app, KeyCode::Enter),
        Effect::SquadSettings("product".into())
    );
    let (path, config) = settings_config("open", "[squad.product.idle_alert]\nenabled = true\n");
    app.open_settings_in(config, Some("product")).unwrap();
    let overlay = app.settings.as_ref().unwrap();
    assert!(overlay.from_menu);
    assert_eq!(overlay.settings.context.as_deref(), Some("product"));
    // Opened from the entry, the overlay is titled by the squad and shows the whole idle alert
    // group (both rows and the hint) with `enabled` selected, whatever the height.
    for (width, height) in [(120, 40), (100, 24), (80, 20)] {
        let mut fresh = board();
        let (fresh_path, fresh_config) =
            settings_config("reveal", "[squad.product.idle_alert]\nenabled = true\n");
        fresh
            .open_settings_in(fresh_config, Some("product"))
            .unwrap();
        let screen = paint(&fresh, width, height).join("\n");
        std::fs::remove_file(fresh_path).unwrap();
        let case = format!("{width}x{height}: {screen}");
        assert!(screen.contains("Squad settings: product"), "{case}");
        assert!(
            screen.contains("Tells the lead once when a member stays idle")
                && screen.contains("longer than after."),
            "{case}"
        );
        assert!(
            screen.lines().any(|line| line.contains('›')
                && line.contains("enabled")
                && line.contains("true")),
            "{case}"
        );
        assert!(
            screen
                .lines()
                .any(|line| line.contains("after") && line.contains("1800s")),
            "{case}"
        );
    }
    // A data refresh while it is open does not close it for the workspace's own tab.
    let mut refreshed = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
    refreshed.tabs.push(crate::board::ALL.into());
    refreshed.view = app.view.take().map(Ok).unwrap_or(refreshed.view);
    app.apply(refreshed);
    assert!(app.settings.is_some());
    assert_eq!(key(&mut app, KeyCode::Esc), Effect::CancelSettings);
    assert!(app.settings.is_none());
    assert_eq!(menu_labels(&app).last().unwrap(), "Squad settings: product");
    std::fs::remove_file(path).unwrap();
}

/// The oracle's idle-alert squad: a lead and a worker over the 30m threshold, one member excepted by
/// a pending decision, one resting on `hold`, and one still working.
pub(in crate::board) fn populate(home: &mut crate::board::home::Home) {
    use crate::board::workspace::{OpsMember, OpsMembershipState};
    let now = crate::status::now_ms();
    let people: [(&str, &str, Phase, u64, &str, bool, bool); 5] = [
        (
            "lead",
            "lead",
            Phase::Idle,
            35 * MINUTE,
            "idle",
            false,
            true,
        ),
        (
            "worker-2",
            "worker-2",
            Phase::Working,
            4 * MINUTE,
            "working",
            false,
            false,
        ),
        (
            "worker-3",
            "worker-3",
            Phase::Idle,
            32 * MINUTE,
            "review",
            false,
            true,
        ),
        (
            "worker-4",
            "worker-4",
            Phase::Idle,
            41 * MINUTE,
            "blocked",
            true,
            false,
        ),
        (
            "night-1",
            "night-1",
            Phase::Idle,
            9 * 60 * MINUTE,
            "hold",
            false,
            false,
        ),
    ];
    let workspace = &mut home.workspace;
    for (index, (_, name, phase, ago, state, waiting, over)) in people.into_iter().enumerate() {
        let id = format!("55555555-5555-4555-8555-{:012}", index + 1);
        workspace.roster.members.insert(
            id.clone(),
            OpsMember {
                id: id.clone(),
                name: name.into(),
                state: Some(state.into()),
                waiting,
                presence: Some(true),
                usage: None,
                activity: ObservedActivity {
                    phase,
                    since_ms: Some(now - ago),
                    last_ms: None,
                },
            },
        );
        let squad = &mut workspace.roster.squads[0];
        squad.members.push(id.clone());
        squad.member_states.insert(
            id.clone(),
            OpsMembershipState {
                state: Some(state.into()),
                waiting,
                rank: None,
                idle_over: over,
            },
        );
        if index == 0 {
            squad.lead = Some(id.clone());
        }
        workspace.rows.insert(
            OpsTarget::Member {
                room_id: Some(ROOM.into()),
                identity_id: id.clone(),
            },
            json!({"id":id,"name":name,"state":state,"presence":"active","pane":{"location":{"host":"tmux"}}}),
        );
    }
    workspace.roster.squads[0].idle_alert = alert(30);
    home.summary.members = 6;
    home.summary.working = 2;
    home.summary.idle = 4;
}

/// The scene where `Squad settings: product` was chosen from the menu and the idle rows are in view.
pub(in crate::board) fn open_squad_settings(app: &mut App) {
    let path = std::env::temp_dir().join(format!(
        "ops-idle-oracle-{}.toml",
        crate::id::new_v4().unwrap()
    ));
    std::fs::write(
        &path,
        "[squad.product.idle_alert]\nenabled = true\nafter = \"30m\"\n",
    )
    .unwrap();
    let config = Config::read(path.clone()).unwrap();
    std::fs::remove_file(&path).unwrap();
    app.open_settings_in(config, Some("product")).unwrap();
    app.settings
        .as_ref()
        .unwrap()
        .surface
        .borrow_mut()
        .select("idle_alert.enabled");
}

#[test]
fn disabled_alert_is_visible_and_direct_action_is_in_the_catalog() {
    let label = IdleAlertSetting {
        enabled: false,
        after_ms: 30 * MINUTE,
    }
    .label(30);
    assert_eq!(label.as_deref(), Some("○ off"));
    let action = crate::action::Action::parse("idle-alert");
    assert!(
        action.is_ok(),
        "idle-alert must be a bindable catalog action"
    );
}
