use super::*;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

const ACTOR: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";
const FIRST: &str = "33333333-3333-4333-8333-333333333333";
const SECOND: &str = "44444444-4444-4444-8444-444444444444";

struct Fixture(PathBuf);
impl Fixture {
    fn new(body: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ops-unread-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("ops.toml"), body).unwrap();
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
fn mark(actor: &str, member: &str, at: u64) -> ReadMarkers {
    let mut markers = ReadMarkers::default();
    assert!(markers.advance(actor, member, at).unwrap());
    markers
}

#[test]
fn stale_views_merge_monotonic_pairs_without_losing_authored_preferences() {
    let fixture = Fixture::new("# keep authored text\n[workspace]\nlayout='side' # keep comment\n");
    let mut first = fixture.config();
    let mut stale = fixture.config();
    let preferences = first.workspace_preferences().unwrap();
    first.save_read_markers(&mark(ACTOR, FIRST, 100)).unwrap();
    stale.save_read_markers(&mark(ACTOR, SECOND, 200)).unwrap();
    stale.save_read_markers(&mark(OTHER, FIRST, 300)).unwrap();
    first.save_read_markers(&mark(ACTOR, FIRST, 50)).unwrap();
    let restarted = fixture.config();
    let seen = restarted.read_markers().unwrap();
    assert_eq!(seen.get(ACTOR, FIRST), 100);
    assert_eq!(seen.get(ACTOR, SECOND), 200);
    assert_eq!(seen.get(OTHER, FIRST), 300);
    assert_eq!(seen.get(OTHER, SECOND), 0);
    assert_eq!(restarted.workspace_preferences().unwrap(), preferences);
    let text = String::from_utf8(fixture.bytes()).unwrap();
    assert!(text.contains("# keep authored text"));
    assert!(text.contains("layout='side' # keep comment"));
}

#[test]
fn concurrent_layouts_keep_both_markers_and_snapshot_edits_still_require_cas() {
    let fixture = Fixture::new("# authored\n");
    let mut stale = fixture.config();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = [(FIRST, 100), (SECOND, 200)]
        .into_iter()
        .map(|(member, at)| {
            let mut config = fixture.config();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                config.save_read_markers(&mark(ACTOR, member, at)).unwrap();
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let current = fixture.config().read_markers().unwrap();
    assert_eq!(current.get(ACTOR, FIRST), 100);
    assert_eq!(current.get(ACTOR, SECOND), 200);
    let bytes = fixture.bytes();
    assert_eq!(
        stale
            .set_theme_base(&crate::theme::ThemeScope::Board, tmt_cli_style::Base::Tmt)
            .unwrap_err()
            .code,
        "SQUAD_CONFIG_CHANGED"
    );
    assert_eq!(fixture.bytes(), bytes);
    fixture
        .config()
        .set_theme_base(&crate::theme::ThemeScope::Board, tmt_cli_style::Base::Tmt)
        .unwrap();
    assert_eq!(fixture.config().read_markers().unwrap(), current);
}

#[test]
fn inline_marks_keep_their_comment_and_invalid_pairs_cannot_modify_the_file() {
    let fixture = Fixture::new(&format!(
        "workspace = {{ layout='side', read_markers = {{ {ACTOR} = {{ {FIRST} = 10 }} }} }} # keep inline\n"
    ));
    let mut config = fixture.config();
    config.save_read_markers(&mark(ACTOR, FIRST, 20)).unwrap();
    config.save_read_markers(&mark(OTHER, SECOND, 30)).unwrap();
    assert_eq!(
        fixture.config().read_markers().unwrap().get(OTHER, SECOND),
        30
    );
    assert!(
        String::from_utf8(fixture.bytes())
            .unwrap()
            .contains("# keep inline")
    );
    let before = fixture.bytes();
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
    assert_eq!(fixture.bytes(), before);
}

#[test]
fn a_symlink_lock_and_an_oversized_edit_preserve_the_original_bytes() {
    let fixture = Fixture::new("# valid configuration\n");
    fixture
        .config()
        .save_read_markers(&mark(ACTOR, FIRST, 10))
        .unwrap();
    let before = fixture.bytes();
    fs::remove_file(fixture.0.join("ops.toml.lock")).unwrap();
    let sentinel = fixture.0.join("sentinel");
    fs::write(&sentinel, b"untouched").unwrap();
    std::os::unix::fs::symlink(&sentinel, fixture.0.join("ops.toml.lock")).unwrap();
    assert!(
        fixture
            .config()
            .save_read_markers(&mark(ACTOR, FIRST, 20))
            .is_err()
    );
    assert_eq!(fixture.bytes(), before);
    assert_eq!(fs::read(sentinel).unwrap(), b"untouched");

    let huge =
        Fixture::new(&("#".to_owned() + &"x".repeat(super::super::FILE_LIMIT as usize - 2) + "\n"));
    assert_eq!(huge.bytes().len() as u64, super::super::FILE_LIMIT);
    let before = huge.bytes();
    assert!(
        huge.config()
            .save_read_markers(&mark(ACTOR, FIRST, 10))
            .unwrap_err()
            .message
            .contains("1 MiB")
    );
    assert_eq!(huge.bytes(), before);
}
