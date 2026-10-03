use super::*;
use crate::test_support::TestDirectory;

fn environment(root: &Path, override_home: Option<&Path>) -> ProviderEnvironment {
    ProviderEnvironment::from_parts(
        root,
        root,
        Vec::new(),
        override_home.map(|home| ("CODEX_HOME", home.to_path_buf())),
    )
}

fn config(home: &Path, paths: &[(&Path, &str)]) {
    fs::create_dir_all(home).unwrap();
    let mut document = DocumentMut::new();
    for (path, level) in paths {
        document["projects"][path.to_str().unwrap()]["trust_level"] = toml_edit::value(*level);
    }
    fs::write(home.join("config.toml"), document.to_string()).unwrap();
}

#[test]
fn exact_cwd_masks_repository_root_and_parents_do_not_inherit() {
    let fixture = TestDirectory::new();
    let home = fixture.path.join(".codex");
    let repo = fixture.path.join("repo");
    let cwd = repo.join("sub");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::create_dir(&cwd).unwrap();
    let env = environment(&fixture.path, None);
    config(&home, &[(&repo, "trusted")]);
    assert_eq!(local_project_trust(&env, &cwd), LocalProjectTrust::Trusted);
    config(&home, &[(&repo, "trusted"), (&cwd, "untrusted")]);
    assert_eq!(
        local_project_trust(&env, &cwd),
        LocalProjectTrust::Untrusted
    );
    let mut document = fs::read_to_string(home.join("config.toml"))
        .unwrap()
        .parse::<DocumentMut>()
        .unwrap();
    document["projects"][cwd.to_str().unwrap()]
        .as_table_like_mut()
        .unwrap()
        .remove("trust_level");
    fs::write(home.join("config.toml"), document.to_string()).unwrap();
    assert_eq!(local_project_trust(&env, &cwd), LocalProjectTrust::Unset);
    config(&home, &[(&fixture.path, "trusted")]);
    assert_eq!(local_project_trust(&env, &cwd), LocalProjectTrust::Unset);
}

#[test]
fn alternate_home_uses_invocation_directory_and_never_changes_files() {
    let fixture = TestDirectory::new();
    let default = fixture.path.join(".codex");
    let alternate = fixture.path.join("alternate");
    config(&default, &[(&fixture.path, "trusted")]);
    config(&alternate, &[(&fixture.path, "untrusted")]);
    fs::write(alternate.join("auth.json"), "credential sentinel").unwrap();
    let before = fs::read(alternate.join("config.toml")).unwrap();
    let modified = fs::metadata(alternate.join("config.toml"))
        .unwrap()
        .modified()
        .unwrap();
    let names = || {
        let mut entries: Vec<_> = fs::read_dir(&alternate)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        entries.sort();
        entries
    };
    let before_names = names();
    assert_eq!(
        local_project_trust(
            &environment(&fixture.path, Some(Path::new("alternate"))),
            &fixture.path
        ),
        LocalProjectTrust::Untrusted
    );
    assert_eq!(fs::read(alternate.join("config.toml")).unwrap(), before);
    assert_eq!(
        fs::metadata(alternate.join("config.toml"))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    assert_eq!(
        fs::read_to_string(alternate.join("auth.json")).unwrap(),
        "credential sentinel"
    );
    assert_eq!(names(), before_names);
    assert_eq!(
        local_project_trust(&environment(&fixture.path, None), &fixture.path),
        LocalProjectTrust::Trusted
    );
}

#[test]
fn oversized_and_unreadable_config_remain_unchanged() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = TestDirectory::new();
    let home = fixture.path.join(".codex");
    fs::create_dir(&home).unwrap();
    let path = home.join("config.toml");
    let oversized = vec![b' '; CONFIG_LIMIT + 1];
    fs::write(&path, &oversized).unwrap();
    let env = environment(&fixture.path, None);
    assert_eq!(
        local_project_trust(&env, &fixture.path),
        LocalProjectTrust::Unknown
    );
    assert_eq!(fs::read(&path).unwrap(), oversized);
    fs::write(&path, "# unreadable fixture\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    let result = local_project_trust(&env, &fixture.path);
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(mode, 0);
    assert_eq!(result, LocalProjectTrust::Unknown);
    assert_eq!(fs::read_to_string(&path).unwrap(), "# unreadable fixture\n");
}

#[test]
fn missing_home_is_not_created_and_unsupported_inputs_are_unknown() {
    let fixture = TestDirectory::new();
    let home = fixture.path.join(".codex");
    let env = environment(&fixture.path, None);
    assert_eq!(
        local_project_trust(&env, &fixture.path),
        LocalProjectTrust::Unset
    );
    assert!(!home.exists());
    fs::create_dir(&home).unwrap();
    for contents in ["[invalid", "projects = 3", "projects = []"] {
        fs::write(home.join("config.toml"), contents).unwrap();
        assert_eq!(
            local_project_trust(&env, &fixture.path),
            LocalProjectTrust::Unknown
        );
        assert_eq!(
            fs::read_to_string(home.join("config.toml")).unwrap(),
            contents
        );
    }
    fs::remove_file(home.join("config.toml")).unwrap();
    fs::create_dir(home.join("config.toml")).unwrap();
    assert_eq!(
        local_project_trust(&env, &fixture.path),
        LocalProjectTrust::Unknown
    );
    fs::remove_dir(home.join("config.toml")).unwrap();
    fs::write(fixture.path.join(".git"), "gitdir: elsewhere\n").unwrap();
    assert_eq!(
        local_project_trust(&env, &fixture.path),
        LocalProjectTrust::Unknown
    );
}

#[test]
fn canonical_cwd_precedes_original_and_symlink_config_is_unknown() {
    let fixture = TestDirectory::new();
    let cwd = fixture.path.join("cwd");
    let alias = fixture.path.join("alias");
    let home = fixture.path.join(".codex");
    fs::create_dir(&cwd).unwrap();
    std::os::unix::fs::symlink(&cwd, &alias).unwrap();
    config(
        &home,
        &[
            (&fs::canonicalize(&cwd).unwrap(), "trusted"),
            (&alias, "untrusted"),
        ],
    );
    let env = environment(&fixture.path, None);
    assert_eq!(
        local_project_trust(&env, &alias),
        LocalProjectTrust::Trusted
    );
    fs::rename(home.join("config.toml"), home.join("target.toml")).unwrap();
    std::os::unix::fs::symlink("target.toml", home.join("config.toml")).unwrap();
    let before = fs::read(home.join("target.toml")).unwrap();
    assert_eq!(local_project_trust(&env, &cwd), LocalProjectTrust::Unknown);
    assert_eq!(fs::read(home.join("target.toml")).unwrap(), before);
}
