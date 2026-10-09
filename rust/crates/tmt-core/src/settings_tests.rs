use super::limits::{MAX_CAPTURE_LINES, MAX_JS_SAFE_INTEGER, MAX_TIMER_DELAY_MS};
use super::retention::DEFAULT_RETENTION_DAYS;
use super::settings::{
    EDITABLE_KEYS, LocalClear, PaneBadge, PreambleMode, ResolvedSettings, Scalar, Scope, Setting,
    SettingKey, Settings,
};

#[test]
fn defaults_are_canonical_and_isolated() {
    assert_eq!(DEFAULT_RETENTION_DAYS, 90);
    let mut first = Settings::default();
    let second = Settings::default();

    first.timeout = 42.0;
    first.retention_days = 1;

    assert_eq!(second.timeout, 180.0);
    assert_eq!(second.poll_interval, 1.0);
    assert_eq!(second.capture_lines, 100);
    assert_eq!(second.preamble_every, 3);
    assert_eq!(second.paste_enter_delay_ms, 500.0);
    assert_eq!(second.retention_days, DEFAULT_RETENTION_DAYS);
    assert_eq!(second.preamble_mode, PreambleMode::Always);
    assert_eq!(second.pane_badge, PaneBadge::On);
    assert_ne!(first, second);
}

#[test]
fn resolved_layers_share_precedence_and_source_accounting() {
    let resolved = ResolvedSettings::from_layers(
        vec![Setting::PreambleEvery(7), Setting::RetentionDays(30)],
        vec![Setting::PreambleEvery(0)],
    );
    assert_eq!(resolved.settings.preamble_every, 0);
    assert_eq!(resolved.settings.retention_days, 30);
    assert_eq!(resolved.source(SettingKey::PreambleEvery), "local");
    assert_eq!(resolved.source(SettingKey::RetentionDays), "global");
    assert_eq!(resolved.source(SettingKey::PaneBadge), "default");
}

/// #1599: the pane badge is on unless the user said otherwise; an explicit
/// `off` still wins and an explicit `on` is the same as the default.
#[test]
fn the_pane_badge_defaults_on_and_an_explicit_off_wins() {
    let unset = ResolvedSettings::from_layers(Vec::new(), Vec::new());
    assert_eq!(unset.settings.pane_badge, PaneBadge::On);
    assert_eq!(unset.source(SettingKey::PaneBadge), "default");
    let off = ResolvedSettings::from_layers(vec![Setting::PaneBadge(PaneBadge::Off)], Vec::new());
    assert_eq!(off.settings.pane_badge, PaneBadge::Off);
    assert_eq!(off.source(SettingKey::PaneBadge), "global");
    let on = ResolvedSettings::from_layers(vec![Setting::PaneBadge(PaneBadge::On)], Vec::new());
    assert_eq!(on.settings.pane_badge, PaneBadge::On);
    assert_eq!(on.source(SettingKey::PaneBadge), "global");
}

#[test]
fn editing_scope_and_obsolete_clear_policy_are_shared_outside_the_cli() {
    assert_eq!(
        Setting::edit("ui.paneBadge", "on", Scope::Global).unwrap(),
        Setting::PaneBadge(PaneBadge::On)
    );
    assert!(Setting::edit("ui.paneBadge", "on", Scope::Local).is_err());
    assert!(Setting::edit("exchange.retentionDays", "90", Scope::Local).is_err());
    assert!(Setting::edit("mode", "legacy", Scope::Local).is_err());
    assert_eq!(LocalClear::parse(None).unwrap(), LocalClear::All);
    assert_eq!(
        LocalClear::parse(Some("mode")).unwrap(),
        LocalClear::ObsoleteMode
    );
    assert_eq!(
        LocalClear::parse(Some("preambleEvery")).unwrap().key(),
        Some("preambleEvery")
    );
    assert!(LocalClear::parse(Some("exchange.retentionDays")).is_err());
    assert!(LocalClear::parse(Some("ui.paneBadge")).is_err());
    assert!(LocalClear::parse(Some("unknown")).is_err());
}

