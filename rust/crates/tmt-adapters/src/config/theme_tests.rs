use super::{ConfigFiles, ConfigPaths, Scope, ThemeProblem};
use crate::test_support::TestDirectory;
use std::{fs, path::Path};
use tmt_core::settings::{PaneBadge, Setting};

fn files(directory: &Path) -> ConfigFiles {
    ConfigFiles {
        paths: ConfigPaths::resolve(directory, directory, Some(&directory.join("global")), None),
    }
}

fn write_global(config: &ConfigFiles, text: &str) {
    fs::create_dir_all(config.paths.global_config.parent().unwrap()).unwrap();
    fs::write(&config.paths.global_config, text).unwrap();
}

#[test]
fn the_theme_is_read_as_written_from_the_global_file() {
    let directory = TestDirectory::new();
    let config = files(&directory.path);
    assert!(
        config.theme().unwrap().unwrap().is_empty(),
        "no file, no theme"
    );
    write_global(
        &config,
        r##"{"theme": {"base": "tmt", "waiting": "#e0a458", "anything": "kept"}}"##,
    );
    let mut theme = config.theme().unwrap().unwrap();
    theme.sort();
    assert_eq!(
        theme,
        [
            ("anything".to_owned(), "kept".to_owned()),
            ("base".to_owned(), "tmt".to_owned()),
            ("waiting".to_owned(), "#e0a458".to_owned()),
        ],
        "meaning is the caller's to check"
    );
}

/// A malformed theme is reported by `theme()` and never breaks loading the
/// other settings, and a `config set` keeps it as written.
#[test]
fn a_malformed_theme_never_breaks_settings_and_survives_a_write() {
    let directory = TestDirectory::new();
    let config = files(&directory.path);
    write_global(&config, r#"{"theme": {"waiting": 3}}"#);
    assert_eq!(
        config.theme().unwrap().unwrap_err(),
        ThemeProblem {
            key: "theme.waiting".into(),
            message: "must be a string".into()
        }
    );
    assert!(config.load().is_ok());

    write_global(&config, r#"{"theme": ["tmt"]}"#);
    assert_eq!(config.theme().unwrap().unwrap_err().key, "theme");
    assert!(config.load().is_ok());

    // An unreadable file stays a configuration error, as for every setting.
    write_global(&config, "{not json");
    assert_eq!(config.theme().unwrap_err().code, "CONFIG_ERROR");

    write_global(&config, r#"{"theme": {"base": "mono"}}"#);
    config
        .set(Setting::PaneBadge(PaneBadge::On), Scope::Global)
        .unwrap();
    assert_eq!(
        config.theme().unwrap().unwrap(),
        [("base".to_owned(), "mono".to_owned())]
    );
    assert_eq!(config.load().unwrap().settings.pane_badge, PaneBadge::On);
}

#[test]
fn setting_the_base_preserves_every_other_value_even_malformed_token_values() {
    let directory = TestDirectory::new();
    let config = files(&directory.path);
    let before = serde_json::json!({
        "theme": {"base": "terminal", "waiting": 3, "future": {"keep": true}},
        "defaults": {"timeout": 30}, "future": [null, {"nested": "keep"}]
    });
    write_global(&config, &before.to_string());
    config.set_theme_base("tmt-light").unwrap();
    let after: serde_json::Value =
        serde_json::from_slice(&fs::read(&config.paths.global_config).unwrap()).unwrap();
    let mut expected = before;
    expected["theme"]["base"] = "tmt-light".into();
    assert_eq!(after, expected);
    assert!(config.load().is_ok());
    assert_eq!(config.theme().unwrap().unwrap_err().key, "theme.waiting");
}

#[test]
fn setting_the_base_creates_a_theme_but_refuses_a_malformed_existing_container() {
    let directory = TestDirectory::new();
    let config = files(&directory.path);
    config.set_theme_base("mono").unwrap();
    assert_eq!(
        config.theme().unwrap().unwrap(),
        [("base".into(), "mono".into())]
    );
    for value in ["null", "3", "[]", "\"mono\""] {
        let before = format!("{{\"theme\":{value},\"future\":true}}");
        write_global(&config, &before);
        assert_eq!(
            config.set_theme_base("tmt").unwrap_err().code,
            "CONFIG_ERROR"
        );
        assert_eq!(
            fs::read_to_string(&config.paths.global_config).unwrap(),
            before
        );
        assert!(config.load().is_ok());
    }
}
