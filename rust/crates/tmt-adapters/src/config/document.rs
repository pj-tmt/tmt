use serde_json::{Map, Value, json};
use std::{fs, path::Path};
use tmt_core::settings::{Scalar, Setting, SettingKey, Settings};

use super::{ConfigError, Scope};

struct Field {
    key: SettingKey,
    container: Option<&'static str>,
    property: &'static str,
}

const GLOBAL_FIELDS: [Field; 8] = [
    Field {
        key: SettingKey::PreambleMode,
        container: None,
        property: "preambleMode",
    },
    Field {
        key: SettingKey::RetentionDays,
        container: Some("exchange"),
        property: "retentionDays",
    },
    Field {
        key: SettingKey::PaneBadge,
        container: Some("ui"),
        property: "paneBadge",
    },
    Field {
        key: SettingKey::Timeout,
        container: Some("defaults"),
        property: "timeout",
    },
    Field {
        key: SettingKey::PollInterval,
        container: Some("defaults"),
        property: "pollInterval",
    },
    Field {
        key: SettingKey::CaptureLines,
        container: Some("defaults"),
        property: "captureLines",
    },
    Field {
        key: SettingKey::PreambleEvery,
        container: Some("defaults"),
        property: "preambleEvery",
    },
    Field {
        key: SettingKey::PasteEnterDelayMs,
        container: Some("defaults"),
        property: "pasteEnterDelayMs",
    },
];
const LOCAL_FIELDS: [Field; 3] = [
    Field {
        key: SettingKey::PreambleMode,
        container: Some("$config"),
        property: "preambleMode",
    },
    Field {
        key: SettingKey::PreambleEvery,
        container: Some("$config"),
        property: "preambleEvery",
    },
    Field {
        key: SettingKey::PasteEnterDelayMs,
        container: Some("$config"),
        property: "pasteEnterDelayMs",
    },
];

fn fields(scope: Scope) -> &'static [Field] {
    match scope {
        Scope::Global => &GLOBAL_FIELDS,
        Scope::Local => &LOCAL_FIELDS,
    }
}

fn object<'a>(
    value: &'a Value,
    path: &Path,
    field: &str,
) -> Result<&'a Map<String, Value>, ConfigError> {
    value
        .as_object()
        .ok_or_else(|| ConfigError::validation(path, field, "a non-null object"))
}

fn shape(value: &Value, path: &Path, scope: Scope) -> Result<(), ConfigError> {
    let root = object(value, path, "<root>")?;
    let containers: &[&str] = match scope {
        Scope::Global => &["defaults", "exchange", "ui"],
        Scope::Local => &["$config"],
    };
    for name in containers {
        if let Some(value) = root.get(*name) {
            object(value, path, name)?;
        }
    }
    Ok(())
}

pub(super) fn read(path: &Path, scope: Scope) -> Result<Value, ConfigError> {
    let value = if path.exists() {
        let content = fs::read_to_string(path).map_err(|error| ConfigError::parse(path, error))?;
        crate::json_document::parse(&content).map_err(|error| ConfigError::parse(path, error))?
    } else {
        json!({})
    };
    shape(&value, path, scope)?;
    Ok(value)
}

pub(super) fn project(
    value: &Value,
    path: &Path,
    scope: Scope,
) -> Result<Vec<Setting>, ConfigError> {
    shape(value, path, scope)?;
    let mut settings = Vec::new();
    for field in fields(scope) {
        let container = field.container.map_or(Some(value), |name| value.get(name));
        let Some(value) = container.and_then(|container| container.get(field.property)) else {
            continue;
        };
        let scalar = match value {
            Value::String(text) => Scalar::Text(text),
            Value::Number(number) => number.as_f64().map_or(Scalar::Invalid, Scalar::Number),
            _ => Scalar::Invalid,
        };
        let setting = Setting::validate(field.key, scalar).ok_or_else(|| {
            let label = field.container.map_or_else(
                || field.property.to_string(),
                |container| format!("{container}.{}", field.property),
            );
            ConfigError::validation(path, &label, field.key.expected())
        })?;
        settings.push(setting);
    }
    Ok(settings)
}