#[test]
fn editable_keys_are_limited_to_supported_set() {
    let valid = [
        "preambleMode",
        "ui.paneBadge",
        "preambleEvery",
        "pasteEnterDelayMs",
        "exchange.retentionDays",
    ];
    for name in valid {
        let key = SettingKey::editable(name).unwrap_or_else(|error| {
            panic!("expected {name} to be editable, got {error}");
        });
        assert!(EDITABLE_KEYS.contains(&key), "{name}");
    }

    for name in ["timeout", "pollInterval", "captureLines", "unknown", "mode"] {
        assert!(SettingKey::editable(name).is_err(), "{name}");
    }
}

#[test]
fn enum_settings_accept_only_documented_values() {
    let cases = [
        (
            SettingKey::PreambleMode,
            Scalar::Text("always"),
            Some(Setting::PreambleMode(PreambleMode::Always)),
        ),
        (
            SettingKey::PreambleMode,
            Scalar::Text("disabled"),
            Some(Setting::PreambleMode(PreambleMode::Disabled)),
        ),
        (
            SettingKey::PaneBadge,
            Scalar::Text("on"),
            Some(Setting::PaneBadge(PaneBadge::On)),
        ),
        (
            SettingKey::PaneBadge,
            Scalar::Text("off"),
            Some(Setting::PaneBadge(PaneBadge::Off)),
        ),
    ];
    for (key, value, expected) in cases {
        assert_eq!(Setting::validate(key, value), expected);
    }

    for (key, value) in [
        (SettingKey::PreambleMode, "always "),
        (SettingKey::PreambleMode, "on"),
        (SettingKey::PaneBadge, "ON"),
        (SettingKey::PaneBadge, "disabled"),
    ] {
        assert!(
            Setting::validate(key, Scalar::Text(value)).is_none(),
            "{key:?}={value:?}"
        );
    }
}

#[test]
fn loaded_fractional_delay_is_valid_but_cli_edit_requires_integer_text() {
    assert_eq!(
        Setting::validate(SettingKey::PasteEnterDelayMs, Scalar::Number(0.5),),
        Some(Setting::PasteEnterDelayMs(0.5))
    );
    assert!(Setting::parse_edit(SettingKey::PasteEnterDelayMs, "0.5").is_err());
    assert_eq!(
        Setting::parse_edit(SettingKey::PasteEnterDelayMs, "0")
            .expect("zero is a valid integer edit"),
        Setting::PasteEnterDelayMs(0.0)
    );
    assert_eq!(
        Setting::parse_edit(
            SettingKey::PasteEnterDelayMs,
            &MAX_TIMER_DELAY_MS.to_string()
        )
        .expect("maximum delay is a valid integer edit"),
        Setting::PasteEnterDelayMs(MAX_TIMER_DELAY_MS)
    );
}

#[test]
fn timing_settings_require_positive_finite_values_with_their_bounds() {
    for (key, value, expected) in [
        (SettingKey::Timeout, 1.0, Some(Setting::Timeout(1.0))),
        (
            SettingKey::Timeout,
            86_400.0,
            Some(Setting::Timeout(86_400.0)),
        ),
        (
            SettingKey::PollInterval,
            f64::MIN_POSITIVE,
            Some(Setting::PollInterval(f64::MIN_POSITIVE)),
        ),
        (
            SettingKey::PollInterval,
            f64::MAX,
            Some(Setting::PollInterval(f64::MAX)),
        ),
    ] {
        assert_eq!(Setting::validate(key, Scalar::Number(value)), expected);
    }

    for (key, value) in [
        (SettingKey::Timeout, 0.0),
        (SettingKey::Timeout, -0.001),
        (SettingKey::Timeout, 86_400.001),
        (SettingKey::Timeout, f64::NAN),
        (SettingKey::Timeout, f64::INFINITY),
        (SettingKey::PollInterval, 0.0),
        (SettingKey::PollInterval, -0.001),
        (SettingKey::PollInterval, f64::NAN),
        (SettingKey::PollInterval, f64::NEG_INFINITY),
    ] {
        assert!(
            Setting::validate(key, Scalar::Number(value)).is_none(),
            "{key:?}={value:?}"
        );
    }
}

