//! Guided setup: detected agents plus a consented selection of official extensions.
//! Detection reads only the filesystem. Skills go to agents that are present
//! or configured; session hooks only to agents whose executable is here. It
//! plans only what is missing, asks once, then applies skills before hooks.

pub(crate) mod extensions;
pub(crate) mod upgrade;

use super::{USAGE_NOTE, failure};
use crate::{invocation::OutputMode, output::Failure};
use serde_json::json;
use std::{io::Write, path::PathBuf};
use tmt_adapters::{
    config::ConfigPaths,
    drivers::{Detection, DriverDefinition, Registry},
    setup::{
        self, SetupEnvironment, SetupPlan, UsageHook,
        provider_mod::{self, PendingMod},
        record,
    },
    skill_installation::{self, ProviderEnvironment, SkillState, SkillTarget},
};
use tmt_cli_style::{
    Terminal, Token,
    list::{self, Section},
    mark::Mark,
    message,
    stream::Stream,
    table::{Cell, Column, Table},
    value,
};

struct Plan {
    detections: Vec<(&'static DriverDefinition, Detection)>,
    /// Core skill targets, one row per target; the driver installs it.
    core: Vec<(&'static DriverDefinition, SkillTarget)>,
    owned: Vec<SkillTarget>,
    /// Every hook-capable present driver's plan, current or not.
    hooks: Vec<SetupPlan>,
    /// The managed mod each consenting driver keeps, when it is not as wanted yet (#5187).
    mods: Vec<PendingMod>,
}

impl Plan {
    fn core_changes(&self) -> impl Iterator<Item = &(&'static DriverDefinition, SkillTarget)> {
        self.core.iter().filter(|(_, skill)| skill.state.changes())
    }

    fn hook_changes(&self) -> impl Iterator<Item = &SetupPlan> {
        self.hooks.iter().filter(|plan| plan.change.changed())
    }

    fn mod_changes(&self) -> impl Iterator<Item = &PendingMod> {
        self.mods.iter().filter(|pending| pending.changes())
    }

    fn is_current(&self) -> bool {
        self.core_changes().next().is_none()
            && self.owned.is_empty()
            && self.hook_changes().next().is_none()
            && self.mod_changes().next().is_none()
    }
}

fn gets_skills(detection: &Detection) -> bool {
    matches!(detection, Detection::Present { .. } | Detection::ConfigOnly)
}

fn plan(
    drivers: &Registry,
    environment: &SetupEnvironment,
    env: &ProviderEnvironment,
    global: &std::path::Path,
    usage: UsageHook,
) -> Result<Plan, Failure> {
    plan_for(drivers, environment, env, global, usage, None)
}

fn plan_for(
    drivers: &Registry,
    environment: &SetupEnvironment,
    env: &ProviderEnvironment,
    global: &std::path::Path,
    usage: UsageHook,
    provider: Option<&str>,
) -> Result<Plan, Failure> {
    let recorded = record::read(global).map_err(failure)?;
    let detections = drivers
        .detect(env)
        .into_iter()
        .filter(|(driver, _)| provider.is_none_or(|name| name == driver.name()))
        .collect::<Vec<_>>();
    let mut core: Vec<(&'static DriverDefinition, SkillTarget)> = Vec::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    for (driver, detection) in &detections {
        if !gets_skills(detection) {
            continue;
        }
        let root = env.locations(driver).skills;
        if roots.contains(&root) {
            continue;
        }
        roots.push(root);
        for skill in skill_installation::plan_core(env, global, driver).map_err(failure)? {
            core.push((driver, skill));
        }
    }
    let owned = skill_installation::plan_owned(global, &roots).map_err(failure)?;
    let mut hooks = Vec::new();
    for (driver, detection) in &detections {
        if !matches!(detection, Detection::Present { .. }) || driver.descriptor.hooks.is_none() {
            continue;
        }
        let path = environment
            .settings_path(driver)
            .map_err(failure)?
            .to_path_buf();
        let before = setup::read_settings(&path).map_err(failure)?;
        let opted_out = record::usage_opted_out(&recorded, driver.name(), &path);
        let usage = record::usage_policy(&recorded, driver.name(), &path, usage);
        hooks.push(
            setup::plan_with_choice(
                driver,
                path,
                before,
                environment.launcher.clone(),
                false,
                usage,
                opted_out,
            )
            .map_err(failure)?,
        );
    }
    let mods = hooks
        .iter()
        .map(|plan| provider_mod::pending(plan.provider, &plan.change.path, plan.usage))
        .collect::<Result<Vec<_>, _>>()
        .map_err(failure)?
        .into_iter()
        .flatten()
        .collect();
    Ok(Plan {
        detections,
        core,
        owned,
        hooks,
        mods,
    })
}

fn home(path: &std::path::Path) -> String {
    value::home_path(path, std::env::home_dir().as_deref())
}

fn agent_row(detection: &Detection) -> (Mark, String) {
    match detection {
        Detection::Present { .. } => (Mark::Done, "present".into()),
        Detection::ConfigOnly => (
            Mark::Warning,
            "config only; skills, no hooks (no executable on PATH)".into(),
        ),
        Detection::Broken { reason } => (Mark::Failed, format!("not runnable: {reason}")),
        Detection::Absent => (Mark::Offline, "not found".into()),
    }
}

fn present(output: &mut impl Write, terminal: Terminal, plan: &Plan) -> std::io::Result<()> {
    let mut agents = Table::new(&[Column::Fixed, Column::Name, Column::Detail]);
    let mut absent = Vec::new();
    for (driver, detection) in &plan.detections {
        if matches!(detection, Detection::Absent) {
            absent.push(driver.name());
            continue;
        }
        let (mark, state) = agent_row(detection);
        agents.row([
            Cell::styled(mark.symbol(), mark.token()),
            Cell::from(driver.name()),
            Cell::from(state),
        ]);
    }
    let found = plan.detections.len() - absent.len();
    let note = (!absent.is_empty()).then(|| format!("not found: {}", absent.join(" · ")));
    let mut changes = Table::new(&[Column::Fixed, Column::Detail, Column::Detail]);
    for (driver, skill) in plan.core_changes() {
        let note = match &skill.state {
            SkillState::Foreign { source } => format!(
                "{}; replaces an older TMT skill from {}",
                driver.name(),
                home(source)
            ),
            _ => driver.name().to_owned(),
        };
        changes.row([
            Cell::styled("skill", Token::Dim),
            Cell::from(home(&skill.target)),
            Cell::styled(note, Token::Dim),
        ]);
    }
    for skill in &plan.owned {
        changes.row([
            Cell::styled("skill", Token::Dim),
            Cell::from(home(&skill.target)),
            Cell::styled("extension", Token::Dim),
        ]);
    }
    for hooks in plan.hook_changes() {
        changes.row([
            Cell::styled("hooks", Token::Dim),
            Cell::from(home(&hooks.change.path)),
            Cell::styled(
                format!(
                    "{}; backups in {} (warning above 32; manual cleanup only)",
                    hooks.events(),
                    home(&setup::backup_directory(&hooks.change.path).expect("settings parent"))
                ),
                Token::Dim,
            ),
        ]);
    }
    for pending in plan.mod_changes() {
        changes.row([
            Cell::styled("mod", Token::Dim),
            Cell::from(home(pending.target())),
            Cell::styled(
                format!("{}; {}", pending.provider(), pending.purpose()),
                Token::Dim,
            ),
        ]);
    }
    let change_count = plan.core_changes().count()
        + plan.owned.len()
        + plan.hook_changes().count()
        + plan.mod_changes().count();
    let mut sections = vec![Section {
        title: "agents",
        count: Some(found),
        rows: agents,
        note: note.as_deref(),
        hint: None,
    }];
    if change_count > 0 {
        sections.push(Section {
            title: "changes",
            count: Some(change_count),
            rows: changes,
            note: None,
            hint: None,
        });
    }
    list::write(output, terminal, &sections)?;
    for hooks in plan.hooks.iter().filter(|hooks| !hooks.usage) {
        writeln!(output, "{}", super::collection_off(hooks.provider))?;
    }
    if plan.hook_changes().any(|hooks| !hooks.usage) {
        writeln!(output, "{}", super::ANSWERS_NOTE)?;
    }
    if plan.hook_changes().any(|hooks| hooks.usage) {
        writeln!(output, "{USAGE_NOTE}")?;
    }
    for pending in plan.mods.iter().filter(|pending| !pending.changes()) {
        writeln!(output, "{}", super::mod_note(pending))?;
    }
    Ok(())
}

fn document(plan: &Plan, applied: bool, skipped: &[PathBuf]) -> serde_json::Value {
    let state = |detection: &Detection| match detection {
        Detection::Present { .. } => "present",
        Detection::ConfigOnly => "configOnly",
        Detection::Broken { .. } => "broken",
        Detection::Absent => "absent",
    };
    let mut changes: Vec<serde_json::Value> = plan
        .core_changes()
        .map(|(driver, skill)| {
            let mut change =
                json!({"kind": "skill", "driver": driver.name(), "path": skill.target});
            if let SkillState::Foreign { source } = &skill.state {
                change["replaces"] = json!(source);
            }
            change
        })
        .collect();
    changes.extend(
        plan.owned
            .iter()
            .map(|skill| json!({"kind": "skill", "driver": null, "path": skill.target})),
    );
    changes.extend(plan.hook_changes().map(
        |hooks| json!({"kind": "hooks", "driver": hooks.provider, "path": hooks.change.path}),
    ));
    changes.extend(plan.mod_changes().map(
        |pending| json!({"kind": "mod", "driver": pending.provider(), "path": pending.target()}),
    ));
    let mut document = json!({
        "detectedProviders": plan.detections.iter().filter(|(_, detection)| gets_skills(detection)).map(|(driver, _)| driver.name()).collect::<Vec<_>>(),
        "integrations": plan.hooks.iter().map(|hooks| {
            let mut integration = json!({
                "provider": hooks.provider, "current": !hooks.change.changed(),
                "settingsPath": hooks.change.path, "launcher": hooks.launcher
            });
            super::collection_document(&mut integration, hooks.provider, hooks.usage);
            integration
        }).collect::<Vec<_>>(),
        "agents": plan.detections.iter().map(|(driver, detection)| json!({"name": driver.name(), "state": state(detection)})).collect::<Vec<_>>(),
        "plan": changes,
        "kept": [],
        "applied": applied,
        "skipped": skipped,
    });
    let warnings: Vec<String> = plan
        .hooks
        .iter()
        .filter_map(|hooks| setup::backup_warning(&hooks.change.path))
        .collect();
    if !warnings.is_empty() {
        document["warnings"] = json!(warnings);
    }
    document
}

pub(super) fn run(
    drivers: &Registry,
    environment: &SetupEnvironment,
    usage: UsageHook,
    yes: bool,
    mode: OutputMode,
    output: &mut Stream<impl Write>,
    terminal: Terminal,
) -> Result<u8, Failure> {
    let env = ProviderEnvironment::capture().map_err(failure)?;
    let global = ConfigPaths::discover().map_err(Failure::from)?.global_dir;
    // An unreadable record is preserved and stops setup before any change.
    record::read(&global).map_err(failure)?;
    let plan = plan(drivers, environment, &env, &global, usage)?;
    let mut extensions = extensions::Plan::read();
    if !mode.json {
        present(output, terminal, &plan).map_err(failure)?;
        if !extensions.select(output, yes, mode).map_err(failure)? {
            writeln!(output, "No changes made.").map_err(failure)?;
            return Ok(0);
        }
    }
    if plan.is_current() && extensions.is_current() {
        // Only TMT's own record changes, so this needs no consent.
        adopt(&plan, &global)?;
        if mode.json {
            let mut document = document(&plan, false, &[]);
            document["extensions"] = json!(extensions.current_document());
            document["guidance"] = crate::install_guidance::read(None, &[]).document();
            writeln!(output, "{document}").map_err(failure)?;
        } else {
            message::success(output, terminal, "Everything is set up").map_err(failure)?;
            backup_warnings(&plan, output, terminal)?;
            crate::install_guidance::read(None, &[])
                .write("Next", terminal, output)
                .map_err(failure)?;
        }
        return Ok(0);
    }
    if !crate::consent::ask(
        output,
        yes,
        mode,
        crate::consent::Consent {
            code: "SETUP_CONSENT_REQUIRED",
            refusal: "Review setup interactively, or pass --yes to apply the plan; nothing was changed.",
            question: "Apply all of this",
            declined: "No changes made.",
        },
        failure,
    )? {
        return Ok(0);
    }
    let agents = apply(&plan, &global, output, terminal, mode);
    let extension_rows = extensions.apply(output, terminal, mode)?;
    let incomplete = agents
        .as_ref()
        .map_or(true, |report| !report.failures.is_empty())
        || extensions::failed(&extension_rows);
    let status = if extensions::interrupted(&extension_rows) {
        130
    } else {
        u8::from(incomplete)
    };
    if mode.json {
        let mut document = document(
            &plan,
            true,
            agents
                .as_ref()
                .map_or(&[], |report| report.skipped.as_slice()),
        );
        document["extensions"] = json!(extension_rows);
        document["guidance"] = crate::install_guidance::read(None, &extension_rows).document();
        let errors: Vec<_> = match &agents {
            Ok(report) => report
                .failures
                .iter()
                .map(|error| error.document()["error"].clone())
                .collect(),
            Err(error) => vec![error.document()["error"].clone()],
        };
        if !errors.is_empty() {
            document["agentErrors"] = json!(errors);
        }
        if incomplete {
            document["error"] = json!({"code":"SETUP_INCOMPLETE", "message":"Some setup items need attention. Completed items were kept. Rerun: tmt setup --yes"});
        }
        writeln!(output, "{document}").map_err(failure)?;
    } else {
        let errors = match agents {
            Ok(report) => report.failures,
            Err(error) => vec![error],
        };
        for error in errors {
            message::warning(output, terminal, &error.message, None).map_err(failure)?;
        }
        let ready = extension_rows
            .iter()
            .filter(|row| matches!(row["status"].as_str(), Some("current" | "installed")))
            .count();
        message::hint(
            output,
            terminal,
            &format!("{ready} extension(s) ready; reload your agents so they read the new skills"),
        )
        .map_err(failure)?;
        if incomplete {
            message::warning(
                output,
                terminal,
                "Completed items were kept. Rerun: tmt setup --yes",
                None,
            )
            .map_err(failure)?;
        }
        backup_warnings(&plan, output, terminal)?;
        crate::install_guidance::read(None, &extension_rows)
            .write("Next", terminal, output)
            .map_err(failure)?;
    }
    Ok(status)
}

fn backup_warnings(
    plan: &Plan,
    output: &mut impl Write,
    terminal: Terminal,
) -> Result<(), Failure> {
    for hooks in &plan.hooks {
        if let Some(warning) = setup::backup_warning(&hooks.change.path) {
            message::warning(output, terminal, &warning, None).map_err(failure)?;
        }
    }
    Ok(())
}

struct AgentReport {
    skipped: Vec<PathBuf>,
    failures: Vec<Failure>,
}

/// Skills, then hooks; keep independent successes when an item fails.
fn apply(
    plan: &Plan,
    global: &std::path::Path,
    output: &mut impl Write,
    terminal: Terminal,
    mode: OutputMode,
) -> Result<AgentReport, Failure> {
    apply_with_policy(plan, global, output, terminal, mode, false)
}

fn apply_with_policy(
    plan: &Plan,
    global: &std::path::Path,
    output: &mut impl Write,
    terminal: Terminal,
    mode: OutputMode,
    new_scope: bool,
) -> Result<AgentReport, Failure> {
    let mut done = |text: String| -> Result<(), Failure> {
        if mode.json {
            Ok(())
        } else {
            message::success(&mut *output, terminal, &text).map_err(failure)
        }
    };
    // Publication replaces existing entries at the planned core skill names.
    let planned: Vec<SkillTarget> = plan
        .core_changes()
        .map(|(_, skill)| skill.clone())
        .collect();
    let mut skipped = Vec::new();
    let mut failures = Vec::new();
    if !planned.is_empty() {
        match if new_scope {
            skill_installation::publish_missing_core(global, &planned)
        } else {
            skill_installation::publish_core(global, &planned)
        } {
            Ok(published) => {
                done(format!("Installed {} skill(s)", published.linked.len()))?;
                skipped = published.skipped;
            }
            Err(error) => failures.push(
                Failure::new(
                    "SETUP_ERROR",
                    format!("Core agent skills could not be set up: {error}"),
                    1,
                )
                .caused_by(error),
            ),
        }
    }
    if !plan.owned.is_empty() {
        match skill_installation::publish_owned(global, &plan.owned) {
            Ok(linked) => done(format!("Linked {} extension skill(s)", linked.len()))?,
            Err(error) => failures.push(
                Failure::new(
                    "SETUP_ERROR",
                    format!("Recorded extension skill links could not be set up: {error}"),
                    1,
                )
                .caused_by(error),
            ),
        }
    }
    for hooks in plan.hook_changes() {
        let result = setup::apply(hooks).and_then(|_| {
            record::remember(
                global,
                record::RecordedHooks {
                    driver: hooks.provider.to_owned(),
                    settings: hooks.change.path.clone(),
                    launcher: hooks.launcher.clone(),
                    usage: Some(hooks.usage),
                },
            )
        });
        match result {
            Ok(()) => done(format!("Configured {} hooks", hooks.provider))?,
            Err(error) => failures.push(
                Failure::new(
                    "SETUP_ERROR",
                    format!("{} hooks could not be set up: {error}", hooks.provider),
                    1,
                )
                .caused_by(error),
            ),
        }
    }
    for pending in plan.mod_changes() {
        match pending.apply() {
            Ok(outcome) => done(format!(
                "{} the {} mod",
                super::capitalized(outcome.label()),
                pending.provider()
            ))?,
            Err(error) => failures.push(
                Failure::new(
                    "SETUP_ERROR",
                    format!(
                        "The {} mod could not be set up: {error}",
                        pending.provider()
                    ),
                    1,
                )
                .caused_by(error),
            ),
        }
    }
    if !mode.json {
        for path in &skipped {
            message::warning(
                &mut *output,
                terminal,
                &format!("Left {} as it is; it changed after planning", home(path)),
                None,
            )
            .map_err(failure)?;
        }
    }
    if let Err(error) = adopt(plan, global) {
        failures.push(error);
    }
    Ok(AgentReport { skipped, failures })
}

/// Records hooks that are already exactly what setup writes (installed
/// before the record existed), so uninstall knows them.
fn adopt(plan: &Plan, global: &std::path::Path) -> Result<(), Failure> {
    for hooks in plan.hooks.iter().filter(|hooks| !hooks.change.changed()) {
        record::remember(
            global,
            record::RecordedHooks {
                driver: hooks.provider.to_owned(),
                settings: hooks.change.path.clone(),
                launcher: hooks.launcher.clone(),
                usage: Some(hooks.usage),
            },
        )
        .map_err(failure)?;
    }
    Ok(())
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "Completed items were kept. Rerun: tmt setup --yes",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "Some setup items need attention. Completed items were kept. Rerun: tmt setup --yes",
        &[""],
        &[],
    ),
];
#[cfg(test)]
pub(crate) use extensions::PRINTED_HINTS as EXTENSION_HINTS;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) struct Directory {
        pub(crate) path: PathBuf,
    }
    impl Directory {
        pub(crate) fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "tmt-setup-independent-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir(&path).unwrap();
            Self { path }
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }
    fn claude_world(root: &Directory) -> (ProviderEnvironment, SetupEnvironment) {
        use std::os::unix::fs::PermissionsExt;
        let bin = root.path.join("bin");
        std::fs::create_dir(&bin).unwrap();
        std::fs::write(bin.join("claude"), "#!/bin/sh\nexit 99\n").unwrap();
        std::fs::set_permissions(bin.join("claude"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let env = ProviderEnvironment::from_parts(
            root.path.join("home"),
            root.path.clone(),
            vec![bin.clone()],
            [],
        );
        let setup = SetupEnvironment {
            launcher: bin.join("tmt"),
            settings: Registry::builtin()
                .with_hooks()
                .map(|driver| (driver, env.locations(driver).hook_settings.unwrap()))
                .collect(),
        };
        (env, setup)
    }

    #[test]
    fn consented_usage_plans_the_mod_and_an_opt_out_or_a_foreign_directory_does_not() {
        use tmt_adapters::setup::provider_mod::ModStep;
        let root = Directory::new();
        let (env, setup) = claude_world(&root);
        let drivers = Registry::builtin();
        let global = root.path.join("state");
        let plan =
            |usage| plan_for(&drivers, &setup, &env, &global, usage, Some("claude")).unwrap();

        let consented = plan(UsageHook::Default);
        assert_eq!(consented.mods.len(), 1);
        assert_eq!(consented.mods[0].step, ModStep::Install);
        assert!(!consented.is_current());

        assert!(plan(UsageHook::Remove).mods.is_empty());

        let target = consented.mods[0].target().to_path_buf();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("mine"), b"mine").unwrap();
        let blocked = plan(UsageHook::Default);
        assert_eq!(blocked.mods[0].step, ModStep::Blocked);
        assert_eq!(blocked.mod_changes().count(), 0);
    }

    #[test]
    fn applying_the_plan_installs_the_mod_after_the_hooks() {
        let root = Directory::new();
        let (env, setup) = claude_world(&root);
        let drivers = Registry::builtin();
        let global = root.path.join("state");
        let planned = plan_for(
            &drivers,
            &setup,
            &env,
            &global,
            UsageHook::Default,
            Some("claude"),
        )
        .unwrap();
        let target = planned.mods[0].target().to_path_buf();
        let report = apply(
            &planned,
            &global,
            &mut Vec::new(),
            Terminal::PLAIN,
            OutputMode { json: true },
        )
        .unwrap();
        assert!(report.failures.is_empty(), "{:?}", report.failures.len());
        assert!(target.join(".tmt-managed.json").is_file());
        assert!(target.join("hooks/register.mjs").is_file());
        // Nothing left to do on the next plan.
        let again = plan_for(
            &drivers,
            &setup,
            &env,
            &global,
            UsageHook::Default,
            Some("claude"),
        )
        .unwrap();
        assert!(again.mods.is_empty());
    }

    #[test]
    fn a_changed_provider_file_preserves_it_and_does_not_block_an_independent_provider() {
        let root = Directory::new();
        let drivers = Registry::builtin();
        let paths = [
            root.path.join("claude/settings.json"),
            root.path.join("codex/hooks.json"),
        ];
        let hooks = ["claude", "codex"]
            .into_iter()
            .zip(&paths)
            .map(|(name, path)| {
                setup::plan_with_choice(
                    drivers.find(name).unwrap(),
                    path.clone(),
                    None,
                    root.path.join("bin/tmt"),
                    false,
                    UsageHook::Remove,
                    false,
                )
                .unwrap()
            })
            .collect();
        let plan = Plan {
            detections: Vec::new(),
            core: Vec::new(),
            owned: Vec::new(),
            hooks,
            mods: Vec::new(),
        };
        std::fs::create_dir_all(paths[0].parent().unwrap()).unwrap();
        std::fs::write(&paths[0], b"user changed this after planning").unwrap();
        let report = apply(
            &plan,
            &root.path.join("global"),
            &mut Vec::new(),
            Terminal::PLAIN,
            OutputMode { json: true },
        )
        .unwrap();
        assert_eq!(report.failures.len(), 1);
        assert_eq!(
            std::fs::read(&paths[0]).unwrap(),
            b"user changed this after planning"
        );
        assert!(
            std::fs::read_to_string(&paths[1])
                .unwrap()
                .contains("__hook codex")
        );
        let recorded = record::read(&root.path.join("global")).unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].driver, "codex");
    }
}
