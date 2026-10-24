use super::*;

fn render(table: &Table, terminal: Terminal) -> String {
    if terminal.color {
        // Production anstream honors NO_COLOR before enabling color; these explicit
        // color fixtures override crossterm's cached choice.
        static COLOR: std::sync::Once = std::sync::Once::new();
        COLOR.call_once(|| crossterm::style::force_color_output(true));
    }
    let mut output = Vec::new();
    table.write(&mut output, terminal).unwrap();
    String::from_utf8(output).unwrap()
}

fn agents() -> Table {
    let mut table = Table::new(&[Column::Fixed, Column::Name, Column::Detail]);
    table
        .row(["●", "coordinator", "~/dev/tmt-local/very/long/path"])
        .row(["○", "peer", "~/dev/other"]);
    table
}

fn width(width: u16) -> Terminal {
    Terminal {
        color: false,
        width: Some(width),
        theme: None,
    }
}

#[test]
fn rows_are_indented_aligned_and_never_cut_without_a_known_width() {
    assert_eq!(
        render(&agents(), Terminal::PLAIN),
        "  ●  coordinator  ~/dev/tmt-local/very/long/path\n  ○  peer         ~/dev/other\n"
    );
    assert_eq!(render(&Table::new(&[Column::Name]), Terminal::PLAIN), "");
}

#[test]
fn details_truncate_before_names_and_rows_never_wrap() {
    assert_eq!(
        render(&agents(), width(36)),
        "  ●  coordinator  ~/dev/tmt-local/v…\n  ○  peer         ~/dev/other\n"
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
            theme: None,
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
        .row(["●", "astra", "~/dev/tmt-local"])
        .row_with_action(["○", "sol", "~/dev/tmt-local"], "↻ tmt resume sol");
    assert_eq!(
        render(&table, Terminal::PLAIN),
        "  ●  astra  ~/dev/tmt-local\n  ○  sol    ~/dev/tmt-local  ↻ tmt resume sol\n"
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
            theme: None,
        },
    );
    assert!(
        colored.contains("\u{1b}[34m↻ tmt resume sol"),
        "{colored:?}"
    );
}

#[test]
fn a_note_trails_its_row_dimmed_in_the_action_slot_and_is_never_truncated() {
    let mut table = Table::new(&[Column::Fixed, Column::Name, Column::Detail]);
    table
        .row(["●", "astra", "~/dev/tmt-local"])
        .row_with_note(["●", "pi-1", "~/dev/tmt-local"], "basic")
        .row_with_action(["○", "sol", "~/dev/tmt-local"], "↻ tmt resume sol");
    assert_eq!(
        render(&table, Terminal::PLAIN),
        "  ●  astra  ~/dev/tmt-local\n  ●  pi-1   ~/dev/tmt-local  basic\n  ○  sol    ~/dev/tmt-local  ↻ tmt resume sol\n"
    );
    // The detail gives way; the note keeps every character.
    assert_eq!(
        render(&table, width(36)),
        "  ●  astra  ~/dev…\n  ●  pi-1   ~/dev…  basic\n  ○  sol    ~/dev…  ↻ tmt resume sol\n"
    );
    let colored = render(
        &table,
        Terminal {
            color: true,
            width: None,
            theme: None,
        },
    );
    assert!(colored.contains("\u{1b}[2mbasic"), "{colored:?}");
    assert!(
        !colored.contains("\u{1b}[34mbasic"),
        "a note is not accent-colored: {colored:?}"
    );
}

#[test]
fn a_colored_first_column_keeps_its_gap_before_text_ending_in_m() {
    // `custom` ends in `m`, like a color reset: only a whole SGR sequence
    // may be taken for one, or the gap after `skill` disappears.
    let mut table = Table::new(&[Column::Fixed, Column::Detail]);
    table
        .row([
            Cell::styled("command", Token::Dim),
            "~/.local/bin/tmt".into(),
        ])
        .row([
            Cell::styled("skill", Token::Dim),
            "~/.agents/skills/custom".into(),
        ]);
    let colored = render(
        &table,
        Terminal {
            color: true,
            width: Some(120),
            theme: None,
        },
    );
    let plain: String = anstream::adapter::strip_str(&colored).to_string();
    assert_eq!(
        plain,
        "  command  ~/.local/bin/tmt\n  skill    ~/.agents/skills/custom\n"
    );
}