#[test]
fn integer_settings_enforce_safe_and_feature_specific_bounds() {
    for (key, value, expected) in [
        (
            SettingKey::CaptureLines,
            0.0,
            Some(Setting::CaptureLines(0)),
        ),
        (
            SettingKey::CaptureLines,
            MAX_CAPTURE_LINES as f64,
            Some(Setting::CaptureLines(MAX_CAPTURE_LINES)),
        ),
        (
            SettingKey::PreambleEvery,
            0.0,
            Some(Setting::PreambleEvery(0)),
        ),
        (
            SettingKey::PreambleEvery,
            MAX_JS_SAFE_INTEGER as f64,
            Some(Setting::PreambleEvery(MAX_JS_SAFE_INTEGER)),
        ),
        (
            SettingKey::RetentionDays,
            1.0,
            Some(Setting::RetentionDays(1)),
        ),
        (
            SettingKey::RetentionDays,
            3650.0,
            Some(Setting::RetentionDays(3650)),
        ),
    ] {
        assert_eq!(Setting::validate(key, Scalar::Number(value)), expected);
    }

    for (key, value) in [
        (SettingKey::CaptureLines, -1.0),
        (SettingKey::CaptureLines, 1.5),
        (SettingKey::CaptureLines, MAX_CAPTURE_LINES as f64 + 1.0),
        (SettingKey::PreambleEvery, -1.0),
        (SettingKey::PreambleEvery, 1.5),
        (SettingKey::PreambleEvery, MAX_JS_SAFE_INTEGER as f64 + 1.0),
        (SettingKey::RetentionDays, 0.0),
        (SettingKey::RetentionDays, 1.5),
        (SettingKey::RetentionDays, 3651.0),
        (SettingKey::RetentionDays, f64::INFINITY),
    ] {
        assert!(
            Setting::validate(key, Scalar::Number(value)).is_none(),
            "{key:?}={value:?}"
        );
    }
}

#[test]
fn invalid_scalars_are_rejected_without_coercion() {
    for key in [
        SettingKey::PreambleMode,
        SettingKey::Timeout,
        SettingKey::CaptureLines,
        SettingKey::PaneBadge,
    ] {
        assert!(Setting::validate(key, Scalar::Invalid).is_none(), "{key:?}");
    }

    for key in [SettingKey::Timeout, SettingKey::CaptureLines] {
        assert!(Setting::parse_edit(key, "").is_err(), "{key:?}");
        assert!(Setting::parse_edit(key, "true").is_err(), "{key:?}");
        assert!(Setting::parse_edit(key, "null").is_err(), "{key:?}");
    }
}

#[test]
fn parse_edit_uses_strict_integer_syntax_and_exact_enum_values() {
    assert_eq!(
        Setting::parse_edit(SettingKey::PreambleEvery, "0").unwrap(),
        Setting::PreambleEvery(0)
    );
    assert_eq!(
        Setting::parse_edit(SettingKey::PreambleEvery, &MAX_JS_SAFE_INTEGER.to_string()).unwrap(),
        Setting::PreambleEvery(MAX_JS_SAFE_INTEGER)
    );
    assert_eq!(
        Setting::parse_edit(SettingKey::RetentionDays, "3650").unwrap(),
        Setting::RetentionDays(3650)
    );
    assert_eq!(
        Setting::parse_edit(SettingKey::PreambleMode, "disabled").unwrap(),
        Setting::PreambleMode(PreambleMode::Disabled)
    );
    assert_eq!(
        Setting::parse_edit(SettingKey::PaneBadge, "on").unwrap(),
        Setting::PaneBadge(PaneBadge::On)
    );

    for (key, text) in [
        (SettingKey::PreambleEvery, "-1"),
        (SettingKey::PreambleEvery, "1.5"),
        (SettingKey::PreambleEvery, "9007199254740992"),
        (SettingKey::RetentionDays, "0"),
        (SettingKey::RetentionDays, "3651"),
        (SettingKey::PaneBadge, "ON"),
        (SettingKey::PreambleMode, "enabled"),
    ] {
        assert!(Setting::parse_edit(key, text).is_err(), "{key:?}={text:?}");
    }
}

