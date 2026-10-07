//! Thin configuration command composition and presentation. Policy and raw
//! document mutation remain in the shared core and filesystem adapter.

use crate::invocation::{ConfigRequest, OutputMode};
use crate::output::Failure;
use serde_json::json;
use std::io::{self, Write};
use tmt_adapters::config::{ConfigFiles, ConfigPaths, Scope, ThemeProblem};
use tmt_core::settings::{EDITABLE_KEYS, LocalClear, ResolvedSettings, Setting, SettingKey};

fn invalid_setting(message: String) -> Failure {
    Failure::new("ERROR", message, 1)
}

fn cli_theme_bases() -> String {
    tmt_cli_style::Base::ALL
        .into_iter()
        .filter(|base| *base != tmt_cli_style::Base::Auto)
        .map(tmt_cli_style::Base::name)
        .collect::<Vec<_>>()
        .join(", ")
}

fn editable_theme_base(value: &str) -> Result<tmt_cli_style::Base, Failure> {
    tmt_cli_style::Base::parse(value)
        .filter(|base| *base != tmt_cli_style::Base::Auto)
        .ok_or_else(|| {
            invalid_setting(format!("Invalid value for theme.base: {value}."))
                .suggestion(format!("Valid bases: {}", cli_theme_bases()))
        })
}

struct Shown {
    loaded: ResolvedSettings,
    theme: Vec<(String, String)>,
    /// A bad theme is reported, never a failure: it must not stop
    /// `config show`, which Squad reads to find its own file.
    theme_error: Option<ThemeProblem>,
    paths: ConfigPaths,
}

enum Report {
    Show(Box<Shown>),
    Changed(String),
}

fn run(request: ConfigRequest) -> Result<Report, Failure> {
    let files = ConfigFiles {
        paths: ConfigPaths::discover()?,
    };
    match request {
        ConfigRequest::Show => {
            let loaded = files.load()?;
            let (theme, theme_error) = match files.theme()? {
                Ok(theme) => {
                    // The meaning is checked here, so a bad value names its key.
                    let error = crate::appearance::parse(&theme)
                        .err()
                        .map(|error| ThemeProblem {
                            key: error.key,
                            message: error.message,
                        });
                    (theme, error)
                }
                Err(problem) => (Vec::new(), Some(problem)),
            };
            Ok(Report::Show(Box::new(Shown {
                loaded,
                theme,
                theme_error,
                paths: files.paths,
            })))
        }
        ConfigRequest::Set { key, value, global } => {
            let scope = if global { Scope::Global } else { Scope::Local };
            if key == "theme.base" {
                if !global {
                    return Err(invalid_setting(
                        "theme.base can only be set in global config with --global.".into(),
                    ));
                }
                let base = editable_theme_base(&value)?;
                files.set_theme_base(base.name())?;
            } else {
                files.set(
                    Setting::edit(&key, &value, scope).map_err(invalid_setting)?,
                    scope,
                )?;
            }
            let destination = if global {
                "global config"
            } else {
                "local config (repo override)"
            };
            Ok(Report::Changed(format!(
                "Set {key}={value} in {destination}"
            )))
        }
        ConfigRequest::Clear { key } => {
            files.clear_local(LocalClear::parse(key.as_deref()).map_err(invalid_setting)?)?;
            Ok(Report::Changed(key.map_or_else(
                || "Cleared all local config overrides".into(),
                |key| format!("Cleared local override for {key}"),
            )))
        }
    }
}

pub fn execute(request: ConfigRequest, mode: OutputMode) -> io::Result<u8> {
    let report = match run(request) {
        Ok(report) => report,
        Err(error) => return error.publish(mode),
    };
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    let terminal = output.terminal();
    match report {
        Report::Changed(message) => {
            if mode.json {
                writeln!(output, "{{\"ok\":true}}")?;
            } else {
                tmt_cli_style::message::success(&mut output, terminal, &message)?;
            }
        }
        Report::Show(shown) => {
            let Shown {
                loaded,
                theme,
                theme_error,
                paths,
            } = *shown;
            if mode.json {
                let mut document = show_json(&loaded, &theme, &paths);
                document["themeError"] = theme_error.as_ref().map_or(
                    serde_json::Value::Null,
                    |problem| json!({"key": problem.key, "message": problem.message}),
                );
                writeln!(output, "{document}")?;
            } else {
                show_text(&mut output, terminal, &loaded, &theme, &paths)?;
                if let Some(problem) = theme_error {
                    let mut stderr = tmt_cli_style::stream::stderr();
                    let terminal = stderr.terminal();
                    write_theme_problem(&mut stderr, terminal, &theme, &problem)?;
                }
            }
        }
    }
    Ok(0)
}

