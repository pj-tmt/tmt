use super::{ConfigFiles, ConfigPaths, Scope};
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
    assert!(config.theme().unwrap().is_empty(), "no file, no theme");
    write_global(
        &config,
        r##"{"theme": {"base": "tmt", "waiting": "#e0a458", "anything": "kept"}}"##,
    );
    let mut theme = config.theme().unwrap();
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
    let error = config.theme().unwrap_err();
    assert_eq!(error.code, "CONFIG_ERROR");
    assert!(
        error.message.contains("(theme.waiting): must be a string"),
        "{}",
        error.message
    );
    assert!(config.load().is_ok());

    write_global(&config, r#"{"theme": ["tmt"]}"#);
    assert!(
        config
            .theme()
            .unwrap_err()
            .message
            .contains("(theme): must be a non-null object")
    );
    assert!(config.load().is_ok());

    write_global(&config, r#"{"theme": {"base": "mono"}}"#);
    config
        .set(Setting::PaneBadge(PaneBadge::On), Scope::Global)
        .unwrap();
    assert_eq!(
        config.theme().unwrap(),
        [("base".to_owned(), "mono".to_owned())]
    );
    assert_eq!(config.load().unwrap().settings.pane_badge, PaneBadge::On);
}
