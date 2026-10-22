//! Markdown tables in a chat message paint as grids at every pane width (#4456).
use super::*;

const WIDE: &str = "Status of the squads:\n\n\
| Name | Role | Notes |\n|---|---|---|\n\
| ux-2 | tui | renders a table in the pane |\n\
| core | arch | wide column with a long description that goes on and on past the edge |";

fn frame(body: &str, width: u16) -> String {
    let mut app = fixture();
    app.view.as_mut().unwrap().replies[0]["response"] = json!(body);
    draw(&app, width, 40)
}

#[test]
fn a_table_wider_than_the_pane_paints_as_a_grid_not_raw_pipes() {
    for width in [80, 120] {
        let screen = frame(WIDE, width);
        assert!(
            screen.contains('┌') && screen.contains('└'),
            "{width}: {screen}"
        );
        assert!(!screen.contains("|---"), "{width}: {screen}");
        assert!(!screen.contains("| Name |"), "{width}: {screen}");
        for cell in ["Name", "Role", "Notes", "ux-2", "core", "arch", "edge"] {
            assert!(screen.contains(cell), "{width}: {cell} missing\n{screen}");
        }
    }
}

#[test]
fn a_table_in_a_quote_keeps_its_bar_and_its_grid() {
    let quoted = WIDE
        .lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    for width in [80, 120] {
        let screen = frame(&quoted, width);
        assert!(
            screen.contains('┌') && screen.contains('└'),
            "{width}: {screen}"
        );
        assert!(!screen.contains("|---"), "{width}: {screen}");
    }
}