/// The global file's `theme` settings as written. Only the shape is checked
/// here, an object of strings; what a theme value means belongs to the CLI
/// style, which the caller parses it with. A file without `theme` has none.
/// A wrong shape is a [`ThemeProblem`], never a configuration error: the
/// theme is presentation and must not stop a command.
pub(super) fn theme(value: &Value) -> Result<Vec<(String, String)>, ThemeProblem> {
    let Some(theme) = value.get("theme") else {
        return Ok(Vec::new());
    };
    let problem = |key: String, message: &str| ThemeProblem {
        key,
        message: message.to_owned(),
    };
    theme
        .as_object()
        .ok_or_else(|| problem("theme".into(), "must be an object of settings"))?
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|text| (key.clone(), text.to_owned()))
                .ok_or_else(|| problem(format!("theme.{key}"), "must be a string"))
        })
        .collect()
}

/// What is wrong with the global `theme`, by the setting it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeProblem {
    pub key: String,
    pub message: String,
}

fn setting_value(setting: Setting) -> Value {
    match setting {
        Setting::PreambleMode(value) => json!(value.as_str()),
        Setting::PaneBadge(value) => json!(value.as_str()),
        Setting::CaptureLines(value)
        | Setting::PreambleEvery(value)
        | Setting::RetentionDays(value) => json!(value),
        Setting::Timeout(value)
        | Setting::PollInterval(value)
        | Setting::PasteEnterDelayMs(value) => {
            // JSON.stringify emits integral numbers without a fractional suffix.
            if value.fract() == 0.0 && value >= 0.0 && value <= u64::MAX as f64 {
                json!(value as u64)
            } else {
                json!(value)
            }
        }
    }
}

pub(super) fn set(value: &mut Value, setting: Setting, scope: Scope) -> Result<(), ConfigError> {
    let field = fields(scope)
        .iter()
        .find(|field| field.key == setting.key())
        .ok_or_else(|| ConfigError::internal("Unsupported configuration scope"))?;
    // read() validated all relevant containers before this mutation.
    let root = value.as_object_mut().expect("validated configuration root");
    let target = if let Some(container) = field.container {
        let initial = if container == "defaults" {
            let defaults = Settings::default();
            json!({
                "timeout": defaults.timeout as u64,
                "pollInterval": defaults.poll_interval as u64,
                "captureLines": defaults.capture_lines,
                "preambleEvery": defaults.preamble_every,
                "pasteEnterDelayMs": defaults.paste_enter_delay_ms as u64,
            })
        } else {
            json!({})
        };
        root.entry(container)
            .or_insert(initial)
            .as_object_mut()
            .expect("validated configuration container")
    } else {
        root
    };
    target.insert(field.property.into(), setting_value(setting));
    Ok(())
}

/// Return false only for a key-specific clear with no local container. The
/// reference command does not create a file in that no-op case.
pub(super) fn clear(value: &mut Value, key: Option<&str>) -> bool {
    let root = value.as_object_mut().expect("validated configuration root");
    if let Some(key) = key {
        let Some(settings) = root.get_mut("$config") else {
            return false;
        };
        let settings = settings.as_object_mut().expect("validated local settings");
        settings.remove(key);
        if settings.is_empty() {
            root.remove("$config");
        }
    } else {
        root.remove("$config");
    }
    true
}

pub(super) fn write(path: &Path, value: &Value, scope: Scope) -> Result<(), ConfigError> {
    project(value, path, scope)?;
    if scope == Scope::Global
        && let Some(parent) = path.parent()
    {
        fs::create_dir_all(parent).map_err(|error| ConfigError::internal(error.to_string()))?;
    }
    let mut content = serde_json::to_string_pretty(value)
        .map_err(|error| ConfigError::internal(error.to_string()))?;
    content.push('\n');
    fs::write(path, content).map_err(|error| ConfigError::internal(error.to_string()))
}
