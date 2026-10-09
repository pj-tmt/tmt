use super::*;
use crate::config::Config;
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicUsize, Ordering},
};

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "squad-path-migration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let config = root.join("config");
        let data = root.join("data");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir(&data).unwrap();
        Self { root, config, data }
    }
    fn old_config(&self) -> Vec<u8> {
        let bytes = b"# Hand edited\nopaque = 'literal' # preserve\n[squad.product.board]\nrefresh = '5s'\n";
        fs::write(self.config.join("squad.toml"), bytes).unwrap();
        bytes.to_vec()
    }
    fn old_state(&self) -> PathBuf {
        let root = self.data.join("squad");
        fs::create_dir_all(root.join("cron")).unwrap();
        fs::create_dir_all(root.join("checklist/room")).unwrap();
        for (path, bytes) in [
            ("cron/jobs.lock", b"".as_slice()),
            ("cron/clock.lock", b""),
            ("cron/jobs.json", b"{\"literal\":\"tmt sq / tmt-squad\"}"),
            ("checklist/room/items.lock", b""),
            ("checklist/room/items.json", b"literal checklist bytes"),
        ] {
            fs::write(root.join(path), bytes).unwrap();
        }
        root
    }
    fn run(&self) -> io::Result<(Paths, Option<String>)> {
        prepare(&self.config, &self.data, 100)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn current_decision_requires_a_selected_nonlegacy_layout_without_creating_paths() {
    let f = Fixture::new();
    let original = f.old_config();
    assert!(!Decision::default().is_current());
    for (selected, expected) in [
        (Err(failed("migration unavailable")), false),
        (Ok(Paths::at(&f.config, true)), false),
        (Ok(Paths::at(&f.config, false)), true),
    ] {
        let decision = Decision(Mutex::new(Some(selected)));
        assert_eq!(decision.is_current(), expected);
        assert!(!f.data.join("ops").exists());
        assert_eq!(fs::read(f.config.join("squad.toml")).unwrap(), original);
        assert_eq!(fs::read_dir(&f.config).unwrap().count(), 1);
    }
}

#[test]
fn fresh_install_and_completed_fast_path_never_read_legacy_or_take_lock() {
    let f = Fixture::new();
    let (paths, warning) = f.run().unwrap();
    assert!(!paths.legacy);
    assert!(warning.is_none());
    assert!(!paths.config.exists());
    assert!(!f.data.join("ops").exists());
    // New legacy files after completion must be completely ignored.
    fs::write(f.config.join("squad.toml"), "invalid TOML [").unwrap();
    fs::write(f.config.join(PENDING), "invalid journal").unwrap();
    let guard = lock(&f.config.join(LOCK), false).unwrap().unwrap();
    assert!(completed(&f.config).unwrap());
    assert!(
        ready(&f.config)
            .unwrap()
            .unwrap()
            .1
            .unwrap()
            .contains("has reappeared")
    );
    drop(guard);
}

#[test]
fn old_only_preserves_exact_comments_state_and_user_backups_then_archives() {
    let f = Fixture::new();
    let original = f.old_config();
    let old = f.old_state();
    fs::write(f.config.join("squad.toml.bak-user"), "keep backup").unwrap();
    let (paths, warning) = f.run().unwrap();
    assert_eq!(paths.config, f.config.join("ops.toml"));
    assert!(warning.is_none());
    assert_eq!(bytes(&paths.config).unwrap(), original);
    let stamp = format!("100-{}", std::process::id());
    equal(
        &f.data.join(format!("squad.migrated-{stamp}")),
        &f.data.join("ops"),
    )
    .unwrap();
    assert!(!old.exists());
    assert!(!f.config.join("squad.toml").exists());
    assert_eq!(
        bytes(&f.config.join(format!("squad.toml.migrated-{stamp}"))).unwrap(),
        original
    );
    assert_eq!(
        bytes(&f.config.join("squad.toml.bak-user")).unwrap(),
        b"keep backup"
    );
    assert!(completed(&f.config).unwrap());
    assert!(!f.config.join(PENDING).exists());
}

#[test]
fn both_present_new_wins_old_is_untouched_and_only_first_run_notices() {
    let f = Fixture::new();
    let original = f.old_config();
    let old = f.old_state();
    fs::write(f.config.join("ops.toml"), "new='wins'\n").unwrap();
    fs::create_dir(f.data.join("ops")).unwrap();
    fs::write(f.data.join("ops/keep"), "new").unwrap();
    let (_, warning) = f.run().unwrap();
    let warning = warning.unwrap();
    assert!(warning.contains(&f.config.join("squad.toml").display().to_string()));
    assert!(warning.contains(&old.display().to_string()));
    assert_eq!(bytes(&f.config.join("squad.toml")).unwrap(), original);
    assert_eq!(bytes(&f.config.join("ops.toml")).unwrap(), b"new='wins'\n");
    assert_eq!(bytes(&f.data.join("ops/keep")).unwrap(), b"new");
    assert!(f.run().unwrap().1.is_none());
}

#[test]
fn live_old_clock_defers_entire_cutover_and_later_invocation_migrates() {
    let f = Fixture::new();
    let original = f.old_config();
    let root = f.old_state();
    fs::write(
        root.join("cron/clock.json"),
        json!({"version":1,"pid":123,"pane":"%41","sinceMs":50,"expiresMs":1000}).to_string(),
    )
    .unwrap();
    let (paths, warning) = f.run().unwrap();
    assert!(paths.legacy);
    let warning = warning.unwrap();
    assert!(warning.contains("PID 123 in pane %41"));
    assert!(warning.contains("board switch completes"));
    assert!(!warning.contains("kill -TERM"));
    assert!(!completed(&f.config).unwrap());
    assert!(!f.config.join(PENDING).exists());
    assert!(!f.config.join("ops.toml").exists());
    assert!(!f.data.join("ops").exists());
    assert_eq!(bytes(&paths.config).unwrap(), original);
    fs::remove_file(root.join("cron/clock.json")).unwrap();
    assert!(!f.run().unwrap().0.legacy);
    assert!(f.data.join("ops/cron/jobs.json").exists());
}

#[test]
fn concurrent_first_runs_wait_and_observe_one_completed_cutover() {
    let f = Fixture::new();
    f.old_config();
    f.old_state();
    let barrier = Arc::new(Barrier::new(5));
    let threads = (0..4)
        .map(|_| {
            let barrier = barrier.clone();
            let config = f.config.clone();
            let data = f.data.clone();
            std::thread::spawn(move || {
                barrier.wait();
                prepare(&config, &data, 100).unwrap()
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    for thread in threads {
        assert!(!thread.join().unwrap().0.legacy);
    }
    assert_eq!(
        fs::read_dir(&f.data)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e
                .file_name()
                .to_string_lossy()
                .starts_with("squad.migrated-"))
            .count(),
        1
    );
}

#[test]
fn old_board_cas_fails_after_archival_and_never_recreates_config() {
    let f = Fixture::new();
    let original = f.old_config();
    let old = f.config.join("squad.toml");
    let mut board = Config::read(old.clone()).unwrap();
    f.run().unwrap();
    assert_eq!(
        board
            .set_theme_base(&crate::theme::ThemeScope::Board, tmt_cli_style::Base::Mono)
            .unwrap_err()
            .code,
        "SQUAD_CONFIG_CHANGED"
    );
    assert!(!old.exists());
    assert_eq!(bytes(&f.config.join("ops.toml")).unwrap(), original);
}

#[test]
fn invalid_config_unsafe_state_and_unwritable_target_preserve_sources() {
    let f = Fixture::new();
    f.old_config();
    let old = f.old_state();
    let outside = f.root.join("outside");
    fs::write(&outside, "keep").unwrap();
    std::os::unix::fs::symlink(&outside, old.join("foreign")).unwrap();
    assert!(f.run().is_err());
    assert_eq!(bytes(&outside).unwrap(), b"keep");
    fs::remove_file(old.join("foreign")).unwrap();
    fs::write(f.config.join("squad.toml"), "invalid [").unwrap();
    assert!(f.run().is_err());
    assert!(!f.config.join("ops.toml").exists());
    assert!(!completed(&f.config).unwrap());
    let f = Fixture::new();
    let original = f.old_config();
    fs::set_permissions(&f.config, fs::Permissions::from_mode(0o500)).unwrap();
    assert!(f.run().is_err());
    assert_eq!(bytes(&f.config.join("squad.toml")).unwrap(), original);
    fs::set_permissions(&f.config, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn interrupted_publication_resumes_archival_instead_of_treating_both_as_user_conflict() {
    let f = Fixture::new();
    f.old_config();
    f.old_state();
    let stamp = "90-123";
    write_new_file(
        &f.config.join(PENDING),
        json!({"stamp":stamp,"config":true,"state":true})
            .to_string()
            .as_bytes(),
        0o600,
    )
    .unwrap();
    let config = Move::new(&f.config, true, stamp);
    let state = Move::new(&f.data, false, stamp);
    config.stage().unwrap();
    state.stage().unwrap();
    // Simulate process loss between publication and source archival.
    fs::hard_link(&config.stage, &config.target).unwrap();
    fs::remove_file(&config.stage).unwrap();
    f.run().unwrap();
    assert!(config.backup.exists());
    assert!(state.backup.exists());
    assert!(!config.source.exists());
    assert!(!state.source.exists());
    equal(&config.backup, &config.target).unwrap();
    equal(&state.backup, &state.target).unwrap();
}

#[test]
fn pending_new_board_with_absent_baseline_cannot_recreate_old_config_after_cutover() {
    let f = Fixture::new();
    let root = f.old_state();
    fs::write(
        root.join("cron/clock.json"),
        json!({"version":1,"pid":123,"pane":null,"sinceMs":50,"expiresMs":1000}).to_string(),
    )
    .unwrap();
    assert!(f.run().unwrap().0.legacy);
    let mut pending = Config::read(f.config.join("squad.toml")).unwrap();
    fs::remove_file(root.join("cron/clock.json")).unwrap();
    f.run().unwrap();
    assert_eq!(
        pending
            .set_theme_base(&crate::theme::ThemeScope::Board, tmt_cli_style::Base::Mono)
            .unwrap_err()
            .code,
        "SQUAD_CONFIG_CHANGED"
    );
    assert!(!f.config.join("squad.toml").exists());
    assert!(!f.config.join("ops.toml").exists());
}

#[test]
fn reappeared_config_is_not_read_or_merged_and_migrated_backup_is_never_detected() {
    let f = Fixture::new();
    f.old_config();
    f.run().unwrap();
    assert!(ready(&f.config).unwrap().unwrap().1.is_none());
    let original = bytes(&f.config.join("ops.toml")).unwrap();
    fs::write(f.config.join("squad.toml"), "invalid TOML [").unwrap();
    let (paths, warning) = ready(&f.config).unwrap().unwrap();
    assert!(!paths.legacy);
    let warning = warning.unwrap();
    assert!(warning.contains("has reappeared"));
    assert!(warning.contains("settings are not used"));
    assert!(warning.contains("then delete"));
    assert_eq!(bytes(&f.config.join("ops.toml")).unwrap(), original);
    assert_eq!(
        bytes(&f.config.join("squad.toml")).unwrap(),
        b"invalid TOML ["
    );
}

#[test]
fn interrupted_copy_rebuilds_only_journal_owned_unpublished_staging() {
    let f = Fixture::new();
    f.old_config();
    f.old_state();
    let stamp = "90-123";
    write_new_file(
        &f.config.join(PENDING),
        json!({"stamp":stamp,"config":true,"state":true})
            .to_string()
            .as_bytes(),
        0o600,
    )
    .unwrap();
    let state = Move::new(&f.data, false, stamp);
    fs::create_dir(&state.stage).unwrap();
    fs::write(state.stage.join("partial"), "incomplete").unwrap();
    f.run().unwrap();
    equal(&state.backup, &state.target).unwrap();
    assert!(!state.stage.exists());
}

#[test]
fn pending_invocation_clones_never_mix_layouts_and_state_writes_refuse_later_cutover() {
    use crate::cron_service::{
        Mutation,
        test_support::{Fixture as CronFixture, LEAD, WORKER},
    };
    let mut f = CronFixture::new();
    let actor = f.actor(LEAD);
    let job = f.add(&actor, WORKER).unwrap().job.job;
    let original = fs::read(f.directory.join("ops/cron/jobs.json")).unwrap();
    fs::rename(f.directory.join("ops"), f.directory.join("squad")).unwrap();
    fs::rename(f.directory.join("ops.toml"), f.directory.join("squad.toml")).unwrap();
    fs::remove_file(f.directory.join(COMPLETE)).unwrap();
    fs::remove_file(f.directory.join(CUTOVER)).unwrap();
    let lease = f.directory.join("squad/cron/clock.json");
    fs::write(f.directory.join("squad/cron/clock.lock"), "").unwrap();
    let now = jiff::Timestamp::now().as_millisecond();
    fs::write(
        &lease,
        json!({"version":1,"pid":123,"pane":"%41","sinceMs":now-1000,"expiresMs":now+120000})
            .to_string(),
    )
    .unwrap();
    f.core = Core::at(f.core.executable().into());
    f.config = Config::load(&f.core).unwrap();
    assert!(paths(&f.core, None).unwrap().legacy);
    let clone = f.core.clone();
    fs::remove_file(lease).unwrap();
    let next = Core::at(f.core.executable().into());
    assert_eq!(
        Config::load(&next).unwrap().path(),
        f.directory.join("ops.toml")
    );
    assert!(
        paths(&clone, None).unwrap().legacy,
        "the old invocation never selects Ops halfway through"
    );
    assert_eq!(
        f.mutate(&actor, &job, Mutation::Pause).err().unwrap().code,
        "SQUAD_CONFIG_CHANGED"
    );
    assert!(!f.directory.join("squad").exists());
    assert!(!f.directory.join("squad.toml").exists());
    assert_eq!(
        fs::read(f.directory.join("ops/cron/jobs.json")).unwrap(),
        original
    );
    assert!(Config::load(&clone).is_err());
}

#[test]
fn active_legacy_writer_defers_instead_of_hanging_board_startup() {
    let f = Fixture::new();
    f.old_config();
    let old = f.old_state();
    let _writer = lock(&old.join("cron/jobs.lock"), false).unwrap().unwrap();
    let (paths, warning) = f.run().unwrap();
    assert!(paths.legacy);
    assert!(warning.unwrap().contains("active writer"));
    assert!(!f.config.join(COMPLETE).exists());
    assert!(!f.data.join("ops").exists());
}

#[test]
fn interrupted_before_durable_cutover_keeps_all_legacy_sources_for_live_lease_deferral() {
    let f = Fixture::new();
    let original = f.old_config();
    let old = f.old_state();
    let stamp = "90-123";
    write_new_file(
        &f.config.join(PENDING),
        json!({"stamp":stamp,"config":true,"state":true})
            .to_string()
            .as_bytes(),
        0o600,
    )
    .unwrap();
    let config = Move::new(&f.config, true, stamp);
    let state = Move::new(&f.data, false, stamp);
    config.stage().unwrap();
    state.stage().unwrap();
    config.publish().unwrap();
    state.publish().unwrap();
    assert!(!config.backup.exists());
    assert!(!state.backup.exists());
    fs::write(
        old.join("cron/clock.json"),
        json!({"version":1,"pid":123,"pane":"%41","sinceMs":50,"expiresMs":1000}).to_string(),
    )
    .unwrap();
    let (paths, warning) = f.run().unwrap();
    assert!(paths.legacy);
    assert!(warning.unwrap().contains("PID 123"));
    assert_eq!(bytes(&paths.config).unwrap(), original);
    assert!(old.join("cron/jobs.json").exists());
    assert!(!marked(&f.config, CUTOVER).unwrap());
    fs::remove_file(old.join("cron/clock.json")).unwrap();
    assert!(!f.run().unwrap().0.legacy);
    assert!(config.backup.exists());
    assert!(state.backup.exists());
}

#[test]
fn durable_cutover_resumes_archival_without_reading_legacy_or_reverting_later_ops_writes() {
    let f = Fixture::new();
    let original = f.old_config();
    f.old_state();
    let stamp = "90-123";
    write_new_file(
        &f.config.join(PENDING),
        json!({"stamp":stamp,"config":true,"state":true})
            .to_string()
            .as_bytes(),
        0o600,
    )
    .unwrap();
    let config = Move::new(&f.config, true, stamp);
    let state = Move::new(&f.data, false, stamp);
    config.stage().unwrap();
    state.stage().unwrap();
    config.publish().unwrap();
    state.publish().unwrap();
    mark_cutover(
        &f.config.join(CUTOVER),
        json!({"version":1,"ignoredConfig":null,"stamp":stamp,"config":true,"state":true})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    assert!(completed(&f.config).unwrap());
    assert!(config_write_guard(&config.source).is_err());
    config.archive().unwrap(); // Simulate interruption between the two archives.
    fs::write(&config.target, "later='Ops edit'\n").unwrap();
    fs::write(state.target.join("cron/jobs.json"), "later Ops jobs").unwrap();
    // Even malformed legacy content, an alleged live lease, or a held old lock
    // cannot cause any legacy content reads after the durable cutover decision.
    fs::write(state.source.join("cron/clock.json"), "invalid live lease [").unwrap();
    let _old_writer = lock(&state.source.join("cron/jobs.lock"), false)
        .unwrap()
        .unwrap();
    fs::write(f.config.join(PENDING), "invalid stale journal").unwrap();
    let (paths, warning) = f.run().unwrap();
    assert!(!paths.legacy);
    assert!(warning.is_none());
    assert_eq!(bytes(&config.target).unwrap(), b"later='Ops edit'\n");
    assert_eq!(bytes(&config.backup).unwrap(), original);
    assert_eq!(
        bytes(&state.target.join("cron/jobs.json")).unwrap(),
        b"later Ops jobs"
    );
    assert!(state.backup.join("cron/clock.json").exists());
    assert!(marked(&f.config, COMPLETE).unwrap());
    assert!(!f.config.join(PENDING).exists());
}

#[test]
fn marker_publication_recovers_partial_staging_and_never_publishes_partial_content() {
    let f = Fixture::new();
    fs::write(f.config.join(".ops-paths-cutover-v1.tmp"), "partial").unwrap();
    assert!(!completed(&f.config).unwrap());
    f.old_config();
    f.run().unwrap();
    let value: Value = serde_json::from_slice(&bytes(&f.config.join(CUTOVER)).unwrap()).unwrap();
    assert_eq!(value["version"], 1);
    assert!(!f.config.join(".ops-paths-cutover-v1.tmp").exists());
}

#[test]
fn both_present_ignores_unrelated_unsafe_legacy_state_and_busy_writers() {
    let f = Fixture::new();
    let original = f.old_config();
    let old = f.old_state();
    fs::write(f.config.join("ops.toml"), "new='wins'\n").unwrap();
    fs::create_dir(f.data.join("ops")).unwrap();
    let foreign = f.root.join("foreign");
    fs::write(&foreign, "never read or changed").unwrap();
    std::os::unix::fs::symlink(&foreign, old.join("foreign")).unwrap();
    let _writer = lock(&old.join("cron/jobs.lock"), false).unwrap().unwrap();
    let (paths, warning) = f.run().unwrap();
    assert!(!paths.legacy);
    assert!(warning.unwrap().contains("legacy paths left untouched"));
    assert_eq!(bytes(&paths.config).unwrap(), b"new='wins'\n");
    assert_eq!(bytes(&f.config.join("squad.toml")).unwrap(), original);
    assert!(old.join("foreign").is_symlink());
    assert_eq!(bytes(&foreign).unwrap(), b"never read or changed");
    assert!(f.run().unwrap().1.is_none());
}

#[test]
fn both_present_still_defers_whole_layout_for_a_live_old_clock() {
    let f = Fixture::new();
    f.old_config();
    let old = f.old_state();
    fs::write(f.config.join("ops.toml"), "new='wins'\n").unwrap();
    fs::create_dir(f.data.join("ops")).unwrap();
    fs::write(
        old.join("cron/clock.json"),
        json!({"version":1,"pid":123,"pane":"%41","sinceMs":50,"expiresMs":1000}).to_string(),
    )
    .unwrap();
    let (paths, warning) = f.run().unwrap();
    assert!(paths.legacy);
    assert_eq!(paths.config, f.config.join("squad.toml"));
    assert!(warning.unwrap().contains("PID 123 in pane %41"));
    assert!(!completed(&f.config).unwrap());
    fs::remove_file(old.join("cron/clock.json")).unwrap();
    assert!(!f.run().unwrap().0.legacy);
    assert!(f.config.join("squad.toml").exists());
    assert!(old.join("cron/jobs.json").exists());
    assert_eq!(bytes(&f.config.join("ops.toml")).unwrap(), b"new='wins'\n");
}

#[test]
fn deferred_retry_preserves_guard_root_pairing_then_promotes_every_clone() {
    use crate::cron_service::test_support::Fixture as CronFixture;
    let f = CronFixture::new();
    paths(&f.core, None).unwrap();
    // No state files are needed: a config-only cutover still takes the same lock.
    fs::rename(f.directory.join("ops.toml"), f.directory.join("squad.toml")).unwrap();
    fs::remove_file(f.directory.join(COMPLETE)).unwrap();
    fs::remove_file(f.directory.join(CUTOVER)).unwrap();
    let old = f.directory.join("squad/cron");
    fs::create_dir_all(&old).unwrap();
    let time = jiff::Timestamp::now().as_millisecond();
    let lease = tmt_ops::cron::Clock::new(&f.directory.join("squad"))
        .unwrap()
        .acquire(time, 123, Some("%41".into()))
        .unwrap()
        .unwrap();
    let core = Core::at(f.core.executable().into());
    let stale = Config::load(&core).unwrap();
    let clone = core.clone();
    assert!(paths(&clone, None).unwrap().legacy);
    assert!(
        core.paths
            .board_notice()
            .unwrap()
            .contains("old board in pane %41")
    );
    lease.release().unwrap();
    let guard = state_guard(&core).unwrap();
    assert!(
        retry(&clone).unwrap().legacy,
        "cutover must not wait on a retained shared guard"
    );
    assert_eq!(state_root(&core).unwrap(), f.directory.join("squad"));
    drop(guard);
    assert!(!retry(&clone).unwrap().legacy);
    assert!(!paths(&core, None).unwrap().legacy);
    assert!(core.paths.board_notice().is_none());
    assert_eq!(state_root(&core).unwrap(), f.directory.join("ops"));
    assert_eq!(
        config_write_guard(stale.path()).err().unwrap().code,
        "SQUAD_CONFIG_CHANGED"
    );
    assert!(f.directory.join(COMPLETE).exists());
    assert!(!f.directory.join("squad.toml").exists());
}

#[test]
fn retry_writer_deferral_explains_paused_sends_and_migration_failure_is_shared() {
    use crate::cron_service::test_support::Fixture as CronFixture;
    let f = CronFixture::new();
    paths(&f.core, None).unwrap();
    fs::rename(f.directory.join("ops.toml"), f.directory.join("squad.toml")).unwrap();
    fs::remove_file(f.directory.join(COMPLETE)).unwrap();
    fs::remove_file(f.directory.join(CUTOVER)).unwrap();
    let old = f.directory.join("squad/cron");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("jobs.lock"), "").unwrap();
    let guard = lock(&old.join("jobs.lock"), false).unwrap().unwrap();
    let core = Core::at(f.core.executable().into());
    assert!(paths(&core, None).unwrap().legacy);
    assert_eq!(
        core.paths.board_notice().unwrap(),
        "Ops migration pending: quit old Squad boards (q); Ops then retries."
    );
    assert!(retry(&core).unwrap().legacy);
    drop(guard);
    fs::write(f.directory.join("squad.toml"), "invalid TOML [").unwrap();
    assert_eq!(
        retry(&core).unwrap_err().code,
        "SQUAD_PATH_MIGRATION_FAILED"
    );
    assert_eq!(
        paths(&core.clone(), None).unwrap_err().code,
        "SQUAD_PATH_MIGRATION_FAILED"
    );
    assert!(!f.directory.join(CUTOVER).exists());
    assert!(f.directory.join("squad.toml").exists());
}
