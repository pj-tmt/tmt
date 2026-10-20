use super::*;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

fn temp(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let directory = std::env::temp_dir().join(format!(
        "ops-idle-alert-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&directory).unwrap();
    directory.join("ops.toml")
}

fn config(path: &PathBuf, text: &str) -> Config {
    fs::write(path, text).unwrap();
    Config::read(path.clone()).unwrap()
}

fn entry(config: &Config, squad: &str, key: &str) -> (serde_json::Value, String) {
    let settings = config.settings(Some(squad), false, None).unwrap();
    let found = settings
        .entries
        .iter()
        .find(|entry| entry.key == key)
        .unwrap_or_else(|| panic!("{key} is not listed"));
    (found.value.clone(), found.source.clone())
}

#[test]
fn the_alert_is_off_for_every_layout_and_waits_thirty_minutes() {
    let path = temp("defaults");
    let config = config(
        &path,
        "[squad.t]\nlayout = \"team\"\n[squad.c]\nlayout = \"crew\"\n[squad.m]\nlayout = \"minimal\"\n",
    );
    for squad in ["t", "c", "m", "absent"] {
        assert_eq!(
            config.idle_alert(squad).unwrap(),
            IdleAlert {
                enabled: false,
                after: Duration::from_secs(1800)
            },
            "{squad}"
        );
    }
    assert!(config.idle_alert_squads().is_empty());
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_squad_sets_both_keys_and_only_enabled_squads_are_listed() {
    let path = temp("valid");
    let config = config(
        &path,
        "[squad.a.idle_alert]\nenabled = true\nafter = \"20m\"\n\
         [squad.b.idle_alert]\nenabled = false\nafter = \"5m\"\n\
         [squad.c.idle_alert]\nafter = \"2h\"\n[squad.d]\nlayout = \"crew\"\n",
    );
    assert_eq!(
        config.idle_alert("a").unwrap(),
        IdleAlert {
            enabled: true,
            after: Duration::from_secs(1200)
        }
    );
    assert_eq!(config.idle_alert("b").unwrap().after.as_secs(), 300);
    assert!(!config.idle_alert("c").unwrap().enabled);
    assert_eq!(config.idle_alert_squads(), ["a"]);
    for (value, seconds) in [
        ("300s", 300),
        ("5m", 300),
        ("0005m", 300),
        ("1h", 3600),
        ("24h", 86400),
        ("1440m", 86400),
        ("86400s", 86400),
    ] {
        let config = config_with(&path, &format!("[squad.a.idle_alert]\nafter = {value:?}\n"));
        assert_eq!(config.idle_alert("a").unwrap().after.as_secs(), seconds);
    }
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

fn config_with(path: &PathBuf, text: &str) -> Config {
    config(path, text)
}

#[test]
fn an_invalid_value_is_refused_by_name_and_never_changes_the_file() {
    let path = temp("invalid");
    for setting in [
        "enabled = 1",
        "enabled = \"true\"",
        "after = 300",
        "after = \"299s\"",
        "after = \"4m\"",
        "after = \"25h\"",
        "after = \"1441m\"",
        "after = \"1.5h\"",
        "after = \"+5m\"",
        "after = \"-5m\"",
        "after = \"5d\"",
        "after = \"5\"",
        "after = \"5分\"",
        "after = \"18446744073709551615h\"",
        "every = \"5m\"",
    ] {
        let original = format!("[squad.p.idle_alert]\n{setting}\n");
        let config = config(&path, &original);
        let error = config.idle_alert("p").unwrap_err();
        assert_eq!(error.code, "SQUAD_CONFIG_INVALID", "{setting}");
        assert!(error.message.contains("squad.p.idle_alert"), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }
    let config = config(&path, "[squad.p]\nidle_alert = true\n");
    assert!(config.idle_alert("p").is_err());
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn config_set_writes_the_keys_through_the_validated_writer_and_keeps_comments() {
    let path = temp("set");
    let original = "# mine\n[squad.p]\nlayout = \"team\" # kept\n";
    let mut config = config(&path, original);
    assert!(config.can_edit_setting("idle_alert.enabled", Some("p")));
    assert!(config.can_edit_setting("idle_alert.after", Some("p")));
    assert!(!config.can_edit_setting("idle_alert.enabled", None));
    assert!(
        config
            .set_setting(Some("p"), "idle_alert.enabled", "true")
            .unwrap()
    );
    assert!(
        config
            .set_setting(Some("p"), "idle_alert.after", "20m")
            .unwrap()
    );
    assert!(
        !config
            .set_setting(Some("p"), "idle_alert.after", "20m")
            .unwrap()
    );
    let written = fs::read_to_string(&path).unwrap();
    assert!(written.starts_with("# mine\n"), "{written}");
    assert!(written.contains("layout = \"team\" # kept"), "{written}");
    let reread = Config::read(path.clone()).unwrap();
    assert_eq!(
        reread.idle_alert("p").unwrap(),
        IdleAlert {
            enabled: true,
            after: Duration::from_secs(1200)
        }
    );
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn config_set_refuses_bad_values_and_writes_nothing() {
    let path = temp("refuse");
    let original = "[squad.p.idle_alert]\nenabled = true\n";
    let mut config = config(&path, original);
    for (key, value) in [
        ("idle_alert.enabled", "yes"),
        ("idle_alert.enabled", "1"),
        ("idle_alert.enabled", ""),
        ("idle_alert.after", "4m"),
        ("idle_alert.after", "299s"),
        ("idle_alert.after", "25h"),
        ("idle_alert.after", "30"),
        ("idle_alert.after", "30d"),
        ("idle_alert.after", "1.5m"),
        ("idle_alert.after", "+30m"),
        ("idle_alert.after", ""),
    ] {
        let error = config.set_setting(Some("p"), key, value).unwrap_err();
        assert_eq!(error.code, "SQUAD_CONFIG_INVALID", "{key}={value}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            original,
            "{key}={value}"
        );
    }
    let error = config
        .set_setting(None, "idle_alert.after", "20m")
        .unwrap_err();
    assert!(error.message.contains("read-only"), "{error}");
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn config_show_lists_both_keys_with_their_source_and_marks_them_editable() {
    let path = temp("show");
    let mut config = config(&path, "[squad.p]\nlayout = \"crew\"\n");
    assert_eq!(
        entry(&config, "p", "idle_alert.enabled"),
        (json!(false), "default:off".to_owned())
    );
    assert_eq!(
        entry(&config, "p", "idle_alert.after"),
        (json!("1800s"), "default:30m".to_owned())
    );
    config
        .set_setting(Some("p"), "idle_alert.enabled", "true")
        .unwrap();
    config
        .set_setting(Some("p"), "idle_alert.after", "45m")
        .unwrap();
    assert_eq!(
        entry(&config, "p", "idle_alert.enabled"),
        (json!(true), "squad.p.idle_alert.enabled".to_owned())
    );
    assert_eq!(
        entry(&config, "p", "idle_alert.after"),
        (json!("2700s"), "squad.p.idle_alert.after".to_owned())
    );
    let settings = config.settings(Some("p"), false, None).unwrap();
    assert!(
        settings
            .entries
            .iter()
            .filter(|entry| entry.key.starts_with("idle_alert."))
            .all(|entry| entry.editable)
    );
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_set_key_can_be_reset_to_its_default() {
    let path = temp("reset");
    let mut config = config(
        &path,
        "[squad.p.idle_alert]\nenabled = true\nafter = \"10m\"\n",
    );
    assert!(config.reset_setting("p", "idle_alert.after").unwrap());
    assert_eq!(config.idle_alert("p").unwrap().after.as_secs(), 1800);
    assert!(config.idle_alert("p").unwrap().enabled);
    assert!(!config.reset_setting("p", "idle_alert.after").unwrap());
    let _ = fs::remove_dir_all(path.parent().unwrap());
}
