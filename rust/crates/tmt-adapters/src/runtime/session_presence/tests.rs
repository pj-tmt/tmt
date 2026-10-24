use super::*;
use crate::test_support::TestDirectory;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    time::Duration,
};

const SESSION: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const OTHER: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

fn claude_name(name: &str, session: &str) -> bool {
    name.strip_suffix(".jsonl") == Some(session)
}

fn session() -> ProviderSessionId {
    ProviderSessionId::new(SESSION).unwrap()
}

fn probe_claude(root: &Path) -> SessionPresence {
    probe_with(
        root,
        Instant::now() + Duration::from_secs(5),
        PRESENCE_ENTRIES,
    )
}

fn probe_with(root: &Path, deadline: Instant, entries: usize) -> SessionPresence {
    let layout = Layout {
        root,
        depth: 1,
        names: claude_name,
    };
    probe(&layout, &session(), deadline, entries)
}

fn project(directory: &TestDirectory) -> PathBuf {
    let project = directory.path.join("projects/-work");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join(format!("{OTHER}.jsonl")), b"{}\n").unwrap();
    project
}

#[test]
fn a_regular_session_file_is_present() {
    let directory = TestDirectory::new();
    let project = project(&directory);
    fs::write(project.join(format!("{SESSION}.jsonl")), b"{}\n").unwrap();
    assert_eq!(
        probe_claude(&directory.path.join("projects")),
        SessionPresence::Present
    );
}

#[test]
fn a_fully_read_tree_without_the_session_is_missing() {
    let directory = TestDirectory::new();
    let project = project(&directory);
    // A file deeper than the layout (a subagent transcript) is not the session.
    fs::create_dir_all(project.join(OTHER).join("subagents")).unwrap();
    fs::write(
        project
            .join(OTHER)
            .join(format!("subagents/{SESSION}.jsonl")),
        b"{}\n",
    )
    .unwrap();
    assert_eq!(
        probe_claude(&directory.path.join("projects")),
        SessionPresence::Missing
    );
}

#[test]
fn a_session_file_tmt_may_not_read_is_unreadable() {
    let directory = TestDirectory::new();
    let project = project(&directory);
    let file = project.join(format!("{SESSION}.jsonl"));
    fs::write(&file, b"{}\n").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
    let presence = probe_claude(&directory.path.join("projects"));
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(presence, SessionPresence::Unreadable);
}

#[test]
fn a_symlink_or_fifo_named_for_the_session_is_unreadable() {
    let directory = TestDirectory::new();
    let project = project(&directory);
    let outside = directory.path.join("outside.jsonl");
    fs::write(&outside, b"{}\n").unwrap();
    symlink(&outside, project.join(format!("{SESSION}.jsonl"))).unwrap();
    assert_eq!(
        probe_claude(&directory.path.join("projects")),
        SessionPresence::Unreadable
    );
    fs::remove_file(project.join(format!("{SESSION}.jsonl"))).unwrap();
    nix::unistd::mkfifo(
        &project.join(format!("{SESSION}.jsonl")),
        nix::sys::stat::Mode::S_IRWXU,
    )
    .unwrap();
    assert_eq!(
        probe_claude(&directory.path.join("projects")),
        SessionPresence::Unreadable
    );
}