/// The layout before it moved onto the shared solver, kept as an oracle.
fn legacy_layout(columns: &[Column], natural: &[usize], available: Option<usize>) -> Vec<usize> {
    let mut widths = natural.to_vec();
    let Some(budget) = available else {
        return widths;
    };
    let total = |widths: &[usize]| widths.iter().sum::<usize>() + GAP * (widths.len() - 1);
    for kind in [Column::Detail, Column::Name] {
        while total(&widths) > budget {
            let widest = (0..widths.len())
                .filter(|&index| columns[index] == kind && widths[index] > MINIMUM)
                .max_by_key(|&index| (widths[index], std::cmp::Reverse(index)));
            let Some(index) = widest else { break };
            let excess = total(&widths) - budget;
            let next = (0..widths.len())
                .filter(|&other| other != index && columns[other] == kind)
                .map(|other| widths[other])
                .filter(|&width| width < widths[index])
                .max()
                .unwrap_or(0)
                .max(MINIMUM);
            widths[index] -= excess.min(widths[index] - next).max(1);
        }
    }
    widths
}

#[test]
fn the_shared_solver_lays_out_exactly_as_the_table_did() {
    let mut state = 512_u64;
    let mut next = |bound: usize| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        ((state >> 33) % bound as u64) as usize
    };
    for _ in 0..3_000 {
        let count = 1 + next(6);
        let columns: Vec<Column> = (0..count)
            .map(|_| [Column::Fixed, Column::Name, Column::Detail][next(3)])
            .collect();
        let natural: Vec<usize> = (0..count).map(|_| next(30)).collect();
        let mut table = Table::new(&columns).indent(0);
        table.row(natural.iter().map(|width| "x".repeat(*width)));
        let available = (next(4) > 0).then(|| next(100));
        assert_eq!(
            layout(&[&table], available.map(|width| width as u16)),
            legacy_layout(&columns, &natural, available),
            "{columns:?} {natural:?} at {available:?}"
        );
    }
}

/// A table's styled cells follow the theme; without one they keep the
/// 16-color rendering byte for byte.
#[test]
fn styled_cells_follow_the_theme() {
    use crate::{Base, Depth, Role, Theme};
    let mut table = Table::new(&[Column::Name, Column::Detail]);
    table.row([
        Cell::styled("ada", Token::Warn),
        Cell::styled("3m", Token::Dim),
    ]);
    let colored = |theme| Terminal {
        color: true,
        width: None,
        theme,
    };
    let truecolor = render(&table, colored(Some((Theme::default(), Depth::TrueColor))));
    assert!(
        truecolor.contains("\u{1b}[38;2;255;158;100m"),
        "{truecolor:?}"
    );
    assert!(
        truecolor.contains(
            &Theme::default()
                .style(Role::Dim, Depth::TrueColor)
                .render()
                .to_string()
        ),
        "dim is a color: {truecolor:?}"
    );
    let sixteen = render(
        &table,
        colored(Some((Theme::new(Base::Terminal), Depth::Ansi16))),
    );
    assert!(sixteen.contains("\u{1b}[33mada"), "{sixteen:?}");
    assert!(!sixteen.contains("38;5;"), "16 colors only: {sixteen:?}");
    let untouched = render(&table, colored(None));
    assert!(
        untouched.contains("\u{1b}[2m"),
        "no theme: dim stays an effect: {untouched:?}"
    );
    assert!(!untouched.contains("38;2"), "{untouched:?}");
}