fn write_theme_problem(
    output: &mut impl Write,
    terminal: tmt_cli_style::Terminal,
    theme: &[(String, String)],
    problem: &ThemeProblem,
) -> io::Result<()> {
    if problem.key == "theme.base"
        && theme
            .iter()
            .any(|(key, value)| key == "base" && value == "auto")
    {
        return tmt_cli_style::message::error(
            output,
            terminal,
            "theme.base auto only works for the board, not in config.json",
            Some("tmt sq theme set auto"),
        );
    }
    tmt_cli_style::message::error(
        output,
        terminal,
        &format!(
            "{} {}; commands use the terminal's colors until it is fixed",
            problem.key,
            problem.message.trim_end_matches('.')
        ),
        None,
    )
}

/// `theme` is the global file's settings as written (already checked); an
/// empty one is the default: the terminal's own colors on the command line.
fn show_json(
    loaded: &ResolvedSettings,
    theme: &[(String, String)],
    paths: &ConfigPaths,
) -> serde_json::Value {
    let settings = &loaded.settings;
    let theme_source = if theme.is_empty() {
        "default"
    } else {
        "global"
    };
    let theme: serde_json::Map<String, serde_json::Value> = theme
        .iter()
        .map(|(key, value)| (key.clone(), json!(value)))
        .collect();
    json!({
        "resolved": {
            "preambleMode": settings.preamble_mode.as_str(),
            "preambleEvery": settings.preamble_every,
            "pasteEnterDelayMs": settings.paste_enter_delay_ms,
            "defaults": {
                "timeout": settings.timeout,
                "pollInterval": settings.poll_interval,
                "captureLines": settings.capture_lines,
                "preambleEvery": settings.preamble_every,
                "pasteEnterDelayMs": settings.paste_enter_delay_ms,
            },
            "exchange": { "retentionDays": settings.retention_days },
            "notes": { "compactionReminder": settings.notes_compaction_reminder },
            "notifications": { "replyBatchWindowMs": settings.reply_batch_window_ms, "typingQuietMs": settings.typing_quiet_ms },
            "ui": { "paneBadge": settings.pane_badge.as_str() },
            "theme": theme,
        },
        "sources": {
            "preambleMode": loaded.source(SettingKey::PreambleMode),
            "preambleEvery": loaded.source(SettingKey::PreambleEvery),
            "pasteEnterDelayMs": loaded.source(SettingKey::PasteEnterDelayMs),
            "exchange": { "retentionDays": loaded.source(SettingKey::RetentionDays) },
            "notes": { "compactionReminder": loaded.source(SettingKey::NotesCompactionReminder) },
            "notifications": { "replyBatchWindowMs": loaded.source(SettingKey::ReplyBatchWindowMs), "typingQuietMs": loaded.source(SettingKey::TypingQuietMs) },
            "ui": { "paneBadge": loaded.source(SettingKey::PaneBadge) },
            "theme": theme_source,
        },
        "paths": { "global": paths.global_config, "local": paths.local_config },
    })
}