#[test]
fn apply_has_last_write_precedence_across_all_setting_kinds() {
    let mut settings = Settings::default();
    for setting in [
        Setting::PreambleMode(PreambleMode::Disabled),
        Setting::Timeout(86_400.0),
        Setting::PollInterval(2.5),
        Setting::CaptureLines(MAX_CAPTURE_LINES),
        Setting::PreambleEvery(MAX_JS_SAFE_INTEGER),
        Setting::PasteEnterDelayMs(0.5),
        Setting::RetentionDays(1),
        Setting::PaneBadge(PaneBadge::On),
        Setting::Timeout(30.0),
        Setting::RetentionDays(3650),
        Setting::PaneBadge(PaneBadge::Off),
    ] {
        settings.apply(setting);
    }

    assert_eq!(
        settings,
        Settings {
            preamble_mode: PreambleMode::Disabled,
            timeout: 30.0,
            poll_interval: 2.5,
            capture_lines: MAX_CAPTURE_LINES,
            preamble_every: MAX_JS_SAFE_INTEGER,
            paste_enter_delay_ms: 0.5,
            retention_days: 3650,
            pane_badge: PaneBadge::Off,
            reply_batch_window_ms: 5_000,
            typing_quiet_ms: 2_000,
            notes_compaction_reminder: true,
            workspace_snapshot_enabled: true,
            workspace_snapshot_interval_ms: 60_000,
        }
    );
}

#[test]
fn reply_notice_settings_are_bounded_global_millisecond_integers() {
    for (key, maximum) in [
        (SettingKey::ReplyBatchWindowMs, 60_000),
        (SettingKey::TypingQuietMs, 30_000),
    ] {
        assert!(Setting::edit(key.name(), "0", Scope::Global).is_ok());
        assert!(Setting::edit(key.name(), &maximum.to_string(), Scope::Global).is_ok());
        assert!(Setting::edit(key.name(), &(maximum + 1).to_string(), Scope::Global).is_err());
        assert!(Setting::edit(key.name(), "2s", Scope::Global).is_err());
        assert!(Setting::edit(key.name(), "100", Scope::Local).is_err());
        assert!(LocalClear::parse(Some(key.name())).is_err());
    }
}

#[test]
fn notes_reminder_is_a_default_on_global_boolean() {
    let key = SettingKey::NotesCompactionReminder;
    assert!(Settings::default().notes_compaction_reminder);
    assert!(key.global_only());
    for value in [true, false] {
        assert_eq!(
            Setting::validate(key, Scalar::Boolean(value)),
            Some(Setting::NotesCompactionReminder(value))
        );
        assert_eq!(
            Setting::edit(key.name(), &value.to_string(), Scope::Global).unwrap(),
            Setting::NotesCompactionReminder(value)
        );
    }
    for value in [Scalar::Text("true"), Scalar::Number(1.0), Scalar::Invalid] {
        assert!(Setting::validate(key, value).is_none());
    }
    for value in ["TRUE", "1", "on", "false "] {
        assert!(Setting::edit(key.name(), value, Scope::Global).is_err());
    }
    assert!(Setting::edit(key.name(), "false", Scope::Local).is_err());
    assert!(LocalClear::parse(Some(key.name())).is_err());
    let resolved =
        ResolvedSettings::from_layers(vec![Setting::NotesCompactionReminder(false)], vec![]);
    assert!(!resolved.settings.notes_compaction_reminder);
    assert_eq!(resolved.source(key), "global");
}

#[test]
fn workspace_snapshot_settings_are_global_and_interval_is_a_bounded_integer() {
    assert!(Settings::default().workspace_snapshot_enabled);
    assert_eq!(Settings::default().workspace_snapshot_interval_ms, 60_000);
    for key in [
        SettingKey::WorkspaceSnapshotEnabled,
        SettingKey::WorkspaceSnapshotIntervalMs,
    ] {
        assert!(key.global_only());
        assert!(Setting::edit(key.name(), "0", Scope::Local).is_err());
        assert!(LocalClear::parse(Some(key.name())).is_err());
    }
    assert_eq!(
        Setting::edit("workspace.snapshotEnabled", "false", Scope::Global).unwrap(),
        Setting::WorkspaceSnapshotEnabled(false)
    );
    assert!(Setting::edit("workspace.snapshotEnabled", "1", Scope::Global).is_err());
    for value in ["0", "2147483647"] {
        assert!(Setting::edit("workspace.snapshotIntervalMs", value, Scope::Global).is_ok());
    }
    for value in ["-1", "1.5", "2147483648"] {
        assert!(Setting::edit("workspace.snapshotIntervalMs", value, Scope::Global).is_err());
    }
}
