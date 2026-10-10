use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
const MEMBER: &str = "10000000-0000-4000-8000-000000000001";
const SETTER: &str = "20000000-0000-4000-8000-000000000001";
static NEXT: AtomicU64 = AtomicU64::new(0);

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
        .edit(MEMBER, &MemberSetting::Default, None, 2)
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
            Some(SETTER),
            1,
        )
        .unwrap();
    let bytes = settings.document.to_string();
    assert!(bytes.contains("future = 'keep'"));
    let mut reloaded = DigestConfig::parse(bytes.as_bytes()).unwrap();
    assert_eq!(reloaded.effective(MEMBER).unwrap(), Mode::Auto);
    reloaded
        .edit(MEMBER, &MemberSetting::Default, None, 2)
        .unwrap();
    assert!(!reloaded.document.to_string().contains(SETTER));
}
