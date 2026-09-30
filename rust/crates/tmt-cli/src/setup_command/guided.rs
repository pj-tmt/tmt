//! Guided `tmt setup`: every detected agent is set up with no selection step.
//! Detection reads only the filesystem. Skills go to agents that are present
//! or configured; session hooks only to agents whose executable is here. It
//! plans only what is missing, asks once, then applies skills before hooks.

use super::{USAGE_NOTE, failure};
use crate::{invocation::OutputMode, output::Failure};
use serde_json::json;
use std::{io::Write, path::PathBuf};
use tmt_adapters::{
    config::ConfigPaths,
    drivers::{Detection, DriverDefinition, Registry},
    setup::{self, SetupEnvironment, SetupPlan, UsageHook, record},
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

/// The extension the closing hint offers when it is not installed.
const SUGGESTED_EXTENSION: &str = "squad";

struct Plan {
    detections: Vec<(&'static DriverDefinition, Detection)>,
    /// Core skill targets, one row per target; the driver installs it.
    core: Vec<(&'static DriverDefinition, SkillTarget)>,
    owned: Vec<SkillTarget>,
    /// Every hook-capable present driver's plan, current or not.
    hooks: Vec<SetupPlan>,
}

impl Plan {
    fn core_changes(&self) -> impl Iterator<Item = &(&'static DriverDefinition, SkillTarget)> {
        self.core.iter().filter(|(_, skill)| skill.state.changes())
    }

    fn hook_changes(&self) -> impl Iterator<Item = &SetupPlan> {
        self.hooks.iter().filter(|plan| plan.change.changed())
    }

    fn is_current(&self) -> bool {
        self.core_changes().next().is_none()
            && self.owned.is_empty()
            && self.hook_changes().next().is_none()
    }

    fn occupied(&self) -> impl Iterator<Item = &SkillTarget> {
        self.core
            .iter()
            .map(|(_, skill)| skill)
            .filter(|skill| skill.state == SkillState::Occupied)
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
    let detections = drivers.detect(env);
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
        hooks.push(
            setup::plan(
                driver,
                path,
                before,
                environment.launcher.clone(),
                false,
                usage,
            )
            .map_err(failure)?,
        );
    }
    Ok(Plan {
        detections,
        core,
        owned,
        hooks,
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
            // Named, so an outdated TMT skill is never replaced silently.
            SkillState::Foreign { source } => format!(
                "{}; replaces an older TMT skill from {} (backed up first)",
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
            Cell::styled(hooks.events(), Token::Dim),
        ]);
    }
    let mut kept = Table::new(&[Column::Detail, Column::Detail]);
    for skill in plan.occupied() {
        kept.row([
            Cell::from(home(&skill.target)),
            Cell::from("not TMT's; left as it is"),
        ]);
    }
    let change_count = plan.core_changes().count() + plan.owned.len() + plan.hook_changes().count();
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
    if !kept.is_empty() {
        sections.push(Section {
            title: "keep",
            count: Some(plan.occupied().count()),
            rows: kept,
            note: None,
            hint: None,
        });
    }
    list::write(output, terminal, &sections)?;
    if plan.hook_changes().any(|hooks| hooks.usage) {
        writeln!(output, "{USAGE_NOTE}")?;
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
    json!({
        "detectedProviders": plan.detections.iter().filter(|(_, detection)| gets_skills(detection)).map(|(driver, _)| driver.name()).collect::<Vec<_>>(),
        "integrations": plan.hooks.iter().map(|hooks| {
            let mut integration = json!({
                "provider": hooks.provider, "current": !hooks.change.changed(),
                "settingsPath": hooks.change.path, "launcher": hooks.launcher
            });
            // Additive: present only while the usage hook is installed.
            if hooks.usage {
                integration["usage"] = json!(true);
            }
            integration
        }).collect::<Vec<_>>(),
        "agents": plan.detections.iter().map(|(driver, detection)| json!({"name": driver.name(), "state": state(detection)})).collect::<Vec<_>>(),
        "plan": changes,
        "kept": plan.occupied().map(|skill| &skill.target).collect::<Vec<_>>(),
        "applied": applied,
        "skipped": skipped,
    })
}

pub(super) fn run(
    drivers: &Registry,
    environment: &SetupEnvironment,
    usage: UsageHook,
    yes: bool,
    mode: OutputMode,
    output: &mut Stream<impl Write>,
    terminal: Terminal,
) -> Result<(), Failure> {
    let env = ProviderEnvironment::capture().map_err(failure)?;
    let global = ConfigPaths::discover().map_err(Failure::from)?.global_dir;
    // An unreadable record is preserved and stops setup before any change.
    record::read(&global).map_err(failure)?;
    let plan = plan(drivers, environment, &env, &global, usage)?;
    if !mode.json {
        present(output, terminal, &plan).map_err(failure)?;
    }
    if plan.is_current() {
        // Only TMT's own record changes, so this needs no consent.
        adopt(&plan, &global)?;
        if mode.json {
            writeln!(output, "{}", document(&plan, false, &[])).map_err(failure)?;
        } else {
            message::success(output, terminal, "Everything is set up").map_err(failure)?;
            closing_hint(output, terminal)?;
        }
        return Ok(());
    }
    if !crate::consent::ask(
        output,
        yes,
        mode,
        crate::consent::Consent {
            code: "SETUP_CONSENT_REQUIRED",
            refusal: "Review setup interactively, or pass --yes to apply the plan; nothing was changed.",
            question: "Apply these changes",
            declined: "No changes made.",
        },
        failure,
    )? {
        return Ok(());
    }
    let skipped = apply(&plan, &global, output, terminal, mode)?;
    if mode.json {
        writeln!(output, "{}", document(&plan, true, &skipped)).map_err(failure)?;
    } else {
        message::hint(
            output,
            terminal,
            "reload your agents so they read the new skills",
        )
        .map_err(failure)?;
        closing_hint(output, terminal)?;
    }
    Ok(())
}

/// Skills, then hooks; each step reports as it completes.
fn apply(
    plan: &Plan,
    global: &std::path::Path,
    output: &mut impl Write,
    terminal: Terminal,
    mode: OutputMode,
) -> Result<Vec<PathBuf>, Failure> {
    let mut done = |text: String| -> Result<(), Failure> {
        if mode.json {
            Ok(())
        } else {
            message::success(&mut *output, terminal, &text).map_err(failure)
        }
    };
    // Exactly the planned core targets: occupied ones stay as the plan said.
    let planned: Vec<SkillTarget> = plan
        .core_changes()
        .map(|(_, skill)| skill.clone())
        .collect();
    let mut skipped = Vec::new();
    if !planned.is_empty() {
        let published = skill_installation::publish_core(global, &planned).map_err(failure)?;
        done(format!("Installed {} skill(s)", published.linked.len()))?;
        for backup in &published.backups {
            done(format!("Backed up the replaced skill to {}", home(backup)))?;
        }
        skipped = published.skipped;
    }
    if !plan.owned.is_empty() {
        let linked = skill_installation::publish_owned(global, &plan.owned).map_err(failure)?;
        done(format!("Linked {} extension skill(s)", linked.len()))?;
    }
    for hooks in plan.hook_changes() {
        setup::apply(hooks).map_err(failure)?;
        record::remember(
            global,
            record::RecordedHooks {
                driver: hooks.provider.to_owned(),
                settings: hooks.change.path.clone(),
                launcher: hooks.launcher.clone(),
            },
        )
        .map_err(failure)?;
        done(format!("Configured {} hooks", hooks.provider))?;
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
    adopt(plan, global)?;
    Ok(skipped)
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
            },
        )
        .map_err(failure)?;
    }
    Ok(())
}

fn closing_hint(output: &mut impl Write, terminal: Terminal) -> Result<(), Failure> {
    let installed = crate::extension_command::discover()
        .map(|found| found.has(SUGGESTED_EXTENSION))
        .unwrap_or(true);
    if installed {
        return Ok(());
    }
    message::hint(
        output,
        terminal,
        &format!("tmt extension install {SUGGESTED_EXTENSION} adds the Squad board"),
    )
    .map_err(failure)
}
