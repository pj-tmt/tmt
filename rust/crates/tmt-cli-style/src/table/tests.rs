use super::*;

fn render(table: &Table, terminal: Terminal) -> String {
    let mut output = Vec::new();
    table.write(&mut output, terminal).unwrap();
    String::from_utf8(output).unwrap()
}

fn agents() -> Table {
    let mut table = Table::new(&[Column::Fixed, Column::Name, Column::Detail]);
    table
        .row(["●", "coordinator", "~/dev/tmux-team/very/long/path"])
        .row(["○", "peer", "~/dev/other"]);
    table
}

fn width(width: u16) -> Terminal {
    Terminal {
        color: false,
        width: Some(width),
    }
}

#[test]
fn rows_are_indented_aligned_and_never_cut_without_a_known_width() {
    assert_eq!(
        render(&agents(), Terminal::PLAIN),
        "  ●  coordinator  ~/dev/tmux-team/very/long/path\n  ○  peer         ~/dev/other\n"
    );
    assert_eq!(render(&Table::new(&[Column::Name]), Terminal::PLAIN), "");
}

#[test]
fn details_truncate_before_names_and_rows_never_wrap() {
    assert_eq!(
        render(&agents(), width(36)),
        "  ●  coordinator  ~/dev/tmux-team/v…\n  ○  peer         ~/dev/other\n"
    );
    // Once the detail is at its minimum, the name gives way too.
    assert_eq!(
        render(&agents(), width(20)),
        "  ●  coordina…  ~/d…\n  ○  peer       ~/d…\n"
    );
    for columns in [12, 20, 36] {
        for line in render(&agents(), width(columns)).lines() {
            // Below the columns' minimums (15 here) every column stays at its minimum.
            assert!(line.width() <= usize::from(columns).max(15), "{line}");
        }
    }
}

#[test]
fn control_characters_are_escaped_before_measuring() {
    let mut table = Table::new(&[Column::Name, Column::Detail]).indent(0);
    table
        .row(["a\tb", "\u{1b}[31m\n\u{2028}"])
        .row(["x", r"\n"]);
    assert_eq!(
        render(&table, Terminal::PLAIN),
        "a\\tb  \\u{1b}[31m\\n\\u{2028}\nx     \\n\n"
    );
}

#[test]
fn color_is_applied_after_layout() {
    let mut table = Table::new(&[Column::Fixed, Column::Name]);
    table
        .row([Cell::styled("●", Token::Ok), "long-name".into()])
        .row([Cell::styled("○", Token::Dim), "b".into()]);
    let colored = render(
        &table,
        Terminal {
            color: true,
            width: None,
        },
    );
    assert!(colored.contains("\u{1b}["), "{colored:?}");
    let plain: String = anstream::adapter::strip_str(&colored).to_string();
    assert_eq!(plain, render(&table, Terminal::PLAIN));
}

#[test]
#[should_panic(expected = "one cell per column")]
fn a_row_needs_one_cell_per_column() {
    Table::new(&[Column::Name, Column::Name]).row(["only"]);
}

#[test]
fn actions_trail_only_their_rows_and_are_never_truncated() {
    let mut table = Table::new(&[Column::Fixed, Column::Name, Column::Detail]);
    table
        .row(["●", "astra", "~/dev/tmux-team"])
        .row_with_action(["○", "sol", "~/dev/tmux-team"], "↻ tmt resume sol");
    assert_eq!(
        render(&table, Terminal::PLAIN),
        "  ●  astra  ~/dev/tmux-team\n  ○  sol    ~/dev/tmux-team  ↻ tmt resume sol\n"
    );
    // The detail gives way; the action keeps every character.
    assert_eq!(
        render(&table, width(36)),
        "  ●  astra  ~/dev…\n  ○  sol    ~/dev…  ↻ tmt resume sol\n"
    );
    let colored = render(
        &table,
        Terminal {
            color: true,
            width: None,
        },
    );
    assert!(
        colored.contains("\u{1b}[38;5;4m↻ tmt resume sol"),
        "{colored:?}"
    );
}

#[test]
fn colored_rows_end_without_padding() {
    assert_eq!(
        without_trailing_padding("a  \u{1b}[34mshell   \u{1b}[39m\u{1b}[0m  "),
        "a  \u{1b}[34mshell\u{1b}[39m\u{1b}[0m"
    );
    assert_eq!(without_trailing_padding("plain   "), "plain");
}

#[test]
fn a_colored_first_column_keeps_its_gap_before_text_ending_in_m() {
    // `tmux-team` ends in `m`, like a color reset: only a whole SGR sequence
    // may be taken for one, or the gap after `skill` disappears.
    let mut table = Table::new(&[Column::Fixed, Column::Detail]);
    table
        .row([
            Cell::styled("command", Token::Dim),
            "~/.local/bin/tmt".into(),
        ])
        .row([
            Cell::styled("skill", Token::Dim),
            "~/.agents/skills/tmux-team".into(),
        ]);
    let colored = render(
        &table,
        Terminal {
            color: true,
            width: Some(120),
        },
    );
    let plain: String = anstream::adapter::strip_str(&colored).to_string();
    assert_eq!(
        plain,
        "  command  ~/.local/bin/tmt\n  skill    ~/.agents/skills/tmux-team\n"
    );
    assert_eq!(
        without_trailing_padding("\u{1b}[2mskill  \u{1b}[0m  ~/tmux-team  "),
        "\u{1b}[2mskill  \u{1b}[0m  ~/tmux-team"
    );
}
