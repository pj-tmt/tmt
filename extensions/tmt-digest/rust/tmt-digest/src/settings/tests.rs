use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
const MEMBER: &str = "10000000-0000-4000-8000-000000000001";
const SETTER: &str = "20000000-0000-4000-8000-000000000001";
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Save one member's setting without caps.
fn set(
    path: &Path,
    id: &str,
    setting: &MemberSetting,
    setter: Option<&str>,
    now: i64,
) -> Result<(Mode, i64), Error> {
    set_caps(path, id, setting, &Caps::default(), setter, now)
}

fn set_default(path: &Path, mode: &Mode, setter: Option<&str>, now: i64) -> Result<(), Error> {
    set_default_caps(path, mode, &Caps::default(), setter, now)
}

#[test]
fn exact_positive_subminute_durations_and_overflow_admission() {
    for (text, expected) in [
        ("20s", 20_000),
        ("0.5s", 500),
        ("1ms", 1),
        ("0.5m", 30_000),
        ("30", 30_000),
        ("2d", 172_800_000),
        ("10000000.1h", 36_000_000_360_000),
    ] {
        assert_eq!(duration(text).unwrap(), expected);
    }
    for text in [
        "0",
        "0ms",
        "-1s",
        "NaN",
        "inf",
        ".5s",
        "1.s",
        "0.1ms",
        "1.0001s",
        "999999999999999999999h",
        "1 s",
        "1s ",
        "1e3s",
    ] {
        assert!(duration(text).is_err(), "{text}");
    }
}
#[test]
fn default_inheritance_is_distinct_from_explicit_auto_and_zero_count_is_invalid() {
    let source = format!(
        "default = '30m'\nflushCount = 10\n[members.{MEMBER}]\nmode = 'auto'\nsetByIdentityId = '{SETTER}'\nsetAtMs = 1\n"
    );
    let mut settings = DigestConfig::parse(source.as_bytes()).unwrap();
    assert_eq!(settings.effective(MEMBER).unwrap(), Mode::Auto);
    settings
        .edit(MEMBER, &MemberSetting::Default, &Caps::default(), None, 2)
        .unwrap();
    assert_eq!(settings.effective(MEMBER).unwrap().text(), "30m");
    assert!(!settings.document.to_string().contains(SETTER));
    assert_eq!(DigestConfig::parse(b"").unwrap().flush_count, 10);
    for source in [
        "flushCount = 0",
        "flushCount = -1",
        "flushCount = '10'",
        "default = 'default'",
        "[members.worker]\nmode='off'",
    ] {
        assert!(DigestConfig::parse(source.as_bytes()).is_err(), "{source}");
    }
}
#[test]
fn atomic_roundtrip_preserves_comments_records_setter_and_rejects_bad_file() {
    let directory = std::env::temp_dir().join(format!(
        "tmt-digest-settings-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    let path = directory.join("digest.toml");
    fs::write(
        &path,
        "# user comment\ndefault = '30m'\nflushCount = 5\nfuture = 'keep'\n",
    )
    .unwrap();
    let (effective, count) = set(
        &path,
        MEMBER,
        &MemberSetting::Override(Mode::Auto),
        Some(SETTER),
        3,
    )
    .unwrap();
    assert_eq!(effective, Mode::Auto);
    assert_eq!(count, 5);
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("# user comment"));
    assert!(text.contains("future = 'keep'"));
    assert!(text.contains(SETTER));
    assert!(text.contains("setAtMs = 3"));
    set(&path, MEMBER, &MemberSetting::Override(Mode::Off), None, 4).unwrap();
    assert!(!fs::read_to_string(&path).unwrap().contains(SETTER));
    let (effective, _) = set(&path, MEMBER, &MemberSetting::Default, None, 5).unwrap();
    assert_eq!(effective.text(), "30m");
    assert!(!fs::read_to_string(&path).unwrap().contains("setAtMs"));
    fs::write(&path, "flushCount = 0\n").unwrap();
    let before = fs::read(&path).unwrap();
    assert!(set(&path, MEMBER, &MemberSetting::Override(Mode::Off), None, 6).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(
        !directory
            .join(format!(".digest.toml.{}", std::process::id()))
            .exists()
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn inline_member_tables_accept_new_rows_without_losing_unknown_fields() {
    let mut settings = DigestConfig::parse(b"members = {}\nfuture = 'keep'\n").unwrap();
    settings
        .edit(
            MEMBER,
            &MemberSetting::Override(Mode::Auto),
            &Caps::default(),
            Some(SETTER),
            1,
        )
        .unwrap();
    let bytes = settings.document.to_string();
    assert!(bytes.contains("future = 'keep'"));
    let mut reloaded = DigestConfig::parse(bytes.as_bytes()).unwrap();
    assert_eq!(reloaded.effective(MEMBER).unwrap(), Mode::Auto);
    reloaded
        .edit(MEMBER, &MemberSetting::Default, &Caps::default(), None, 2)
        .unwrap();
    assert!(!reloaded.document.to_string().contains(SETTER));
}

#[test]
fn the_global_default_records_its_setter_and_members_that_inherit_it_adopt_it() {
    let directory = std::env::temp_dir().join(format!(
        "tmt-digest-settings-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    let path = directory.join("digest.toml");
    fs::write(
        &path,
        format!(
            "# user comment\nflushCount = 5\nfuture = 'keep'\n[members.{MEMBER}]\nmode = '5m'\nsetAtMs = 1\n"
        ),
    )
    .unwrap();
    // Nothing is recorded anywhere yet: the member owns its own hold.
    let before = DigestConfig::load(&path).unwrap();
    assert_eq!(before.recorded_setter(MEMBER), None);
    assert_eq!(before.default_setter(), None);
    set_default(&path, &Mode::Auto, Some(SETTER), 7).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("# user comment"));
    assert!(text.contains("future = 'keep'"));
    assert!(text.contains("default = \"auto\""));
    assert!(text.contains("defaultSetAtMs = 7"));
    assert!(text.contains(&format!("defaultSetByIdentityId = \"{SETTER}\"")));
    let settings = DigestConfig::load(&path).unwrap();
    assert_eq!(settings.default, Mode::Auto);
    assert_eq!(settings.flush_count, 5);
    // An override without a setter does not borrow the global one; an inherited member does.
    let other = "10000000-0000-4000-8000-000000000002";
    assert_eq!(settings.recorded_setter(MEMBER), None);
    assert_eq!(settings.recorded_setter(other), Some(SETTER));
    assert_eq!(settings.effective(other).unwrap(), Mode::Auto);
    // A write with no known setter forgets the old one rather than keeping a stale owner.
    set_default(&path, &Mode::parse("20m").unwrap(), None, 8).unwrap();
    let settings = DigestConfig::load(&path).unwrap();
    assert_eq!(settings.default.text(), "20m");
    assert_eq!(settings.recorded_setter(other), None);
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("defaultSetByIdentityId"));
    assert!(text.contains("defaultSetAtMs = 8"));
    // An override with its own setter keeps it.
    set(
        &path,
        other,
        &MemberSetting::Override(Mode::Off),
        Some(SETTER),
        9,
    )
    .unwrap();
    assert_eq!(
        DigestConfig::load(&path).unwrap().recorded_setter(other),
        Some(SETTER)
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_malformed_default_attribution_is_refused_without_changing_the_file() {
    for source in [
        "defaultSetByIdentityId = 'worker'",
        "defaultSetByIdentityId = 7",
        "defaultSetAtMs = -1",
        "defaultSetAtMs = '1'",
        "defaultSetAtMs = 9007199254740992",
    ] {
        assert!(DigestConfig::parse(source.as_bytes()).is_err(), "{source}");
    }
    assert!(DigestConfig::parse(b"defaultSetAtMs = 0\n").is_ok());
}

const OTHER: &str = "10000000-0000-4000-8000-000000000002";

#[test]
fn the_switch_masks_every_setting_and_leaves_the_stored_ones_untouched() {
    let source = format!(
        "default = '30m'\nflushCount = 5\n[members.{MEMBER}]\nmode = 'auto'\nsetAtMs = 1\n[members.{OTHER}]\nmode = '10m'\nsetAtMs = 1\n"
    );
    let mut settings = DigestConfig::parse(source.as_bytes()).unwrap();
    assert!(settings.enabled());
    settings.edit_enabled(false).unwrap();
    assert!(!settings.enabled());
    // A member with its own auto, a member with its own interval and one that inherits.
    for id in [MEMBER, OTHER, "10000000-0000-4000-8000-000000000003"] {
        assert_eq!(settings.effective(id).unwrap(), Mode::Off, "{id}");
    }
    let off = settings.document.to_string();
    assert!(off.contains("enabled = false"));
    assert!(off.contains("default = '30m'"));
    assert!(off.contains("mode = 'auto'") && off.contains("mode = '10m'"));
    // The stored settings come back as they were, including a non-auto default.
    let mut reloaded = DigestConfig::parse(off.as_bytes()).unwrap();
    assert!(!reloaded.enabled());
    reloaded.edit_enabled(true).unwrap();
    assert_eq!(reloaded.effective(MEMBER).unwrap(), Mode::Auto);
    assert_eq!(reloaded.effective(OTHER).unwrap().text(), "10m");
    assert_eq!(
        reloaded
            .effective("10000000-0000-4000-8000-000000000003")
            .unwrap()
            .text(),
        "30m"
    );
    assert!(!reloaded.document.to_string().contains("enabled"));
}

#[test]
fn the_switch_must_be_a_boolean_and_an_absent_one_means_on() {
    assert!(DigestConfig::parse(b"").unwrap().enabled());
    assert!(!DigestConfig::parse(b"enabled = false\n").unwrap().enabled());
    assert!(DigestConfig::parse(b"enabled = true\n").unwrap().enabled());
    for source in ["enabled = 'no'", "enabled = 0", "enabled = 'false'"] {
        assert!(DigestConfig::parse(source.as_bytes()).is_err(), "{source}");
    }
}

#[test]
fn setting_the_switch_or_a_default_through_the_file_helpers_keeps_the_rest() {
    let directory = std::env::temp_dir().join(format!(
        "tmt-digest-settings-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    let path = directory.join("digest.toml");
    fs::write(&path, "# keep\ndefault = '30m'\nfuture = 'keep'\n").unwrap();
    set_enabled(&path, false).unwrap();
    let off = DigestConfig::load(&path).unwrap();
    assert!(!off.enabled());
    assert_eq!(off.default.text(), "30m");
    assert!(fs::read_to_string(&path).unwrap().contains("# keep"));
    // Turning it on restores the default the person had.
    set_enabled(&path, true).unwrap();
    let on = DigestConfig::load(&path).unwrap();
    assert!(on.enabled());
    assert_eq!(on.default.text(), "30m");
    // Choosing a new global default is also choosing to batch.
    set_enabled(&path, false).unwrap();
    set_default(&path, &Mode::Auto, None, 9).unwrap();
    assert!(DigestConfig::load(&path).unwrap().enabled());
    // A legacy `default = "off"` has no default to restore: turning on makes it auto.
    fs::write(&path, "default = 'off'\n").unwrap();
    assert_eq!(
        DigestConfig::load(&path)
            .unwrap()
            .effective(MEMBER)
            .unwrap(),
        Mode::Off
    );
    set_enabled(&path, true).unwrap();
    assert_eq!(DigestConfig::load(&path).unwrap().default, Mode::Auto);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn auto_caps_default_to_five_messages_and_five_minutes_and_a_member_value_beats_the_global_one() {
    let none = DigestConfig::parse(b"").unwrap();
    assert_eq!(
        none.auto_caps(MEMBER).unwrap(),
        AutoCaps {
            max_wait_ms: 300_000,
            max_count: 5
        }
    );
    let source = format!(
        "maxWait = '90s'\nmaxCount = 8\n[members.{MEMBER}]\nmode = 'auto'\nmaxWait = '45s'\n[members.{OTHER}]\nmode = 'auto'\n"
    );
    let settings = DigestConfig::parse(source.as_bytes()).unwrap();
    // One field at a time: the member's wait, the global count.
    assert_eq!(
        settings.auto_caps(MEMBER).unwrap(),
        AutoCaps {
            max_wait_ms: 45_000,
            max_count: 8
        }
    );
    assert_eq!(
        settings.auto_caps(OTHER).unwrap(),
        AutoCaps {
            max_wait_ms: 90_000,
            max_count: 8
        }
    );
    // A member that inherits an auto default uses the global caps as well.
    assert_eq!(
        settings
            .auto_caps("10000000-0000-4000-8000-000000000003")
            .unwrap()
            .max_count,
        8
    );
}

#[test]
fn auto_caps_are_refused_outside_their_bounds_and_on_a_member_that_is_not_auto() {
    assert!(caps_wait("30s").is_ok() && caps_wait("1h").is_ok());
    assert!(caps_wait("3600s").is_ok());
    for text in ["29s", "29999ms", "3601s", "1h1s", "0s", "soon", "-5s"] {
        let error = caps_wait(text).unwrap_err();
        assert!(
            text == "soon"
                || text == "-5s"
                || error.message.contains("30s") && error.message.contains("1h"),
            "{text}: {}",
            error.message
        );
    }
    assert!(max_count(1).is_ok() && max_count(100).is_ok());
    for count in [0, 101, 1000] {
        let error = max_count(count).unwrap_err();
        assert!(
            error.message.contains("1") && error.message.contains("100"),
            "{count}"
        );
    }
    for source in [
        "maxWait = '10s'",
        "maxWait = 90",
        "maxCount = 0",
        "maxCount = 101",
        "maxCount = '5'",
        // Caps belong to auto: a hand-written row for another mode is refused.
        "[members.10000000-0000-4000-8000-000000000001]\nmode = '5m'\nmaxCount = 3\n",
        "[members.10000000-0000-4000-8000-000000000001]\nmode = 'off'\nmaxWait = '60s'\n",
    ] {
        assert!(DigestConfig::parse(source.as_bytes()).is_err(), "{source}");
    }
}

fn caps_wait(text: &str) -> Result<u64, Error> {
    max_wait(text)
}

#[test]
fn saving_caps_keeps_them_until_another_value_or_a_non_auto_mode_replaces_them() {
    let directory = std::env::temp_dir().join(format!(
        "tmt-digest-settings-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    let path = directory.join("digest.toml");
    let caps = |wait: Option<&str>, count: Option<u64>| Caps {
        max_wait_ms: wait.map(|text| max_wait(text).unwrap()),
        max_count: count.map(|count| max_count(count).unwrap()),
    };
    // Global: auto with both caps, then auto again with only one keeps the other.
    set_default_caps(&path, &Mode::Auto, &caps(Some("90s"), Some(3)), None, 1).unwrap();
    set_default_caps(&path, &Mode::Auto, &caps(None, Some(4)), None, 2).unwrap();
    let settings = DigestConfig::load(&path).unwrap();
    assert_eq!(
        settings.auto_caps(MEMBER).unwrap(),
        AutoCaps {
            max_wait_ms: 90_000,
            max_count: 4
        }
    );
    // A new global interval keeps the caps: members that are auto still use them.
    set_default_caps(
        &path,
        &Mode::parse("10m").unwrap(),
        &Caps::default(),
        None,
        3,
    )
    .unwrap();
    assert_eq!(
        DigestConfig::load(&path)
            .unwrap()
            .auto_caps(MEMBER)
            .unwrap()
            .max_count,
        4
    );
    // Caps with a mode that is not auto are refused and write nothing.
    let before = fs::read(&path).unwrap();
    assert!(
        set_default_caps(
            &path,
            &Mode::parse("10m").unwrap(),
            &caps(Some("60s"), None),
            None,
            4
        )
        .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    // Member: caps ride with auto, survive a bare auto, and vanish with another mode.
    set_caps(
        &path,
        MEMBER,
        &MemberSetting::Override(Mode::Auto),
        &caps(Some("60s"), Some(2)),
        None,
        5,
    )
    .unwrap();
    assert_eq!(
        DigestConfig::load(&path)
            .unwrap()
            .auto_caps(MEMBER)
            .unwrap(),
        AutoCaps {
            max_wait_ms: 60_000,
            max_count: 2
        }
    );
    set_caps(
        &path,
        MEMBER,
        &MemberSetting::Override(Mode::Auto),
        &Caps::default(),
        None,
        6,
    )
    .unwrap();
    assert_eq!(
        DigestConfig::load(&path)
            .unwrap()
            .auto_caps(MEMBER)
            .unwrap()
            .max_count,
        2
    );
    set_caps(
        &path,
        MEMBER,
        &MemberSetting::Override(Mode::parse("5m").unwrap()),
        &Caps::default(),
        None,
        7,
    )
    .unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains(
            "members.10000000-0000-4000-8000-000000000001]\nmode = \"5m\"\nsetAtMs = 7\nmaxWait"
        ),
        "{text}"
    );
    assert_eq!(
        DigestConfig::load(&path)
            .unwrap()
            .auto_caps(MEMBER)
            .unwrap()
            .max_count,
        4,
        "back to the global caps"
    );
    assert!(
        set_caps(
            &path,
            MEMBER,
            &MemberSetting::Override(Mode::Off),
            &caps(None, Some(2)),
            None,
            8
        )
        .is_err()
    );
    fs::remove_dir_all(directory).unwrap();
}
