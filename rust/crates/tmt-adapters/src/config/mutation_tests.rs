//! Current-byte config mutations share the destination directory lock.
use super::{ConfigFiles, ConfigPaths, Scope};
use crate::test_support::TestDirectory;
use serde_json::{Value, json};
use std::{fs, sync::mpsc, time::Duration};
use tmt_core::settings::Setting;
use tmt_extension_state::Document;

#[test]
fn a_waiting_config_writer_reads_the_committed_document_and_preserves_both_keys() {
    let directory = TestDirectory::new();
    let paths = ConfigPaths::resolve(
        &directory.path,
        &directory.path,
        Some(&directory.path.join("global")),
        None,
    );
    fs::create_dir_all(&paths.global_dir).unwrap();
    fs::write(&paths.global_config, br#"{"future":{"keep":true}}"#).unwrap();
    let document = Document::new(
        &paths.global_dir,
        &paths.global_config,
        u64::MAX,
        "tmt config set preambleEvery 7 --global",
    )
    .unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let mut premature = false;
    std::thread::scope(|scope| {
        let result: Result<(), tmt_extension_state::UpdateError<()>> = document.update(|current| {
            let config = ConfigFiles {
                paths: paths.clone(),
            };
            scope.spawn(move || {
                started_tx.send(()).unwrap();
                let result = config.set(
                    Setting::edit("preambleEvery", "7", Scope::Global).unwrap(),
                    Scope::Global,
                );
                done_tx.send(result).unwrap();
            });
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            premature = done_rx.recv_timeout(Duration::from_millis(200)).is_ok();
            let mut value: Value = serde_json::from_slice(current.unwrap()).unwrap();
            value["notes"] = json!({"compactionReminder": false});
            Ok(serde_json::to_vec(&value).unwrap())
        });
        result.unwrap();
    });
    assert!(
        !premature,
        "config mutation bypassed the shared document lock"
    );
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    let config = ConfigFiles { paths };
    let value: Value =
        serde_json::from_slice(&fs::read(&config.paths.global_config).unwrap()).unwrap();
    assert_eq!(value["defaults"]["preambleEvery"], 7);
    assert_eq!(value["notes"]["compactionReminder"], false);
    assert_eq!(value["future"], json!({"keep":true}));
}

#[test]
fn config_publication_preserves_permissions_and_recovers_only_its_reserved_staging() {
    use std::os::unix::fs::PermissionsExt;
    let directory = TestDirectory::new();
    let config = ConfigFiles {
        paths: ConfigPaths::resolve(
            &directory.path,
            &directory.path,
            Some(&directory.path.join("global")),
            None,
        ),
    };
    config.initialize_local().unwrap();
    fs::set_permissions(
        &config.paths.local_config,
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    let name = config
        .paths
        .local_config
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let staging = directory.path.join(format!(".{name}.tmt-staging"));
    fs::write(&staging, b"{partial crashed publication").unwrap();
    let foreign = directory.path.join("foreign.tmp");
    fs::write(&foreign, b"keep").unwrap();
    config
        .set(
            Setting::edit("preambleEvery", "7", Scope::Local).unwrap(),
            Scope::Local,
        )
        .unwrap();
    assert!(!staging.exists());
    assert_eq!(
        fs::metadata(&config.paths.local_config)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
    assert_eq!(fs::read(&foreign).unwrap(), b"keep");
    assert_eq!(fs::read_dir(&directory.path).unwrap().count(), 3); // config, foreign file, caller state root
    let value: Value =
        serde_json::from_slice(&fs::read(&config.paths.local_config).unwrap()).unwrap();
    assert_eq!(value["$config"]["preambleEvery"], 7);
    assert!(!config.paths.database.exists());
}

#[test]
fn config_mutations_refuse_malformed_or_linked_inputs_without_replacing_them() {
    use std::os::unix::fs::symlink;
    let directory = TestDirectory::new();
    let config = ConfigFiles {
        paths: ConfigPaths::resolve(
            &directory.path,
            &directory.path,
            Some(&directory.path.join("global")),
            None,
        ),
    };
    fs::create_dir_all(&config.paths.global_dir).unwrap();
    let setting = Setting::edit("preambleEvery", "7", Scope::Global).unwrap();
    for content in [
        "{malformed",
        r#"{"defaults":{"timeout":"bad"},"opaque":true}"#,
    ] {
        fs::write(&config.paths.global_config, content).unwrap();
        assert_eq!(
            config.set(setting, Scope::Global).unwrap_err().code,
            "CONFIG_ERROR"
        );
        assert_eq!(
            fs::read_to_string(&config.paths.global_config).unwrap(),
            content
        );
    }
    fs::remove_file(&config.paths.global_config).unwrap();
    let target = directory.path.join("foreign.json");
    fs::write(&target, b"{}").unwrap();
    symlink(&target, &config.paths.global_config).unwrap();
    assert!(config.set(setting, Scope::Global).is_err());
    assert_eq!(fs::read_link(&config.paths.global_config).unwrap(), target);
    assert_eq!(fs::read(&target).unwrap(), b"{}");
}

#[test]
fn clearing_an_absent_key_still_creates_no_local_file_or_state_root() {
    let directory = TestDirectory::new();
    let config = ConfigFiles {
        paths: ConfigPaths::resolve(
            &directory.path,
            &directory.path,
            Some(&directory.path.join("global")),
            None,
        ),
    };
    config
        .clear_local(tmt_core::settings::LocalClear::parse(Some("preambleEvery")).unwrap())
        .unwrap();
    assert_eq!(fs::read_dir(&directory.path).unwrap().count(), 0);
}
