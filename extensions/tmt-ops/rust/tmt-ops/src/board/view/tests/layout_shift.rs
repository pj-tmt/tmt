//! A refresh moves no cell: the same row keeps its column offsets whether its
//! values are placeholders, carried from the previous read, or fresh.
use super::*;
use crate::board::app::tests::snapshot;
use tmt_cli_style::Role;

const NOW: u64 = 10_000_000;

/// Where one row's cells sit on its screen line, in character columns.
#[derive(Debug, PartialEq)]
struct Offsets {
    name: usize,
    state: usize,
    model: Option<usize>,
    /// The column after the last visible character: where the right-aligned age ends.
    age_end: usize,
}

fn member(id: &str, model: Option<&str>, changed_ms_ago: Option<u64>) -> Value {
    let mut row = json!({
        "id": id, "name": id, "state": "working", "presence": "active", "pending": null,
        "colors": {"state": "default"},
        "fields": {"state": "working", "task": "write the report"},
        "staleness": {"state": "unknown"},
    });
    if let Some(model) = model {
        row["fields"]["model"] = model.into();
    }
    if let Some(ago) = changed_ms_ago {
        row["staleness"] = json!({"state": "fresh", "unchangedSinceMs": NOW - ago, "ageMs": ago});
    }
    row
}

fn members_view(app: &mut App, rows: Vec<Value>) {
    app.apply(snapshot("product", json!([{"title": null, "rows": rows}])));
    app.view.as_mut().unwrap().board.members = true;
    // The selection restyles a row, so the row under test stays unselected.
    app.selected = 1;
}

fn offsets(app: &App, width: u16, name: &str, model: &str) -> Offsets {
    let screen = crate::status::with_now_ms(NOW, || draw(app, width, 20));
    let line = screen
        .iter()
        .find(|line| line.contains(name))
        .unwrap_or_else(|| panic!("{name} in {screen:#?}"));
    let chars: Vec<char> = line.trim_end().chars().collect();
    let at = |text: &str| {
        let needle: Vec<char> = text.chars().collect();
        chars.windows(needle.len()).position(|w| w == needle)
    };
    Offsets {
        name: at(name).unwrap(),
        state: at("working").unwrap(),
        model: at(model),
        age_end: chars.len(),
    }
}

fn foreground(app: &App, width: u16, name: &str, text: &str) -> ratatui::style::Color {
    let screen = crate::status::with_now_ms(NOW, || draw(app, width, 20));
    let y = screen.iter().position(|line| line.contains(name)).unwrap();
    let x = screen[y][..screen[y].find(text).unwrap()].chars().count();
    let buffer = crate::status::with_now_ms(NOW, || board_buffer(app, width, 20));
    buffer[(x as u16, y as u16)].fg
}

#[test]
fn a_refresh_moves_no_member_cell_between_placeholder_carried_and_fresh_values() {
    for width in [60, 100, 160] {
        let mut app = App::new(Some("product".into()));
        // `bravo` keeps its values; `alpha` is the row whose reads come and go.
        let rows = |model: Option<&str>, ago: Option<u64>| {
            vec![
                member("alpha", model, ago),
                member("bravo", Some("gpt-6.1-sol"), Some(1_680_000)),
            ]
        };
        let both = |app: &App| {
            (
                offsets(app, width, "alpha", "sol"),
                offsets(app, width, "bravo", "sol"),
            )
        };
        // Placeholder: alpha has been read for neither a model nor an age.
        members_view(&mut app, rows(None, None));
        let (placeholder, bravo) = both(&app);
        assert_eq!(placeholder.model, None);
        // Fresh: the values arrive.
        members_view(&mut app, rows(Some("gpt-6.1-sol"), Some(420_000)));
        let (fresh, fresh_bravo) = both(&app);
        let screen = crate::status::with_now_ms(NOW, || draw(&app, width, 20)).join("\n");
        assert!(screen.contains("7m ago"), "the age is shown: {screen}");
        // Carried: the next refresh has not read them yet.
        members_view(&mut app, rows(None, None));
        let (carried, carried_bravo) = both(&app);
        let screen = crate::status::with_now_ms(NOW, || draw(&app, width, 20)).join("\n");
        assert!(screen.contains("7m ago"), "the age is carried: {screen}");
        assert_eq!(carried, fresh, "{width}: carried values keep every cell");
        assert_eq!(
            (placeholder.name, placeholder.state, placeholder.age_end),
            (fresh.name, fresh.state, fresh.age_end),
            "{width}: a placeholder keeps the name, state and age cells"
        );
        assert_eq!(
            fresh.model, bravo.model,
            "{width}: the model cell is shared"
        );
        assert_eq!(
            (fresh_bravo, carried_bravo),
            (bravo, offsets(&app, width, "bravo", "sol"))
        );
        // The carried model is dim; the fresh one is muted.
        let look = app.look();
        let role = |role: Role| look.role(role).fg.unwrap_or_default();
        assert_eq!(foreground(&app, width, "alpha", "sol"), role(Role::Dim));
        members_view(&mut app, rows(Some("gpt-6.1-sol"), Some(420_000)));
        assert_eq!(foreground(&app, width, "alpha", "sol"), role(Role::Muted));
    }
}

