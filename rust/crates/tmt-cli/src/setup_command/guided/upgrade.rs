//! New provider scopes share guided setup's planner and publisher, with explicit human consent.
use super::*;
use serde_json::Value;
use std::{collections::BTreeSet, io, path::Path};
use tmt_core::content_digest::sha256;

pub(crate) fn read(prefix: &Path) -> Vec<Value> {
    match plans(prefix) {
        Ok(plans) => plans.into_iter().map(|(row, _)| row).collect(),
        Err(error) => vec![
            json!({"provider":"providers","status":"needsApproval","selected":false,
            "message":error.message,"hint":"tmt setup"}),
        ],
    }
}

fn plans(prefix: &Path) -> Result<Vec<(Value, Option<Plan>)>, Failure> {
    let env = ProviderEnvironment::capture().map_err(failure)?;
    let global = ConfigPaths::discover().map_err(Failure::from)?.global_dir;
    plans_with(prefix, &env, &global)
}

fn plans_with(
    prefix: &Path,
    env: &ProviderEnvironment,
    global: &Path,
) -> Result<Vec<(Value, Option<Plan>)>, Failure> {
    let drivers = Registry::builtin();
    let recorded = record::read(global).map_err(failure)?;
    let targets = skill_installation::recorded_targets(global).map_err(failure)?;
    let environment = SetupEnvironment {
        launcher: prefix.join("bin/tmt"),
        settings: drivers
            .with_hooks()
            .map(|driver| {
                Ok((
                    driver,
                    env.locations(driver).hook_settings.ok_or_else(|| {
                        failure(io::Error::other("Provider hook settings are unavailable."))
                    })?,
                ))
            })
            .collect::<Result<_, Failure>>()?,
    };
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for (driver, detection) in drivers.detect(env) {
        if !gets_skills(&detection) {
            if let Detection::Broken { reason } = detection {
                rows.push((json!({"provider":driver.name(),"status":"skipped","selected":false,"message":reason,
                    "hint":"tmt setup"}), None));
            }
            continue;
        }
        let hint = if matches!(detection, Detection::Present { .. })
            && driver.descriptor.hooks.is_some()
        {
            format!("tmt setup {}", driver.name())
        } else {
            "tmt setup".into()
        };
        let attempt = (|| {
            let mut plan = plan_for(
                &drivers,
                &environment,
                env,
                global,
                UsageHook::Default,
                Some(driver.name()),
            )?;
            plan.owned.clear(); // Each consented extension owns publication of its bundled skills.
            plan.core
                .retain(|(_, skill)| !targets.contains(&skill.target) && skill.state.changes());
            plan.hooks.retain(|hooks| {
                !recorded.iter().any(|entry| {
                    entry.driver == hooks.provider && entry.settings == hooks.change.path
                })
            });
            if plan
                .core
                .iter()
                .any(|(_, skill)| skill.state != SkillState::Missing)
            {
                return Err(failure(io::Error::other(
                    "An unrecorded provider skill target is occupied; review setup manually.",
                )));
            }
            plan.core
                .retain(|(_, skill)| seen.insert(skill.target.clone()));
            if plan.core.is_empty() && plan.hooks.is_empty() {
                return Ok(None);
            }
            let skills = plan
                .core
                .iter()
                .map(|(_, skill)| json!({"name":skill.name,"path":skill.target}))
                .collect::<Vec<_>>();
            let hooks = plan.hooks.iter().map(|hook| json!({"path":hook.change.path,"launcher":hook.launcher,
                "before":hook.change.before.as_ref().map(|before| sha256(before.as_bytes())),
                "after":sha256(hook.change.after.as_bytes()),"events":hook.events(),"usage":hook.usage})).collect::<Vec<_>>();
            let row = json!({"provider":driver.name(),"status":"pending","selected":true,"skills":skills,"hooks":hooks,
                "hint":hint});
            Ok(Some((row, plan)))
        })();
        match attempt {
            Ok(Some((row, plan))) => rows.push((row, Some(plan))),
            Ok(None) => {}
            Err(error) => rows.push((
                json!({"provider":driver.name(),"status":"needsApproval","selected":false,
                "message":error.message,"hint":hint}),
                None,
            )),
        }
    }
    Ok(rows)
}

