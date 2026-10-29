//! Where Claude Code reads invokable skills (#5531): the config root's
//! `skills`, each installed plugin's `skills`, and the project's
//! `.claude/skills`. Plugin skills are invoked as `plugin:name`.
use crate::runtime::skills::{InvocationSyntax, SkillRoot, SkillRoots, SkillSource};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const PLUGIN_INDEX: &str = "plugins/installed_plugins.json";
const PLUGIN_INDEX_LIMIT: usize = 1024 * 1024;

/// Only the install paths are read from the plugin index.
#[derive(Deserialize)]
struct Index {
    plugins: BTreeMap<String, Vec<Install>>,
}

#[derive(Deserialize)]
struct Install {
    #[serde(rename = "installPath")]
    install_path: Option<PathBuf>,
}

pub(super) fn roots(config_root: &Path, cwd: Option<&Path>) -> SkillRoots {
    let mut roots = vec![SkillRoot {
        path: config_root.join("skills"),
        source: SkillSource::User,
    }];
    roots.extend(plugins(config_root));
    if let Some(cwd) = cwd {
        roots.push(SkillRoot {
            path: cwd.join(".claude/skills"),
            source: SkillSource::Project,
        });
    }
    SkillRoots {
        syntax: InvocationSyntax::Prefix { sigil: '/' },
        roots,
    }
}

/// Each plugin whose install path resolves inside `<config>/plugins`; the
/// index key is `name@marketplace`, and its name must be one composer token.
fn plugins(config_root: &Path) -> Vec<SkillRoot> {
    let Ok(plugins_root) = std::fs::canonicalize(config_root.join("plugins")) else {
        return Vec::new();
    };
    let Ok(bytes) =
        crate::bounded_file::read_no_follow(&config_root.join(PLUGIN_INDEX), PLUGIN_INDEX_LIMIT)
    else {
        return Vec::new();
    };
    let Ok(index) = serde_json::from_slice::<Index>(&bytes) else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    for (key, installs) in index.plugins {
        let plugin = key.split('@').next().unwrap_or_default();
        if !crate::skill_installation::valid_skill_name(plugin) {
            continue;
        }
        for install in installs {
            let Some(path) = install
                .install_path
                .and_then(|path| std::fs::canonicalize(path).ok())
                .filter(|path| path.starts_with(&plugins_root))
            else {
                continue;
            };
            roots.push(SkillRoot {
                path: path.join("skills"),
                source: SkillSource::Plugin {
                    plugin: plugin.to_owned(),
                },
            });
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use std::fs;

    #[test]
    fn plugin_roots_stay_inside_the_config_plugins_directory() {
        let root = TestDirectory::new();
        assert!(root.path.starts_with(std::env::temp_dir()));
        let config = root.path.join("claude");
        let inside = config.join("plugins/cache/kit/1.0");
        let outside = root.path.join("elsewhere");
        fs::create_dir_all(&inside).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let index = serde_json::json!({"version": 2, "plugins": {
            "kit@market": [{"installPath": inside, "scope": "user"}],
            "escape@market": [{"installPath": outside}],
            "Bad Name@market": [{"installPath": inside}],
            "missing@market": [{"scope": "user"}],
        }});
        fs::write(config.join(PLUGIN_INDEX), index.to_string()).unwrap();
        let cwd = root.path.join("project");
        let found = roots(&config, Some(&cwd));
        assert_eq!(found.syntax, InvocationSyntax::Prefix { sigil: '/' });
        let inside = fs::canonicalize(&inside).unwrap();
        assert_eq!(
            found.roots,
            [
                SkillRoot {
                    path: config.join("skills"),
                    source: SkillSource::User
                },
                SkillRoot {
                    path: inside.join("skills"),
                    source: SkillSource::Plugin {
                        plugin: "kit".into()
                    }
                },
                SkillRoot {
                    path: cwd.join(".claude/skills"),
                    source: SkillSource::Project
                },
            ]
        );
        // Without a cwd there is no project root; a linked index is not read.
        let index_path = config.join(PLUGIN_INDEX);
        let moved = root.path.join("index.json");
        fs::rename(&index_path, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &index_path).unwrap();
        assert_eq!(
            roots(&config, None).roots,
            [SkillRoot {
                path: config.join("skills"),
                source: SkillSource::User
            }]
        );
    }
}