#[test]
fn member_cells_grow_with_longer_values_and_never_shrink_until_a_resize() {
    let mut app = App::new(Some("product".into()));
    let rows = |model: &str| vec![member("alpha", Some(model), Some(60_000))];
    members_view(&mut app, rows("sol"));
    let short = offsets(&app, 120, "alpha", "sol");
    members_view(&mut app, rows("sonnet"));
    let long = offsets(&app, 120, "alpha", "sonnet");
    assert_eq!(
        (long.name, long.state, long.age_end),
        (short.name, short.state, short.age_end)
    );
    assert_eq!(long.model, Some(short.model.unwrap() - 3), "the cell grew");
    members_view(&mut app, rows("sol"));
    assert_eq!(
        offsets(&app, 120, "alpha", "sol").model,
        long.model,
        "it kept its width"
    );
    offsets(&app, 100, "alpha", "sol");
    assert_eq!(
        offsets(&app, 120, "alpha", "sol").model,
        short.model,
        "a resize derives the widths again from what is loaded"
    );
}

fn grid_offsets(app: &App, width: u16, task: &str) -> (usize, usize) {
    let screen = draw(app, width, 20);
    let line = screen
        .iter()
        .find(|line| line.contains("alpha"))
        .unwrap_or_else(|| panic!("alpha in {screen:#?}"));
    // Character columns: a placeholder `–` is three bytes in UTF-8.
    let column = |text: &str| {
        let at = line
            .find(text)
            .unwrap_or_else(|| panic!("{text} in {line:?}"));
        line[..at].chars().count()
    };
    (column("alpha"), column(task))
}

#[test]
fn grid_columns_keep_their_widths_while_values_come_and_go() {
    for columns in [
        "{name = 'branch', from = 'meta.branch'}",
        "{name = 'branch', from = 'meta.branch', max = 24}",
    ] {
        for width in [80, 120] {
            let mut app = App::new(Some("product".into()));
            let rows = rows_from(&format!(
                "[p.rows]\ncolumns = [{{name = 'member', width = 10}}, {columns}, {{name = 'task', min = 10, grow = 1}}]\n"
            ));
            let show = |app: &mut App, branch: Option<&str>| {
                let mut alpha = member("alpha", None, None);
                if let Some(branch) = branch {
                    alpha["fields"]["branch"] = branch.into();
                }
                app.apply(snapshot(
                    "product",
                    json!([{"title": null, "rows": [alpha]}]),
                ));
                app.view.as_mut().unwrap().rows = rows.clone();
                grid_offsets(app, width, "write the report")
            };
            let filled = show(&mut app, Some("feature/layout-shift"));
            let placeholder = show(&mut app, None);
            assert_eq!(
                placeholder, filled,
                "{width}: a missing value keeps the column"
            );
            let refilled = show(&mut app, Some("feature/layout-shift"));
            assert_eq!(refilled, filled);
            // A longer value grows the column once and a shorter one does not shrink it.
            let longer = show(&mut app, Some("feature/layout-shift-and-more"));
            assert!(
                longer.1 > filled.1,
                "{width}: the column grew: {longer:?} {filled:?}"
            );
            assert_eq!(show(&mut app, Some("main")), longer);
            assert_eq!(show(&mut app, None), longer);
        }
    }
}
