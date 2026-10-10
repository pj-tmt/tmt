//! Configuration policy shared by invocation setup and editing. JSON, paths,
//! files, and presentation belong to adapters rather than this module.

use crate::limits::{
    MAX_CAPTURE_LINES, MAX_JS_SAFE_INTEGER, is_valid_observer_timeout_seconds,
    is_valid_timer_delay_ms,
};

use crate::retention::{DEFAULT_RETENTION_DAYS, MAX_RETENTION_DAYS, valid_retention_days};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalClear {
    All,
    Setting(SettingKey),
    ObsoleteMode,
}

impl LocalClear {
    pub fn parse(key: Option<&str>) -> Result<Self, String> {
        let Some(name) = key else {
            return Ok(Self::All);
        };
        if name == "mode" {
            return Ok(Self::ObsoleteMode);
        }
        let key = SettingKey::editable(name)?;
        if key.global_only() {
            return Err(format!(
                "{name} is global-only and has no local override to clear."
            ));
        }
        Ok(Self::Setting(key))
    }

    pub fn key(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Setting(key) => Some(key.name()),
            Self::ObsoleteMode => Some("mode"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreambleMode {
    Always,
    Disabled,
}

impl PreambleMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneBadge {
    On,
    Off,
}

impl PaneBadge {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKey {
    PreambleMode,
    Timeout,
    PollInterval,
    CaptureLines,
    PreambleEvery,
    PasteEnterDelayMs,
    RetentionDays,
    PaneBadge,
    ReplyBatchWindowMs,
    TypingQuietMs,
    ExperimentalChannel,
    NotesCompactionReminder,
    WorkspaceSnapshotEnabled,
    WorkspaceSnapshotIntervalMs,
}

pub const EDITABLE_KEYS: [SettingKey; 11] = [
    SettingKey::PreambleMode,
    SettingKey::PaneBadge,
    SettingKey::PreambleEvery,
    SettingKey::PasteEnterDelayMs,
    SettingKey::RetentionDays,
    SettingKey::ReplyBatchWindowMs,
    SettingKey::TypingQuietMs,
    SettingKey::ExperimentalChannel,
    SettingKey::NotesCompactionReminder,
    SettingKey::WorkspaceSnapshotEnabled,
    SettingKey::WorkspaceSnapshotIntervalMs,
];

impl SettingKey {
    pub fn name(self) -> &'static str {
        match self {
            Self::PreambleMode => "preambleMode",
            Self::Timeout => "timeout",
            Self::PollInterval => "pollInterval",
            Self::CaptureLines => "captureLines",
            Self::PreambleEvery => "preambleEvery",
            Self::PasteEnterDelayMs => "pasteEnterDelayMs",
            Self::RetentionDays => "exchange.retentionDays",
            Self::PaneBadge => "ui.paneBadge",
            Self::ReplyBatchWindowMs => "notifications.replyBatchWindowMs",
            Self::TypingQuietMs => "notifications.typingQuietMs",
            Self::ExperimentalChannel => "experimental.channel",
            Self::NotesCompactionReminder => "notes.compactionReminder",
            Self::WorkspaceSnapshotEnabled => "workspace.snapshotEnabled",
            Self::WorkspaceSnapshotIntervalMs => "workspace.snapshotIntervalMs",
        }
    }

    pub fn expected(self) -> &'static str {
        match self {
            Self::PreambleMode => "'always' or 'disabled'",
            Self::Timeout => "a finite positive number no greater than 86400",
            Self::PollInterval => "a finite positive number",
            Self::CaptureLines => "an integer from 0 through 2147483647",
            Self::PreambleEvery => "a safe non-negative integer",
            Self::PasteEnterDelayMs => "a finite number from 0 through 2147483647",
            Self::RetentionDays => "an integer from 1 through 3650",
            Self::PaneBadge => "'on' or 'off'",
            Self::ReplyBatchWindowMs => "an integer from 0 through 60000 milliseconds",
            Self::TypingQuietMs => "an integer from 0 through 30000 milliseconds",
            Self::ExperimentalChannel
            | Self::NotesCompactionReminder
            | Self::WorkspaceSnapshotEnabled => "true or false",
            Self::WorkspaceSnapshotIntervalMs => {
                "an integer from 0 through 2147483647 milliseconds"
            }
        }
    }

    pub fn global_only(self) -> bool {
        matches!(
            self,
            Self::RetentionDays
                | Self::PaneBadge
                | Self::ReplyBatchWindowMs
                | Self::TypingQuietMs
                | Self::ExperimentalChannel
                | Self::NotesCompactionReminder
                | Self::WorkspaceSnapshotEnabled
                | Self::WorkspaceSnapshotIntervalMs
        )
    }

    pub fn editable(name: &str) -> Result<Self, String> {
        EDITABLE_KEYS
            .into_iter()
            .find(|key| key.name() == name)
            .ok_or_else(|| {
                format!(
                    "Invalid key: {name}. Valid keys: {}",
                    EDITABLE_KEYS.map(Self::name).join(", ")
                )
            })
    }
}

/// The boundary accepts only scalar kinds needed by policy. Null, arrays,
/// objects become Invalid without making core depend on JSON.
pub enum Scalar<'a> {
    Text(&'a str),
    Number(f64),
    Boolean(bool),
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Setting {
    PreambleMode(PreambleMode),
    Timeout(f64),
    PollInterval(f64),
    CaptureLines(u64),
    PreambleEvery(u64),
    PasteEnterDelayMs(f64),
    RetentionDays(u64),
    PaneBadge(PaneBadge),
    ReplyBatchWindowMs(u64),
    TypingQuietMs(u64),
    ExperimentalChannel(bool),
    NotesCompactionReminder(bool),
    WorkspaceSnapshotEnabled(bool),
    WorkspaceSnapshotIntervalMs(u64),
}

fn unsigned_integer(value: f64, maximum: u64) -> bool {
    value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= maximum as f64
}

impl Setting {
    pub fn edit(name: &str, value: &str, scope: Scope) -> Result<Self, String> {
        let key = SettingKey::editable(name)?;
        if scope == Scope::Local && key.global_only() {
            return Err(format!(
                "{name} can only be set in global config with --global."
            ));
        }
        Self::parse_edit(key, value)
    }

    pub fn validate(key: SettingKey, value: Scalar<'_>) -> Option<Self> {
        match (key, value) {
            (SettingKey::ExperimentalChannel, Scalar::Boolean(value)) => {
                Some(Self::ExperimentalChannel(value))
            }
            (SettingKey::WorkspaceSnapshotEnabled, Scalar::Boolean(value)) => {
                Some(Self::WorkspaceSnapshotEnabled(value))
            }
            (SettingKey::WorkspaceSnapshotIntervalMs, Scalar::Number(value))
                if unsigned_integer(value, 2_147_483_647) =>
            {
                Some(Self::WorkspaceSnapshotIntervalMs(value as u64))
            }
            (SettingKey::NotesCompactionReminder, Scalar::Boolean(value)) => {
                Some(Self::NotesCompactionReminder(value))
            }
            (SettingKey::PreambleMode, Scalar::Text("always")) => {
                Some(Self::PreambleMode(PreambleMode::Always))
            }
            (SettingKey::PreambleMode, Scalar::Text("disabled")) => {
                Some(Self::PreambleMode(PreambleMode::Disabled))
            }
            (SettingKey::PaneBadge, Scalar::Text("on")) => Some(Self::PaneBadge(PaneBadge::On)),
            (SettingKey::PaneBadge, Scalar::Text("off")) => Some(Self::PaneBadge(PaneBadge::Off)),
            (SettingKey::Timeout, Scalar::Number(value))
                if is_valid_observer_timeout_seconds(value) =>
            {
                Some(Self::Timeout(value))
            }
            (SettingKey::PollInterval, Scalar::Number(value))
                if value.is_finite() && value > 0.0 =>
            {
                Some(Self::PollInterval(value))
            }
            (SettingKey::CaptureLines, Scalar::Number(value))
                if unsigned_integer(value, MAX_CAPTURE_LINES) =>
            {
                Some(Self::CaptureLines(value as u64))
            }
            (SettingKey::PreambleEvery, Scalar::Number(value))
                if unsigned_integer(value, MAX_JS_SAFE_INTEGER) =>
            {
                Some(Self::PreambleEvery(value as u64))
            }
            (SettingKey::PasteEnterDelayMs, Scalar::Number(value))
                if is_valid_timer_delay_ms(value) =>
            {
                Some(Self::PasteEnterDelayMs(value))
            }
            (SettingKey::RetentionDays, Scalar::Number(value))
                if unsigned_integer(value, MAX_RETENTION_DAYS)
                    && valid_retention_days(value as u64) =>
            {
                Some(Self::RetentionDays(value as u64))
            }
            (SettingKey::ReplyBatchWindowMs, Scalar::Number(value))
                if unsigned_integer(value, 60_000) =>
            {
                Some(Self::ReplyBatchWindowMs(value as u64))
            }
            (SettingKey::TypingQuietMs, Scalar::Number(value))
                if unsigned_integer(value, 30_000) =>
            {
                Some(Self::TypingQuietMs(value as u64))
            }
            _ => None,
        }
    }

    pub fn parse_edit(key: SettingKey, text: &str) -> Result<Self, String> {
        let scalar = if matches!(
            key,
            SettingKey::ExperimentalChannel
                | SettingKey::NotesCompactionReminder
                | SettingKey::WorkspaceSnapshotEnabled
        ) {
            match text {
                "true" => Scalar::Boolean(true),
                "false" => Scalar::Boolean(false),
                _ => Scalar::Invalid,
            }
        } else if matches!(key, SettingKey::PreambleMode | SettingKey::PaneBadge) {
            Scalar::Text(text)
        } else {
            if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(format!(
                    "Invalid value for {}: {text}. Must be a non-negative integer.",
                    key.name()
                ));
            }
            let number = text.parse::<f64>().unwrap_or(f64::INFINITY);
            if !unsigned_integer(number, MAX_JS_SAFE_INTEGER) {
                return Err(format!(
                    "Invalid value for {}: {text}. Must be a supported non-negative integer.",
                    key.name()
                ));
            }
            Scalar::Number(number)
        };
        Self::validate(key, scalar).ok_or_else(|| {
            let expected = match key {
                SettingKey::PreambleMode => "Valid values: always, disabled",
                SettingKey::PaneBadge => "Valid values: on, off",
                SettingKey::ExperimentalChannel
                | SettingKey::NotesCompactionReminder
                | SettingKey::WorkspaceSnapshotEnabled => "Valid values: true, false",
                _ => "Must be a supported non-negative integer.",
            };
            format!("Invalid value for {}: {text}. {expected}", key.name())
        })
    }

    pub fn key(self) -> SettingKey {
        match self {
            Self::PreambleMode(_) => SettingKey::PreambleMode,
            Self::Timeout(_) => SettingKey::Timeout,
            Self::PollInterval(_) => SettingKey::PollInterval,
            Self::CaptureLines(_) => SettingKey::CaptureLines,
            Self::PreambleEvery(_) => SettingKey::PreambleEvery,
            Self::PasteEnterDelayMs(_) => SettingKey::PasteEnterDelayMs,
            Self::RetentionDays(_) => SettingKey::RetentionDays,
            Self::PaneBadge(_) => SettingKey::PaneBadge,
            Self::ReplyBatchWindowMs(_) => SettingKey::ReplyBatchWindowMs,
            Self::TypingQuietMs(_) => SettingKey::TypingQuietMs,
            Self::ExperimentalChannel(_) => SettingKey::ExperimentalChannel,
            Self::NotesCompactionReminder(_) => SettingKey::NotesCompactionReminder,
            Self::WorkspaceSnapshotEnabled(_) => SettingKey::WorkspaceSnapshotEnabled,
            Self::WorkspaceSnapshotIntervalMs(_) => SettingKey::WorkspaceSnapshotIntervalMs,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub preamble_mode: PreambleMode,
    pub timeout: f64,
    pub poll_interval: f64,
    pub capture_lines: u64,
    pub preamble_every: u64,
    pub paste_enter_delay_ms: f64,
    pub retention_days: u64,
    pub pane_badge: PaneBadge,
    pub reply_batch_window_ms: u64,
    pub typing_quiet_ms: u64,
    pub experimental_channel: bool,
    pub notes_compaction_reminder: bool,
    pub workspace_snapshot_enabled: bool,
    /// Reserved for command refresh (#2102); event capture does not consult it.
    pub workspace_snapshot_interval_ms: u64,
}

pub struct ResolvedSettings {
    pub settings: Settings,
    sources: Vec<(SettingKey, Scope)>,
}

impl ResolvedSettings {
    pub fn from_layers(global: Vec<Setting>, local: Vec<Setting>) -> Self {
        let mut resolved = Self {
            settings: Settings::default(),
            sources: Vec::new(),
        };
        for (scope, settings) in [(Scope::Global, global), (Scope::Local, local)] {
            for setting in settings {
                resolved.sources.push((setting.key(), scope));
                resolved.settings.apply(setting);
            }
        }
        resolved
    }

    pub fn source(&self, key: SettingKey) -> &'static str {
        match self
            .sources
            .iter()
            .rev()
            .find(|(candidate, _)| *candidate == key)
            .map(|(_, scope)| scope)
        {
            Some(Scope::Global) => "global",
            Some(Scope::Local) => "local",
            None => "default",
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            preamble_mode: PreambleMode::Always,
            timeout: 180.0,
            poll_interval: 1.0,
            capture_lines: 100,
            preamble_every: 3,
            paste_enter_delay_ms: 500.0,
            retention_days: DEFAULT_RETENTION_DAYS,
            pane_badge: PaneBadge::On,
            reply_batch_window_ms: 5_000,
            typing_quiet_ms: 2_000,
            experimental_channel: false,
            notes_compaction_reminder: true,
            workspace_snapshot_enabled: true,
            workspace_snapshot_interval_ms: 60_000,
        }
    }
}

impl Settings {
    pub fn apply(&mut self, setting: Setting) {
        match setting {
            Setting::PreambleMode(value) => self.preamble_mode = value,
            Setting::Timeout(value) => self.timeout = value,
            Setting::PollInterval(value) => self.poll_interval = value,
            Setting::CaptureLines(value) => self.capture_lines = value,
            Setting::PreambleEvery(value) => self.preamble_every = value,
            Setting::PasteEnterDelayMs(value) => self.paste_enter_delay_ms = value,
            Setting::RetentionDays(value) => self.retention_days = value,
            Setting::PaneBadge(value) => self.pane_badge = value,
            Setting::ReplyBatchWindowMs(value) => self.reply_batch_window_ms = value,
            Setting::TypingQuietMs(value) => self.typing_quiet_ms = value,
            Setting::ExperimentalChannel(value) => self.experimental_channel = value,
            Setting::NotesCompactionReminder(value) => self.notes_compaction_reminder = value,
            Setting::WorkspaceSnapshotEnabled(value) => self.workspace_snapshot_enabled = value,
            Setting::WorkspaceSnapshotIntervalMs(value) => {
                self.workspace_snapshot_interval_ms = value
            }
        }
    }
}