/// Capture all plans before the first write, so providers sharing a skill root remain one scope.
pub(crate) fn apply(
    prefix: &Path,
    approved: &[Value],
    checkpoint: &impl Fn() -> bool,
) -> (Vec<Value>, bool) {
    if !approved.iter().any(|row| row["selected"] == true) {
        return (
            approved
                .iter()
                .map(|row| {
                    let mut row = row.clone();
                    if row["status"] == "pending" {
                        row["status"] = "consentRequired".into();
                    }
                    row
                })
                .collect(),
            false,
        );
    }
    let mut current = match plans(prefix) {
        Ok(plans) => plans,
        Err(error) => {
            return (
                approved
                    .iter()
                    .map(|row| failed(row, &error.message))
                    .collect(),
                false,
            );
        }
    };
    let global = match ConfigPaths::discover() {
        Ok(paths) => paths.global_dir,
        Err(error) => {
            return (
                approved
                    .iter()
                    .map(|row| failed(row, &error.to_string()))
                    .collect(),
                false,
            );
        }
    };
    let mut interrupted = false;
    let rows = approved
        .iter()
        .map(|approved| {
            if approved["selected"] != true || approved["status"] != "pending" {
                let mut row = approved.clone();
                if row["status"] == "pending" {
                    row["status"] = "consentRequired".into();
                }
                return row;
            }
            interrupted |= checkpoint();
            if interrupted {
                return failed(
                    approved,
                    "Setup interrupted; no later provider was changed.",
                );
            }
            let found = current
                .iter_mut()
                .find(|(row, _)| row["provider"] == approved["provider"]);
            let Some((row, plan)) = found else {
                return failed(
                    approved,
                    "Provider scope changed after approval; review setup again.",
                );
            };
            if !same_scope(row, approved) {
                return failed(
                    approved,
                    "Provider scope changed after approval; review setup again.",
                );
            }
            let Some(plan) = plan.take() else {
                return failed(approved, "Provider setup needs separate approval.");
            };
            match apply_with_policy(
                &plan,
                &global,
                &mut Vec::new(),
                Terminal::PLAIN,
                OutputMode { json: true },
                true,
            ) {
                Ok(report) if report.failures.is_empty() && report.skipped.is_empty() => {
                    let mut row = approved.clone();
                    row["status"] = "changed".into();
                    row
                }
                Ok(report) => {
                    interrupted = report.failures.iter().any(|error| error.status == 130);
                    failed(
                        approved,
                        &report
                            .failures
                            .iter()
                            .map(|error| error.message.as_str())
                            .collect::<Vec<_>>()
                            .join("; "),
                    )
                }
                Err(error) => {
                    interrupted = error.status == 130;
                    failed(approved, &error.message)
                }
            }
        })
        .collect();
    (rows, interrupted || checkpoint())
}

pub(crate) fn same_scope(a: &Value, b: &Value) -> bool {
    ["provider", "status", "skills", "hooks"]
        .iter()
        .all(|key| a[key] == b[key])
}

fn failed(row: &Value, message: &str) -> Value {
    json!({"provider":row["provider"],"status":"failed","selected":row["selected"],"message":message,"hint":row["hint"]})
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core("tmt setup", &[""], &[]),
    crate::cli_style_tests::HintSpec::core("tmt setup {}", &[""], &[("{}", "claude")]),
];

