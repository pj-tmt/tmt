//! Representative human output in each terminal mode. Snapshots are reviewed
//! with the change that alters them; escapes are shown as `\u{1b}`.

use std::path::Path;
use tmt_cli_style::{
    CommandSpec, Example, HelpSection, OutputModes, Terminal, Token, command,
    command_with_sections, help_text,
    list::{self, Section},
    mark::Mark,
    message,
    table::{Cell, Column, Table},
    value,
};

const TTY: Terminal = Terminal {
    color: true,
    width: Some(80),
    theme: None,
};
const NARROW: Terminal = Terminal {
    color: true,
    width: Some(40),
    theme: None,
};
const PIPE: Terminal = Terminal::PLAIN;

fn visible(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes)
        .unwrap()
        .replace('\u{1b}', "\\u{1b}")
}

/// The CLI maps each driver descriptor's hue; these fixture drivers stand in.
fn hue(driver: &str) -> Token {
    Token::Driver(match driver {
        "claude" => Some(anstyle::AnsiColor::Magenta),
        "codex" => Some(anstyle::AnsiColor::Cyan),
        _ => None,
    })
}

fn agents() -> Vec<Section<'static>> {
    let home = Path::new("/Users/ada");
    let cells = |mark: Mark, name: &str, driver: &str, id: &str, path: &str| {
        [
            Cell::styled(mark.symbol(), mark.token()),
            name.into(),
            Cell::styled(value::address(driver, id), hue(driver)),
            value::home_path(Path::new(path), Some(home)).into(),
        ]
    };
    let columns = [Column::Fixed, Column::Name, Column::Fixed, Column::Detail];
    let mut saved = Table::new(&columns);
    saved
        .row(cells(
            Mark::Running,
            "astra",
            "codex",
            "019a2f4c-1111-2222-3333-444455556666",
            "/Users/ada/dev/tmux-team",
        ))
        .row(cells(
            Mark::Running,
            "opus-tmt-peer-2",
            "claude",
            "7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f",
            "/Users/ada/dev/tmux-team/worktrees/feature-branch",
        ))
        .row_with_action(
            cells(
                Mark::Offline,
                "sol",
                "claude",
                "3f9a1c07-aaaa-bbbb-cccc-dddd0000ffff",
                "/Users/ada/dev/tmux-team",
            ),
            &format!("{} tmt resume sol", Mark::Resumable.symbol()),
        );
    let mut temporary = Table::new(&columns);
    temporary
        .row(cells(
            Mark::Running,
            "mamezu-astra",
            "codex",
            "01a9c3b8-0000-1111-2222-333344445555",
            "/Users/ada/dev/mosaic-art",
        ))
        .row_with_action(
            cells(Mark::Idle, "opus-1", "tmux", "%31", "/srv/builds/nightly"),
            "shell",
        );
    vec![
        Section {
            title: "saved",
            count: Some(3),
            rows: saved,
            note: None,
            hint: None,
        },
        Section {
            title: "temporary",
            count: Some(2),
            rows: temporary,
            note: Some("offline: gemini-helper · night-owl"),
            hint: Some("tmt ls --all shows offline identities"),
        },
    ]
}

fn render_list(terminal: Terminal) -> String {
    if terminal.color {
        // Production anstream honors NO_COLOR before enabling color; these explicit
        // color fixtures override crossterm's cached choice.
        static COLOR: std::sync::Once = std::sync::Once::new();
        COLOR.call_once(|| crossterm::style::force_color_output(true));
    }
    let mut output = Vec::new();
    list::write(&mut output, terminal, &agents()).unwrap();
    visible(output)
}

#[test]
fn list_on_a_terminal() {
    insta::assert_snapshot!(render_list(TTY));
}

#[test]
fn list_on_a_narrow_terminal() {
    insta::assert_snapshot!(render_list(NARROW));
}

#[test]
fn list_through_a_pipe() {
    insta::assert_snapshot!(render_list(PIPE));
}

fn messages(terminal: Terminal) -> String {
    let mut output = Vec::new();
    message::success(&mut output, terminal, "Named pane %3 worker.").unwrap();
    message::error(
        &mut output,
        terminal,
        "Identity 'nobody' was not found.",
        Some("tmt ls"),
    )
    .unwrap();
    visible(output)
}

#[test]
fn messages_on_a_terminal() {
    insta::assert_snapshot!(messages(TTY));
}

#[test]
fn messages_through_a_pipe() {
    insta::assert_snapshot!(messages(PIPE));
}

fn talk() -> clap::Command {
    const EXAMPLES: &[Example] = &[
        Example {
            command: "tmt talk worker \"Run the tests\"",
            note: "Send a message to one agent",
        },
        Example {
            command: "tmt talk worker \"Summarize\" --wait",
            note: "Wait for the reply",
        },
    ];
    command(&CommandSpec {
        name: "talk",
        summary: "Send a message to an agent",
        examples: EXAMPLES,
        outputs: OutputModes::HumanAndJson,
        details: "",
    })
    .bin_name("tmt talk")
    .arg(clap::Arg::new("target").required(true).help("Agent name"))
    .arg(
        clap::Arg::new("message")
            .required(true)
            .help("Message text"),
    )
    .arg(
        clap::Arg::new("wait")
            .long("wait")
            .action(clap::ArgAction::SetTrue)
            .help("Wait for the reply"),
    )
}

#[test]
fn help_on_a_terminal() {
    insta::assert_snapshot!(visible(help_text(&talk(), TTY).into_bytes()));
}

#[test]
fn help_through_a_pipe() {
    insta::assert_snapshot!(help_text(&talk(), PIPE));
}

#[test]
fn root_help_renders_discovered_sections_in_the_template() {
    const EXAMPLES: &[Example] = &[Example {
        command: "tmt ls",
        note: "See every agent",
    }];
    let root = command_with_sections(
        &CommandSpec {
            name: "tmt",
            summary: "Collaborate with agents through durable tmux exchanges",
            examples: EXAMPLES,
            outputs: OutputModes::HumanAndJson,
            details: "",
        },
        &[HelpSection {
            title: "Extensions".into(),
            entries: vec![
                ("squad (also: sq)".into(), "Lead a squad of agents".into()),
                ("office".into(), "Run the local Office".into()),
            ],
        }],
    )
    .subcommand(clap::Command::new("ls").about("List agents"));
    insta::assert_snapshot!(help_text(&root, PIPE));
}
