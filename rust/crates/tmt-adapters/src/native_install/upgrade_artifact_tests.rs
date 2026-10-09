use super::*;
use crate::native_install::artifact;
use crate::native_install::publication::Layout;
use crate::native_install::{InstallRequest, install_observed};
use crate::process::CommandError;
use crate::process::CommandOutput;
use crate::process::CommandRequest;
use crate::process::CommandRunner;
use crate::process::UnixCommandRunner;
use crate::test_support::TestDirectory;
use serde_json::{Value, json};
use std::{
    env,
    ffi::{OsStr, OsString},
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tmt_core::content_digest::sha256;
use tmt_core::native_install::{Channel, PinAction};

const TASK_ROOT: &str = "TMT_UPGRADE_TASK";
const RELEASE_ID: u64 = 9001;
const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;

fn required_path(name: &str) -> PathBuf {
    PathBuf::from(
        env::var_os(name).unwrap_or_else(|| panic!("explicit acceptance test requires {name}")),
    )
}

fn command_args(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn assignment(name: &str, value: &Path) -> OsString {
    let mut result = OsString::from(name);
    result.push("=");
    result.push(value.as_os_str());
    result
}

fn run(executable: &Path, args: &[OsString], task: &Path) -> Result<CommandOutput, CommandError> {
    let mut command = vec![
        OsString::from("-i"),
        assignment("HOME", task),
        assignment("TMT_HOME", task),
        // Harness only: remove when the prior release is post-rename (#2302).
        assignment("TMUX_TEAM_HOME", task),
        OsString::from("PATH="),
        OsString::from("LANG=C"),
        executable.as_os_str().to_os_string(),
    ];
    command.extend(args.iter().cloned());
    UnixCommandRunner.execute(CommandRequest {
        program: OsStr::new("/usr/bin/env"),
        args: &command,
        input: &[],
        deadline: Instant::now() + Duration::from_secs(20),
        max_output_bytes: OUTPUT_LIMIT,
    })
}

fn release_document(
    version: &semver::Version,
    archive_name: &str,
    manifest: &[u8],
    archive: &[u8],
) -> Value {
    json!({
        "id": RELEASE_ID,
        "tag_name": format!("v{version}"),
        "draft": false,
        "immutable": true,
        "prerelease": !version.pre.is_empty(),
        "assets": [
            {
                "id": RELEASE_ID * 10 + 1,
                "name": "dist-manifest.json",
                "state": "uploaded",
                "size": manifest.len(),
                "digest": format!("sha256:{}", sha256(manifest)),
            },
            {
                "id": RELEASE_ID * 10 + 2,
                "name": archive_name,
                "state": "uploaded",
                "size": archive.len(),
                "digest": format!("sha256:{}", sha256(archive)),
            }
        ]
    })
}

fn injected_release(
    version: &semver::Version,
    release: Value,
    manifest: Vec<u8>,
    archive: Vec<u8>,
) -> impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response> {
    let endpoint = "https://api.github.com/repos/pj-tmt/tmt/releases".to_owned();
    let exact = format!("{endpoint}/tags/v{version}");
    let manifest_url = format!("{endpoint}/assets/{}", RELEASE_ID * 10 + 1);
    let archive_url = format!("{endpoint}/assets/{}", RELEASE_ID * 10 + 2);
    let release_bytes = serde_json::to_vec(&release).unwrap();
    move |url, accept, maximum, deadline| {
        assert!(deadline > Instant::now());
        let (bytes, expected_accept) = match url {
            url if url == exact => (release_bytes.clone(), "application/vnd.github+json"),
            url if url == manifest_url => (manifest.clone(), "application/octet-stream"),
            url if url == archive_url => (archive.clone(), "application/octet-stream"),
            _ => panic!("unexpected canonical release URL: {url}"),
        };
        assert_eq!(accept, expected_accept);
        assert!(bytes.len() <= maximum);
        Ok(bytes.into())
    }
}

// The prior release chooses its own historical directory. Do not infer that
// name from the candidate catalog or a version threshold.
fn skill_name(bytes: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(bytes).ok()?;
    let (header, _) = text.strip_prefix("---\n")?.split_once("\n---\n")?;
    let name = header
        .lines()
        .find_map(|line| line.strip_prefix("name: "))?;
    matches!(name, "tmux-team" | "tmt").then_some(name)
}

fn skill_target(root: &Path, skill: &[u8]) -> PathBuf {
    root.join(skill_name(skill).expect("real CLI skill must have a known main-skill name"))
}

#[test]
fn upgrade_fixture_targets_follow_each_executable_skill_name() {
    let root = Path::new("/private/skills");
    for name in ["tmux-team", "tmt"] {
        let skill = format!("---\nname: {name}\ndescription: fixture\n---\nbody\n");
        assert_eq!(skill_target(root, skill.as_bytes()), root.join(name));
    }
    for unknown in [
        "---\nname: other\n---\nbody\n",
        "body\nname: tmt\n",
        "---\ndescription: fixture\n---\nname: tmt\n",
    ] {
        assert_eq!(skill_name(unknown.as_bytes()), None);
    }
}

fn report_contains(report: &Value, key: &str, path: &Path) -> bool {
    report[key].as_array().is_some_and(|entries| {
        entries.iter().any(|entry| {
            let target = if key == "refreshed" {
                entry["target"].as_str()
            } else {
                entry.as_str()
            };
            target == path.to_str()
        })
    })
}

#[test]
#[ignore = "requires TMT_UPGRADE_* paths and real versioned matching-host binaries"]
fn cargo_dist_upgrade_refreshes_real_artifacts_and_preserves_conflicts() {
    let old_archive_path = required_path("TMT_UPGRADE_OLD_ARCHIVE");
    let old_manifest_path = required_path("TMT_UPGRADE_OLD_MANIFEST");
    let new_archive_path = required_path("TMT_UPGRADE_NEW_ARCHIVE");
    let new_manifest_path = required_path("TMT_UPGRADE_NEW_MANIFEST");
    let target = env::var("TMT_UPGRADE_TARGET")
        .unwrap_or_else(|_| panic!("explicit acceptance test requires TMT_UPGRADE_TARGET"));
    let directory = TestDirectory::new();
    let task = directory.path.join(TASK_ROOT);
    fs::create_dir(&task).unwrap();

    let new_archive = fs::read(&new_archive_path).unwrap();
    let new_manifest = fs::read(&new_manifest_path).unwrap();
    let old_artifact = artifact::acquire(&old_manifest_path, &old_archive_path, &target).unwrap();
    let new_artifact = artifact::acquire(&new_manifest_path, &new_archive_path, &target).unwrap();
    assert!(new_artifact.version > old_artifact.version);
    assert!(old_artifact.files["tmt"] != new_artifact.files["tmt"]);
    let channel = if old_artifact.version.pre.is_empty() {
        Channel::Stable
    } else {
        Channel::Alpha
    };
    assert!(channel.accepts(&new_artifact.version));

    let prefix = directory.path.join("prefix");
    let initial = install_observed(
        InstallRequest {
            archive: &old_archive_path,
            manifest: &old_manifest_path,
            prefix: &prefix,
            target: &target,
            channel,
            pin: PinAction::Preserve,
        },
        None,
        None,
        || Ok(()),
    )
    .unwrap();
    let old_executable = initial.active_executable.clone();
    let old_version = run(&old_executable, &command_args(&["--version"]), &task)
        .unwrap()
        .stdout;
    assert_eq!(
        String::from_utf8(old_version).unwrap().trim(),
        old_artifact.version.to_string()
    );
    let old_skill = run(&old_executable, &command_args(&["learn", "--skill"]), &task)
        .unwrap()
        .stdout;

    let first_root = task.join("first-skills");
    run(
        &old_executable,
        &[
            OsString::from("install"),
            OsString::from("--dir"),
            first_root.as_os_str().to_os_string(),
        ],
        &task,
    )
    .unwrap();
    let first_target = skill_target(&first_root, &old_skill);
    assert_eq!(fs::read(first_target.join("SKILL.md")).unwrap(), old_skill);

    let second_root = task.join("second-skills");
    run(
        &old_executable,
        &[
            OsString::from("install"),
            OsString::from("--dir"),
            second_root.as_os_str().to_os_string(),
        ],
        &task,
    )
    .unwrap();
    let second_target = skill_target(&second_root, &old_skill);

    let release = release_document(
        &new_artifact.version,
        &new_artifact.name,
        &new_manifest,
        &new_archive,
    );
    let new_version = new_artifact.version.to_string();
    let report = upgrade_product_with(
        super::super::Product::Cli,
        UpgradeRequest {
            executable: &old_executable,
            channel: Some(channel),
            exact: Some(&new_version),
            unpin: false,
        },
        None,
        None,
        || Ok(()),
        injected_release(
            &new_artifact.version,
            release,
            new_manifest.clone(),
            new_archive,
        ),
        install_cli,
    )
    .unwrap();
    assert!(report.installation.changed);
    assert_eq!(report.state.version, new_artifact.version);
    assert_eq!(report.installation.version, new_version);

    let layout = Layout::existing(&prefix).unwrap();
    let receipt = layout.current().unwrap().unwrap();
    let provenance = receipt.provenance.as_ref().unwrap().release().unwrap();
    assert_eq!(provenance.release_id, RELEASE_ID);
    assert_eq!(provenance.manifest_sha256, sha256(&new_manifest));
    assert_eq!(receipt.target, target);
    assert_eq!(receipt.state.version, new_artifact.version);
    assert!(
        fs::read_dir(prefix.join("lib/tmux-team"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".download-"))
    );
    assert!(!task.join("tmux-team.db").exists());

    let new_skill = run(
        &report.installation.active_executable,
        &command_args(&["learn", "--skill"]),
        &task,
    )
    .unwrap()
    .stdout;
    assert_eq!(skill_name(&new_skill), Some(tmt_core::skill_catalog::MAIN));
    let refreshed_first = skill_target(&first_root, &new_skill);
    let refreshed_second = skill_target(&second_root, &new_skill);
    assert_eq!(fs::read(first_target.join("SKILL.md")).unwrap(), old_skill);
    // The support floor used a retired name whose user content remains protected.
    // Migrate that root before occupying the current bundled name under test.
    if second_target != refreshed_second {
        run(
            &report.installation.active_executable,
            &[
                OsString::from("install"),
                OsString::from("--dir"),
                second_root.as_os_str().to_os_string(),
            ],
            &task,
        )
        .unwrap();
    }
    fs::remove_file(&refreshed_second).unwrap();
    let mut user_skill = old_skill.clone();
    user_skill.extend_from_slice(b"\nuser conflict\n");
    fs::create_dir(&refreshed_second).unwrap();
    fs::write(refreshed_second.join("SKILL.md"), &user_skill).unwrap();
    let skill_text_changed = new_skill != old_skill;

    let refresh = run(
        &report.installation.active_executable,
        &command_args(&["__native-refresh-skills", "--json"]),
        &task,
    )
    .unwrap();
    let refresh_document: Value = serde_json::from_slice(&refresh.stdout).unwrap();
    assert!(refresh_document["error"].is_null());
    assert_eq!(refresh_document["conflicts"], json!([]));
    for target in [&refreshed_first, &refreshed_second] {
        assert!(report_contains(&refresh_document, "refreshed", target));
        assert!(fs::symlink_metadata(target).unwrap().is_symlink());
        assert_eq!(fs::read(target.join("SKILL.md")).unwrap(), new_skill);
    }
    let new_source = fs::read_link(&refreshed_first).unwrap();
    assert_eq!(fs::read_link(&refreshed_second).unwrap(), new_source);
    assert!(!task.join(".tmt-skill-backups").exists());
    if first_target != refreshed_first {
        assert!(!first_target.exists());
    }
    if second_target != refreshed_second {
        assert!(!second_target.exists());
    }

    let old_still_runs = run(&old_executable, &command_args(&["--version"]), &task)
        .unwrap()
        .stdout;
    assert_eq!(
        String::from_utf8(old_still_runs).unwrap().trim(),
        old_artifact.version.to_string()
    );

    let retry = run(
        &report.installation.active_executable,
        &command_args(&["__native-refresh-skills", "--json"]),
        &task,
    )
    .unwrap();
    let retry_document: Value = serde_json::from_slice(&retry.stdout).unwrap();
    assert!(retry_document["error"].is_null());
    assert_eq!(retry_document["conflicts"], json!([]));
    assert!(
        retry_document["refreshed"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["changed"] == false)
    );
    for target in [&refreshed_first, &refreshed_second] {
        assert!(report_contains(&retry_document, "refreshed", target));
        assert_eq!(fs::read_link(target).unwrap(), new_source);
        assert_eq!(fs::read(target.join("SKILL.md")).unwrap(), new_skill);
    }
    assert!(!task.join(".tmt-skill-backups").exists());
    if skill_text_changed {
        println!("skill-content transition: passed (old and candidate text differ)");
    } else {
        println!(
            "skill-content transition: skipped (old and candidate text are identical); \
             candidate-byte equality, bundled replacement and idempotence passed"
        );
    }
}