fn show_text(
    output: &mut impl Write,
    terminal: tmt_cli_style::Terminal,
    loaded: &ResolvedSettings,
    theme: &[(String, String)],
    paths: &ConfigPaths,
) -> io::Result<()> {
    let settings = &loaded.settings;
    let rows = [
        (
            SettingKey::NotesCompactionReminder,
            "notes.compactionReminder",
            settings.notes_compaction_reminder.to_string(),
        ),
        (
            SettingKey::ReplyBatchWindowMs,
            "notifications.replyBatchWindowMs",
            settings.reply_batch_window_ms.to_string(),
        ),
        (
            SettingKey::TypingQuietMs,
            "notifications.typingQuietMs",
            settings.typing_quiet_ms.to_string(),
        ),
        (
            SettingKey::PreambleMode,
            "preambleMode",
            settings.preamble_mode.as_str().to_string(),
        ),
        (
            SettingKey::PreambleEvery,
            "preambleEvery",
            settings.preamble_every.to_string(),
        ),
        (
            SettingKey::PasteEnterDelayMs,
            "pasteEnterDelayMs",
            settings.paste_enter_delay_ms.to_string(),
        ),
        (
            SettingKey::Timeout,
            "defaults.timeout",
            settings.timeout.to_string(),
        ),
        (
            SettingKey::PollInterval,
            "defaults.pollInterval",
            settings.poll_interval.to_string(),
        ),
        (
            SettingKey::CaptureLines,
            "defaults.captureLines",
            settings.capture_lines.to_string(),
        ),
        (
            SettingKey::RetentionDays,
            "exchange.retentionDays",
            settings.retention_days.to_string(),
        ),
        (
            SettingKey::PaneBadge,
            "ui.paneBadge",
            settings.pane_badge.as_str().to_string(),
        ),
    ];
    use tmt_cli_style::{
        Token,
        list::{self, Section},
        table::{Cell, Column, Table},
        value,
    };
    let mut table = Table::new(&[
        Column::Name,
        Column::Fixed,
        Column::Fixed,
        Column::Fixed,
        Column::Detail,
    ]);
    for (key, name, value) in rows {
        let changes = if !EDITABLE_KEYS.contains(&key) {
            "global file only"
        } else if key.global_only() {
            "global CLI"
        } else {
            "local/global CLI"
        };
        table.row([
            Cell::from(name),
            Cell::from(value),
            Cell::styled(loaded.source(key), Token::Dim),
            Cell::from(changes),
            Cell::styled(key.expected(), Token::Dim),
        ]);
    }
    // Without an explicit base the command line keeps the terminal's own colors.
    let base = theme
        .iter()
        .find(|(key, _)| key == "base")
        .map_or("terminal colors", |(_, value)| value.as_str());
    let source = if theme.is_empty() {
        "default"
    } else {
        "global"
    };
    table.row([
        Cell::from("theme.base"),
        Cell::from(base),
        Cell::styled(source, Token::Dim),
        Cell::from("global CLI"),
        Cell::styled(cli_theme_bases(), Token::Dim),
    ]);
    for (key, value) in theme.iter().filter(|(key, _)| key != "base") {
        table.row([
            Cell::from(format!("theme.{key}")),
            Cell::from(value.as_str()),
            Cell::styled("global", Token::Dim),
            Cell::from("global file only"),
            Cell::styled("#rrggbb or a color name", Token::Dim),
        ]);
    }
    list::write(
        output,
        terminal,
        &[Section {
            title: "settings",
            count: None,
            rows: table,
            note: None,
            hint: Some(
                "CLI numeric writes use unsigned decimal integers; config rm removes local overrides only",
            ),
        }],
    )?;
    writeln!(output)?;
    let home = std::env::home_dir();
    let path = |path: &std::path::Path| value::home_path(path, home.as_deref());
    tmt_cli_style::detail::write(
        output,
        terminal,
        "PATHS",
        &[
            ("global", path(&paths.global_config)),
            ("local", path(&paths.local_config)),
        ],
    )
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] =
    &[crate::cli_style_tests::HintSpec::skipped(
        "tmt sq theme set auto",
        "External extension grammar is owned by its CLI; core parsing cannot validate it.",
    )];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editable_theme_bases_reuse_the_style_registry_without_board_auto() {
        assert_eq!(cli_theme_bases(), "tmt, tmt-light, terminal, mono");
        for base in tmt_cli_style::Base::ALL {
            let parsed = editable_theme_base(base.name());
            if base == tmt_cli_style::Base::Auto {
                assert!(parsed.is_err());
            } else {
                assert_eq!(parsed.unwrap(), base);
            }
        }
        for name in ["unknown", "TMT", "tmt ", ""] {
            let error = editable_theme_base(name).unwrap_err().document();
            assert_eq!(
                error["error"]["suggestion"],
                "Valid bases: tmt, tmt-light, terminal, mono"
            );
        }
    }

    #[test]
    fn global_auto_has_a_short_error_and_action_hint() {
        let theme = vec![("base".into(), "auto".into())];
        let error = crate::appearance::parse(&theme).unwrap_err();
        let problem = ThemeProblem {
            key: error.key,
            message: error.message,
        };
        assert!(problem.message.contains("[board.theme]"));
        assert!(problem.message.contains("[squad.<name>.theme]"));
        let mut output = Vec::new();
        write_theme_problem(
            &mut output,
            tmt_cli_style::Terminal::PLAIN,
            &theme,
            &problem,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "error: theme.base auto only works for the board, not in config.json\nhint: tmt sq theme set auto\n"
        );
    }

    #[test]
    fn other_theme_errors_keep_their_key_and_consequence() {
        let problem = ThemeProblem {
            key: "theme.waiting".into(),
            message: "unknown color orange.".into(),
        };
        let mut output = Vec::new();
        write_theme_problem(&mut output, tmt_cli_style::Terminal::PLAIN, &[], &problem).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "error: theme.waiting unknown color orange; commands use the terminal's colors until it is fixed\n"
        );
    }
}