#[cfg(test)]
mod tests {
    use super::super::tests::Directory;
    use super::*;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };

    fn environment(root: &Directory, present: &[&str]) -> ProviderEnvironment {
        let bin = root.path.join("bin");
        fs::create_dir(&bin).unwrap();
        for name in present {
            let file = bin.join(name);
            fs::write(&file, "#!/bin/sh\nexit 99\n").unwrap();
            fs::set_permissions(file, fs::Permissions::from_mode(0o755)).unwrap();
        }
        ProviderEnvironment::from_parts(root.path.join("home"), root.path.clone(), vec![bin], [])
    }

    #[test]
    fn detected_new_scopes_are_read_only_and_config_only_never_gets_hooks() {
        let root = Directory::new();
        let env = environment(&root, &["claude", "codex"]);
        fs::create_dir_all(root.path.join("home/.gemini")).unwrap();
        let global = root.path.join("state");
        let prefix = root.path.join("custom prefix");
        let plans = plans_with(&prefix, &env, &global).unwrap();
        for name in ["claude", "codex"] {
            let (row, plan) = plans.iter().find(|(r, _)| r["provider"] == name).unwrap();
            assert_eq!(row["status"], "pending");
            assert!(plan.is_some());
            assert_eq!(row["hooks"][0]["launcher"], json!(prefix.join("bin/tmt")));
        }
        let config_only = Directory::new();
        let config_env = environment(&config_only, &[]);
        fs::create_dir_all(config_only.path.join("home/.gemini")).unwrap();
        let config_plans = plans_with(
            &config_only.path.join("prefix"),
            &config_env,
            &config_only.path.join("state"),
        )
        .unwrap();
        let (gemini, _) = config_plans
            .iter()
            .find(|(row, _)| row["provider"] == "gemini")
            .unwrap();
        assert_eq!(gemini["hooks"], json!([]));
        assert_eq!(gemini["skills"].as_array().unwrap().len(), 2);
        assert!(!global.exists());
        assert!(!prefix.exists());
        assert!(!root.path.join("home/.agents").exists());
        assert!(!root.path.join("home/.claude").exists());
    }

    #[test]
    fn unsafe_provider_is_reported_without_blocking_an_independent_plan() {
        let root = Directory::new();
        let env = environment(&root, &["claude", "codex"]);
        let target = root.path.join("settings");
        fs::write(&target, b"user-secret").unwrap();
        let claude = root.path.join("home/.claude");
        fs::create_dir_all(&claude).unwrap();
        symlink(&target, claude.join("settings.json")).unwrap();
        let plans = plans_with(&root.path.join("prefix"), &env, &root.path.join("state")).unwrap();
        assert!(plans.iter().any(|(row, plan)| row["provider"] == "claude"
            && row["status"] == "needsApproval"
            && plan.is_none()));
        assert!(plans.iter().any(|(row, plan)| row["provider"] == "codex"
            && row["status"] == "pending"
            && plan.is_some()));
        assert_eq!(fs::read(&target).unwrap(), b"user-secret");
        assert_eq!(fs::read_link(claude.join("settings.json")).unwrap(), target);
    }

    #[test]
    fn consented_new_scope_uses_setup_and_detects_settings_drift() {
        for drift in [false, true] {
            let root = Directory::new();
            let env = environment(&root, &["claude"]);
            let global = root.path.join("state");
            let prefix = root.path.join("prefix");
            let settings = root.path.join("home/.claude/settings.json");
            fs::create_dir_all(settings.parent().unwrap()).unwrap();
            fs::write(&settings, "{\"userSecret\":\"opaque\"}\n").unwrap();
            let mut plans = plans_with(&prefix, &env, &global).unwrap();
            let (row, plan) = plans
                .iter_mut()
                .find(|(row, _)| row["provider"] == "claude")
                .unwrap();
            assert!(
                !row.to_string().contains("opaque"),
                "scope records contain hashes, never provider settings"
            );
            let plan = plan.take().unwrap();
            if drift {
                fs::write(&settings, "{\"userSecret\":\"changed\"}\n").unwrap();
            }
            let report = apply_with_policy(
                &plan,
                &global,
                &mut Vec::new(),
                Terminal::PLAIN,
                OutputMode { json: true },
                true,
            )
            .unwrap();
            if drift {
                assert!(!report.failures.is_empty());
                assert_eq!(
                    fs::read_to_string(&settings).unwrap(),
                    "{\"userSecret\":\"changed\"}\n"
                );
            } else {
                assert!(report.failures.is_empty());
                assert!(
                    fs::read_to_string(&settings)
                        .unwrap()
                        .contains("\"userSecret\":\"opaque\"")
                );
                assert!(
                    fs::read_to_string(&settings)
                        .unwrap()
                        .contains("SessionStart")
                );
                assert_eq!(record::read(&global).unwrap().len(), 1);
            }
        }
    }
}
