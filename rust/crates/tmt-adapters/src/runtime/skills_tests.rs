use super::*;
use crate::test_support::TestDirectory;
use std::os::unix::fs::{PermissionsExt, symlink};

struct World {
    _root: TestDirectory,
    assets: PathBuf,
    user: PathBuf,
    plugin: PathBuf,
    project: PathBuf,
    outside: PathBuf,
}

impl World {
    fn new() -> Self {
        let root = TestDirectory::new();
        // Private roots only, never a provider's real home.
        assert!(root.path.starts_with(std::env::temp_dir()));
        let path = |name: &str| {
            let path = root.path.join(name);
            fs::create_dir_all(&path).unwrap();
            path
        };
        Self {
            assets: path("tmt/skill-assets"),
            user: path("user"),
            plugin: path("plugin"),
            project: path("project"),
            outside: path("outside"),
            _root: root,
        }
    }

    fn roots(&self) -> SkillRoots {
        SkillRoots {
            syntax: InvocationSyntax::Prefix { sigil: '/' },
            roots: vec![
                SkillRoot {
                    path: self.user.clone(),
                    source: SkillSource::User,
                },
                SkillRoot {
                    path: self.plugin.clone(),
                    source: SkillSource::Plugin {
                        plugin: "kit".into(),
                    },
                },
                SkillRoot {
                    path: self.project.clone(),
                    source: SkillSource::Project,
                },
            ],
        }
    }
}

fn skill(directory: &Path, name: &str, front: &str) -> PathBuf {
    let path = directory.join(name);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join(SKILL_FILE), format!("---\n{front}\n---\nBody\n")).unwrap();
    path
}

fn names(list: &SkillList) -> Vec<(&str, &SkillSource)> {
    list.skills
        .iter()
        .map(|skill| (skill.name.as_str(), &skill.source))
        .collect()
}

#[test]
fn each_group_is_listed_sorted_with_one_line_descriptions() {
    let world = World::new();
    skill(
        &world.user,
        "review",
        "name: review\ndescription: \"Review\tthe\u{7}  diff\"",
    );
    skill(&world.plugin, "deploy", "description: Ship it");
    skill(&world.project, "local", "name: local");
    let published = skill(&world.assets, "tmt", "name: tmt\ndescription: Talk");
    symlink(&published, world.user.join("tmt")).unwrap();
    let list = list(&world.roots(), &world.assets);
    assert!(!list.truncated);
    assert_eq!(
        names(&list),
        [
            ("tmt", &SkillSource::Tmt),
            ("review", &SkillSource::User),
            (
                "kit:deploy",
                &SkillSource::Plugin {
                    plugin: "kit".into()
                }
            ),
            ("local", &SkillSource::Project),
        ]
    );
    assert_eq!(list.skills[1].description, "Review the diff");
    assert_eq!(list.skills[3].description, "");
    let long = "x".repeat(DESCRIPTION_LIMIT + 50);
    skill(&world.user, "long", &format!("description: {long}"));
    let list = super::list(&world.roots(), &world.assets);
    let long = list.skills.iter().find(|s| s.name == "long").unwrap();
    assert_eq!(long.description.chars().count(), DESCRIPTION_LIMIT);
}

#[test]
fn untrusted_or_unreadable_skills_are_dropped() {
    let world = World::new();
    skill(&world.user, "good", "name: good");
    // Names a composer cannot insert as one token.
    skill(&world.user, "spaced", "name: two words");
    skill(&world.user, "Upper", "description: no name field");
    skill(&world.user, "slash", "name: a/b");
    // No closed front matter within the read bound.
    let unclosed = world.user.join("unclosed");
    fs::create_dir_all(&unclosed).unwrap();
    let mut text = "---\nname: unclosed\n".to_owned();
    text.push_str(&"#\n".repeat(FRONT_MATTER_LIMIT as usize));
    text.push_str("---\n");
    fs::write(unclosed.join(SKILL_FILE), text).unwrap();
    // A linked skill file and a link out of the root are never followed.
    let elsewhere = skill(&world.outside, "escape", "name: escape");
    symlink(&elsewhere, world.user.join("escape")).unwrap();
    let linked = world.user.join("linked");
    fs::create_dir_all(&linked).unwrap();
    symlink(elsewhere.join(SKILL_FILE), linked.join(SKILL_FILE)).unwrap();
    // A plain file and a directory without a skill file.
    fs::write(world.user.join("notes.md"), "x").unwrap();
    fs::create_dir_all(world.user.join("empty")).unwrap();
    let list = list(&world.roots(), &world.assets);
    assert_eq!(names(&list), [("good", &SkillSource::User)]);
}

#[test]
fn a_linked_root_is_not_walked_and_unsupported_lists_nothing() {
    let world = World::new();
    skill(&world.outside, "far", "name: far");
    let linked = world.project.join("linked-root");
    symlink(&world.outside, &linked).unwrap();
    let roots = SkillRoots {
        syntax: InvocationSyntax::Prefix { sigil: '$' },
        roots: vec![SkillRoot {
            path: linked,
            source: SkillSource::Project,
        }],
    };
    assert!(list(&roots, &world.assets).skills.is_empty());
    skill(&world.user, "good", "name: good");
    let mut unsupported = world.roots();
    unsupported.syntax = InvocationSyntax::Unsupported;
    assert!(list(&unsupported, &world.assets).skills.is_empty());
}

#[test]
fn more_skills_than_the_limit_are_truncated() {
    let world = World::new();
    for index in 0..=SKILL_LIMIT {
        skill(&world.user, &format!("s{index:03}"), "");
    }
    let list = list(&world.roots(), &world.assets);
    assert!(list.truncated);
    assert_eq!(list.skills.len(), SKILL_LIMIT);
    assert_eq!(list.skills[0].name, "s000");
}

#[test]
fn the_stamp_follows_the_walked_directories_without_reading_files() {
    let world = World::new();
    let roots = world.roots();
    let empty = stamp(&roots, &world.assets);
    let review = skill(&world.user, "review", "name: review");
    let added = stamp(&roots, &world.assets);
    assert_ne!(added, empty);
    // Stable while nothing changes.
    assert_eq!(stamp(&roots, &world.assets), added);
    // An unreadable skill file changes nothing the stamp sees: it never reads it.
    let file = review.join(SKILL_FILE);
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
    assert_eq!(stamp(&roots, &world.assets), added);
    assert!(list(&roots, &world.assets).skills.is_empty());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    // An edit, a new plugin skill and a removal each change it.
    fs::write(&file, "---\nname: review\ndescription: longer now\n---\n").unwrap();
    let edited = stamp(&roots, &world.assets);
    assert_ne!(edited, added);
    skill(&world.plugin, "deploy", "");
    let plugin = stamp(&roots, &world.assets);
    assert_ne!(plugin, edited);
    fs::remove_dir_all(&review).unwrap();
    assert_ne!(stamp(&roots, &world.assets), plugin);
    // A different project directory is a different set of roots.
    let mut moved = world.roots();
    moved.roots[2].path = world.outside.clone();
    assert_ne!(stamp(&moved, &world.assets), stamp(&roots, &world.assets));
}
