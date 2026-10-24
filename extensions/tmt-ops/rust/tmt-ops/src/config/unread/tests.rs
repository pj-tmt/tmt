use super::*;
use std::{fs, path::PathBuf};
const ACTOR: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";
const FIRST: &str = "33333333-3333-4333-8333-333333333333";
const SECOND: &str = "44444444-4444-4444-8444-444444444444";
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ops-ack-migration-{}",
            crate::id::new_v4().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("ops.toml"), format!("# authored\n[workspace]\nlayout='side' # keep comment\n[workspace.read_markers.{ACTOR}]\n{FIRST}=100\n{SECOND}=200\n[workspace.read_markers.{OTHER}]\n{FIRST}=300\n")).unwrap();
        Self(path)
    }
    fn config(&self) -> Config {
        Config::read(self.0.join("ops.toml")).unwrap()
    }
    fn bytes(&self) -> Vec<u8> {
        fs::read(self.0.join("ops.toml")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn migration_consumes_only_exact_cutoffs_and_preserves_other_preferences_and_actors() {
    let fixture = Fixture::new();
    let mut stale = fixture.config();
    let held = stale.read_markers().unwrap().for_actor(ACTOR);
    let text = String::from_utf8(fixture.bytes())
        .unwrap()
        .replace(&format!("{FIRST}=100"), &format!("{FIRST}=101"));
    fs::write(fixture.0.join("ops.toml"), text).unwrap();
    stale.consume_read_markers(ACTOR, &held).unwrap();
    let current = fixture.config();
    let markers = current.read_markers().unwrap();
    assert_eq!(
        markers.get(ACTOR, FIRST),
        101,
        "a concurrent newer legacy marker survives"
    );
    assert_eq!(markers.get(ACTOR, SECOND), 0);
    assert_eq!(markers.get(OTHER, FIRST), 300);
    let text = String::from_utf8(fixture.bytes()).unwrap();
    assert!(text.contains("# authored"));
    assert!(text.contains("layout='side' # keep comment"));
    assert_eq!(
        current.workspace_preferences().unwrap(),
        stale.workspace_preferences().unwrap()
    );
    let before = fixture.bytes();
    stale.consume_read_markers(ACTOR, &held).unwrap();
    assert_eq!(
        fixture.bytes(),
        before,
        "repeating a settled migration is idempotent"
    );
}
#[test]
fn concurrent_migration_consumers_preserve_both_settlements_and_config_cas() {
    let fixture = Fixture::new();
    let mut stale = fixture.config();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads = [(FIRST, 100), (SECOND, 200)]
        .into_iter()
        .map(|(member, cutoff)| {
            let mut config = fixture.config();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                config
                    .consume_read_markers(ACTOR, &BTreeMap::from([(member.into(), cutoff)]))
                    .unwrap();
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
    assert!(
        fixture
            .config()
            .read_markers()
            .unwrap()
            .for_actor(ACTOR)
            .is_empty()
    );
    assert_eq!(
        fixture.config().read_markers().unwrap().get(OTHER, FIRST),
        300
    );
    let bytes = fixture.bytes();
    assert_eq!(
        stale
            .set_theme_base(&crate::theme::ThemeScope::Board, tmt_cli_style::Base::Tmt)
            .unwrap_err()
            .code,
        "SQUAD_CONFIG_CHANGED"
    );
    assert_eq!(fixture.bytes(), bytes);
}
#[test]
fn migration_lock_symlink_refuses_without_modifying_the_document() {
    let fixture = Fixture::new();
    let mut config = fixture.config();
    let held = config.read_markers().unwrap().for_actor(ACTOR);
    config
        .consume_read_markers(ACTOR, &BTreeMap::new())
        .unwrap();
    let before = fixture.bytes();
    fs::remove_dir_all(fixture.0.join("document-locks")).unwrap();
    let sentinel = fixture.0.join("sentinel");
    fs::create_dir(&sentinel).unwrap();
    std::os::unix::fs::symlink(&sentinel, fixture.0.join("document-locks")).unwrap();
    assert!(config.consume_read_markers(ACTOR, &held).is_err());
    assert_eq!(fixture.bytes(), before);
    assert_eq!(fs::read_dir(sentinel).unwrap().count(), 0);
}
#[test]
fn inline_legacy_cutoffs_keep_their_comment_and_invalid_markers_are_rejected() {
    let fixture = Fixture::new();
    fs::write(
        fixture.0.join("ops.toml"),
        format!("workspace={{layout='side',read_markers={{{ACTOR}={{{FIRST}=10}}}}}} # inline\n"),
    )
    .unwrap();
    let mut config = fixture.config();
    let held = config.read_markers().unwrap().for_actor(ACTOR);
    config.consume_read_markers(ACTOR, &held).unwrap();
    assert!(
        String::from_utf8(fixture.bytes())
            .unwrap()
            .contains("# inline")
    );
    let mut markers = ReadMarkers::default();
    for (actor, member, at) in [
        ("../you", FIRST, 1),
        (ACTOR, "name", 1),
        (ACTOR, FIRST, 0),
        (ACTOR, FIRST, MAX_TIMESTAMP + 1),
    ] {
        assert!(markers.advance(actor, member, at).is_err());
    }
    assert!(markers.is_empty());
}
