//! Thin configuration command composition and presentation. Policy and raw
//! document mutation remain in the shared core and filesystem adapter.

use crate::invocation::{ConfigRequest, OutputMode};
use crate::output::Failure;
use serde_json::json;
use std::io::{self, Write};
use tmt_adapters::config::{ConfigFiles, ConfigPaths, Scope};
use tmt_core::settings::{EDITABLE_KEYS, LocalClear, ResolvedSettings, Setting, SettingKey};

fn invalid_setting(message: String) -> Failure {
    Failure::new("ERROR", message, 1)
}

enum Report {
    Show {
        loaded: ResolvedSettings,
        paths: ConfigPaths,
    },
    Changed(String),
}

fn run(request: ConfigRequest) -> Result<Report, Failure> {
    let files = ConfigFiles {
        paths: ConfigPaths::discover()?,
    };
    match request {
        ConfigRequest::Show => Ok(Report::Show {
            loaded: files.load()?,
            paths: files.paths,
        }),
        ConfigRequest::Set { key, value, global } => {
            let scope = if global { Scope::Global } else { Scope::Local };
            files.set(
                Setting::edit(&key, &value, scope).map_err(invalid_setting)?,
                scope,
            )?;
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
        Report::Show { loaded, paths } => {
            if mode.json {
                writeln!(output, "{}", show_json(&loaded, &paths))?;
            } else {
                show_text(&mut output, terminal, &loaded, &paths)?;
            }
        }
    }
    Ok(0)
}

fn show_json(loaded: &ResolvedSettings, paths: &ConfigPaths) -> serde_json::Value {
    let settings = &loaded.settings;
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
            "ui": { "paneBadge": settings.pane_badge.as_str() },
        },
        "sources": {
            "preambleMode": loaded.source(SettingKey::PreambleMode),
            "preambleEvery": loaded.source(SettingKey::PreambleEvery),
            "pasteEnterDelayMs": loaded.source(SettingKey::PasteEnterDelayMs),
            "exchange": { "retentionDays": loaded.source(SettingKey::RetentionDays) },
            "ui": { "paneBadge": loaded.source(SettingKey::PaneBadge) },
        },
        "paths": { "global": paths.global_config, "local": paths.local_config },
    })
}

fn show_text(
    output: &mut impl Write,
    terminal: tmt_cli_style::Terminal,
    loaded: &ResolvedSettings,
    paths: &ConfigPaths,
) -> io::Result<()> {
    let settings = &loaded.settings;
    let rows = [
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
    list::write(
        output,
        terminal,
        &[Section {
            title: "settings",
            count: None,
            rows: table,
            note: None,
            hint: Some(
                "CLI numeric writes use unsigned decimal integers; config clear removes local overrides only",
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