/// One encoding for a token (#4542): a styled cell carries exactly the bytes
/// `Terminal::paint` writes for it, without a theme and at every depth, so a
/// mark in a table matches the same mark painted beside it. The color covers
/// the text only; the column's padding follows it, and a colored last column
/// ends with its text.
#[test]
fn a_styled_cell_is_painted_exactly_as_paint_writes_it() {
    use crate::{Base, Depth, Role, Theme};
    let tokens = [
        Token::Accent,
        Token::Ok,
        Token::Warn,
        Token::Error,
        Token::Dim,
        Token::Title,
        Token::Literal,
        Token::Driver(None),
        Token::Driver(Some(Role::Review)),
    ];
    let themes = [
        None,
        Some((Theme::new(Base::Terminal), Depth::Ansi16)),
        Some((Theme::default(), Depth::Ansi16)),
        Some((Theme::default(), Depth::TrueColor)),
    ];
    for theme in themes {
        let terminal = Terminal {
            color: true,
            width: None,
            theme,
        };
        for token in tokens {
            let mut table = Table::new(&[Column::Fixed, Column::Name]).indent(0);
            table.row([Cell::styled("✓", token), "name".into()]);
            assert_eq!(
                render(&table, terminal),
                format!("{}  name\n", terminal.paint(token, "✓")),
                "{token:?} under {theme:?}"
            );
            let mut table = Table::new(&[Column::Name, Column::Fixed]).indent(0);
            table
                .row(["a".into(), Cell::styled("✓", token)])
                .row(["b".into(), Cell::styled("✓✓", token)]);
            assert_eq!(
                render(&table, terminal),
                format!(
                    "a  {}\nb  {}\n",
                    terminal.paint(token, "✓"),
                    terminal.paint(token, "✓✓")
                ),
                "{token:?} under {theme:?}"
            );
        }
    }
}

/// Reverse video shows where a style ends: it covers the cell's text, the
/// column padding follows the reset, and the next cell starts unstyled.
#[test]
fn a_reversed_cell_covers_its_text_and_not_its_padding() {
    use crate::{Depth, Theme};
    let theme = Some((
        Theme::parse("theme", [("working", "reverse")]).unwrap(),
        Depth::TrueColor,
    ));
    assert!(
        Token::Ok
            .themed(theme)
            .get_effects()
            .contains(anstyle::Effects::INVERT),
        "the control must reverse the token"
    );
    let terminal = Terminal {
        color: true,
        width: None,
        theme,
    };
    let mut table = Table::new(&[Column::Name, Column::Name]).indent(0);
    table
        .row([Cell::styled("x", Token::Ok), "after".into()])
        .row([Cell::styled("long", Token::Ok), "after".into()]);
    assert_eq!(
        render(&table, terminal),
        format!(
            "{}     after\n{}  after\n",
            terminal.paint(Token::Ok, "x"),
            terminal.paint(Token::Ok, "long")
        )
    );
}

/// A cut keeps grapheme clusters whole: a modified or joined emoji is never
/// split, plain or styled, and wide and combining text cut as before.
#[test]
fn a_cut_keeps_each_grapheme_whole() {
    let colored = Terminal {
        color: true,
        width: Some(4),
        theme: None,
    };
    for (text, cut) in [
        ("👩🏽\u{200d}💻abcde", "👩🏽\u{200d}💻a…"),
        (
            "👨\u{200d}👩\u{200d}👧\u{200d}👦abcde",
            "👨\u{200d}👩\u{200d}👧\u{200d}👦a…",
        ),
        ("🇹🇼abcde", "🇹🇼a…"),
        ("e\u{301}abcde", "e\u{301}ab…"),
        ("文件abcde", "文…"),
    ] {
        let mut table = Table::new(&[Column::Name]).indent(0);
        table.row([text]);
        assert_eq!(render(&table, width(4)), format!("{cut}\n"), "{text:?}");
        let mut table = Table::new(&[Column::Name]).indent(0);
        table.row([Cell::styled(text, Token::Ok)]);
        assert_eq!(
            render(&table, colored),
            format!("{}\n", colored.paint(Token::Ok, cut)),
            "{text:?}"
        );
    }
}