#[test]
fn doubt_is_unknown_never_missing() {
    let directory = TestDirectory::new();
    // No provider tree at all: another config root may hold the session.
    assert_eq!(
        probe_claude(&directory.path.join("projects")),
        SessionPresence::Unknown
    );
    let project = project(&directory);
    let root = directory.path.join("projects");
    assert_eq!(
        probe_with(&root, Instant::now(), PRESENCE_ENTRIES),
        SessionPresence::Unknown
    );
    assert_eq!(
        probe_with(&root, Instant::now() + Duration::from_secs(5), 1),
        SessionPresence::Unknown
    );
    // A directory that cannot be listed leaves the scan incomplete.
    fs::set_permissions(&project, fs::Permissions::from_mode(0o000)).unwrap();
    let presence = probe_claude(&root);
    fs::set_permissions(&project, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(presence, SessionPresence::Unknown);
    // Only an exact UUID session is looked up.
    let layout = Layout {
        root: &root,
        depth: 1,
        names: claude_name,
    };
    assert_eq!(
        probe(
            &layout,
            &ProviderSessionId::new("../etc/passwd").unwrap(),
            Instant::now() + Duration::from_secs(5),
            PRESENCE_ENTRIES
        ),
        SessionPresence::Unknown
    );
}

fn remembered(harness: &str, mode: &str) -> tmt_core::binding::session::RememberedSession {
    use tmt_core::binding::session::{HarnessId, RememberedSession, RuntimeMode};
    RememberedSession {
        harness: HarnessId::new(harness).unwrap(),
        mode: RuntimeMode::new(mode).unwrap(),
        provider_session: session(),
        state: None,
        stale_at_ms: None,
        resume_pending_at_ms: None,
    }
}

#[test]
fn first_party_drivers_look_only_in_their_own_private_tree() {
    use crate::{runtime::RuntimeRegistry, skill_installation::ProviderEnvironment};
    let directory = TestDirectory::new();
    let home = directory.path.join("home");
    let environment = ProviderEnvironment::from_parts(&home, &directory.path, Vec::new(), []);
    let registry = RuntimeRegistry::first_party();
    let claude = remembered("claude", crate::drivers::claude::MODE_DEFAULT);
    let codex = remembered("codex", crate::drivers::codex::MODE_EMBEDDED);
    let agy = remembered("agy", crate::drivers::agy::MODE_DEFAULT);
    for session in [&claude, &codex, &agy] {
        assert_eq!(
            registry.session_presence(session, &environment),
            SessionPresence::Unknown,
            "no provider tree yet"
        );
    }
    let project = home.join(".claude/projects/-work");
    fs::create_dir_all(&project).unwrap();
    let day = home.join(".codex/sessions/2026/10/23");
    fs::create_dir_all(&day).unwrap();
    let brain = home.join(".gemini/antigravity-cli/brain");
    fs::create_dir_all(&brain).unwrap();
    assert_eq!(
        registry.session_presence(&claude, &environment),
        SessionPresence::Missing
    );
    assert_eq!(
        registry.session_presence(&codex, &environment),
        SessionPresence::Missing
    );
    assert_eq!(
        registry.session_presence(&agy, &environment),
        SessionPresence::Missing
    );
    // Each driver accepts only its own file name for the session.
    fs::write(
        project.join(format!("rollout-2026-10-23T00-00-00-{SESSION}.jsonl")),
        b"{}\n",
    )
    .unwrap();
    fs::write(day.join(format!("{SESSION}.jsonl")), b"{}\n").unwrap();
    assert_eq!(
        registry.session_presence(&claude, &environment),
        SessionPresence::Missing
    );
    assert_eq!(
        registry.session_presence(&codex, &environment),
        SessionPresence::Missing
    );
    fs::write(project.join(format!("{SESSION}.jsonl")), b"{}\n").unwrap();
    fs::write(
        day.join(format!("rollout-2026-10-23T00-00-00-{SESSION}.jsonl")),
        b"{}\n",
    )
    .unwrap();
    let agy_logs = brain.join(SESSION).join(".system_generated/logs");
    fs::create_dir_all(&agy_logs).unwrap();
    fs::write(agy_logs.join("transcript.jsonl"), b"{}\n").unwrap();
    assert_eq!(
        registry.session_presence(&claude, &environment),
        SessionPresence::Present
    );
    assert_eq!(
        registry.session_presence(&codex, &environment),
        SessionPresence::Present
    );
    assert_eq!(
        registry.session_presence(&agy, &environment),
        SessionPresence::Present
    );
    let other = remembered("gemini", "default");
    assert_eq!(
        registry.session_presence(&other, &environment),
        SessionPresence::Unknown
    );
}

#[test]
fn a_symlinked_project_directory_is_doubt_not_absence() {
    let directory = TestDirectory::new();
    let elsewhere = directory.path.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(elsewhere.join(format!("{SESSION}.jsonl")), b"{}\n").unwrap();
    let root = directory.path.join("projects");
    fs::create_dir_all(&root).unwrap();
    // Not followed, but the session may live behind it.
    symlink(&elsewhere, root.join("-linked")).unwrap();
    assert_eq!(probe_claude(&root), SessionPresence::Unknown);
}

#[test]
fn a_file_that_opens_but_fails_the_transcript_boundary_is_doubt() {
    let directory = TestDirectory::new();
    let project = project(&directory);
    let file = project.join(format!("{SESSION}.jsonl"));
    fs::write(&file, b"{}\n").unwrap();
    // The boundary also fails on races such as a rotated file; that is doubt.
    let other_root = directory.path.join("other");
    fs::create_dir_all(&other_root).unwrap();
    assert_eq!(admit(&other_root, &file), SessionPresence::Unknown);
    assert_eq!(
        admit(&directory.path.join("projects"), &file),
        SessionPresence::Present
    );
}

#[test]
fn an_archived_codex_thread_is_doubt_not_gone() {
    use crate::{runtime::RuntimeRegistry, skill_installation::ProviderEnvironment};
    let directory = TestDirectory::new();
    let home = directory.path.join("home");
    let environment = ProviderEnvironment::from_parts(&home, &directory.path, Vec::new(), []);
    let registry = RuntimeRegistry::first_party();
    let codex = remembered("codex", crate::drivers::codex::MODE_EMBEDDED);
    fs::create_dir_all(home.join(".codex/sessions/2026/10/23")).unwrap();
    let archived = home.join(".codex/archived_sessions");
    fs::create_dir_all(&archived).unwrap();
    fs::write(
        archived.join(format!("rollout-2026-10-01T00-00-00-{OTHER}.jsonl")),
        b"{}\n",
    )
    .unwrap();
    assert_eq!(
        registry.session_presence(&codex, &environment),
        SessionPresence::Missing
    );
    // Codex can unarchive a thread, so an archived one is not gone.
    fs::write(
        archived.join(format!("rollout-2026-10-01T00-00-00-{SESSION}.jsonl")),
        b"{}\n",
    )
    .unwrap();
    assert_eq!(
        registry.session_presence(&codex, &environment),
        SessionPresence::Unknown
    );
}
