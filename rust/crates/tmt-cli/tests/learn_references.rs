//! `tmt learn` serves the skill entry and its references from the binary alone:
//! a copy outside the checkout prints the exact authored bytes, offline, with
//! no storage, host or provider setup.
#![cfg(unix)]

#[allow(dead_code)]
mod support;

use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
};
use tmt_core::skill_catalog::{BUNDLED, MAIN};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct World {
    root: PathBuf,
    binary: PathBuf,
}

impl World {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-learn-references-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let binary = root.join("bin/tmt");
        fs::create_dir(root.join("bin")).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_tmt"), &binary).unwrap();
        Self { root, binary }
    }

    fn learn(&self, args: &[&str]) -> Output {
        let mut command = support::command_for(&self.binary, &self.root, &[]);
        command.arg("learn").args(args);
        command.output().unwrap()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn source(path: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../skills")
            .join(path),
    )
    .unwrap()
}

fn printed(output: &Output) -> &[u8] {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    &output.stdout
}

#[test]
fn a_moved_binary_prints_each_entry_and_reference_exactly() {
    let world = World::new();
    for skill in &BUNDLED {
        let entry = source(&format!("{}/SKILL.md", skill.name));
        assert_eq!(printed(&world.learn(&["--skill", skill.name])), entry);
        for topic in skill.topics {
            assert_eq!(
                printed(&world.learn(&["--skill", skill.name, topic])),
                source(&format!("{}/references/{topic}.md", skill.name)),
                "{}/{topic}",
                skill.name
            );
        }
        let list = world.learn(&["--skill", skill.name, "--topics"]);
        if skill.topics.is_empty() {
            assert_ne!(list.status.code(), Some(0));
            assert!(list.stdout.is_empty());
        } else {
            let expected: String = skill
                .topics
                .iter()
                .map(|topic| format!("{topic}\n"))
                .collect();
            assert_eq!(printed(&list), expected.as_bytes());
        }
    }
    assert_eq!(
        printed(&world.learn(&["--skill"])),
        source(&format!("{MAIN}/SKILL.md"))
    );
    // Reading guidance creates no storage and installs nothing.
    assert!(!support::state_dir(&world.root).exists());
    assert!(!world.root.join("home/.agents").exists());
    assert!(!world.root.join("home/.claude").exists());
}

#[test]
fn an_unknown_topic_prints_nothing_and_names_the_catalog_topics() {
    let world = World::new();
    let output = world.learn(&["--skill", MAIN, "no-such-topic"]);
    assert_ne!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let topics = tmt_core::skill_catalog::topics(MAIN).unwrap();
    if topics.is_empty() {
        assert!(
            stderr.contains("The tmt skill has no reference topics"),
            "{stderr}"
        );
    } else {
        assert!(
            stderr.contains(&format!(
                "Unknown tmt reference topic 'no-such-topic'. Topics: {}",
                topics.join(", ")
            )),
            "{stderr}"
        );
    }
    assert!(!support::state_dir(&world.root).exists());
}
