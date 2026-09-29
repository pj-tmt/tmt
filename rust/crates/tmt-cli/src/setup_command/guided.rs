//! Guided `tmt setup`: every detected agent is set up with no selection step.
//! Detection reads only the filesystem. Skills go to agents that are present
//! or configured; session hooks only to agents whose executable is here. It
//! plans only what is missing, asks once, then applies skills before hooks.

use super::failure;
use crate::{invocation::OutputMode, output::Failure};
use serde_json::json;
use std::{io::Write, path::PathBuf};
use tmt_adapters::{
    config::ConfigPaths,
    drivers::{Detection, DriverDefinition, Registry},
    setup::{self, SetupEnvironment, SetupPlan, record},
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
        self.core
            .iter()
            .filter(|(_, skill)| matches!(skill.state, SkillState::Missing | SkillState::Stale))
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
            setup::plan(driver, path, before, environment.launcher.clone(), false)
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
        Detection::Present { .. } => (Mark::Running, "present".into()),
        Detection::ConfigOnly => (
            Mark::Idle,
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
        changes.row([
            Cell::styled("skill", Token::Dim),
            Cell::from(home(&skill.target)),
            Cell::styled(driver.name(), Token::Dim),
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
            Cell::styled("SessionStart, SessionEnd", Token::Dim),
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
    list::write(output, terminal, &sections)
}

fn document(plan: &Plan, applied: bool) -> serde_json::Value {
    let state = |detection: &Detection| match detection {
        Detection::Present { .. } => "present",
        Detection::ConfigOnly => "configOnly",
        Detection::Broken { .. } => "broken",
        Detection::Absent => "absent",
    };
    let mut changes: Vec<serde_json::Value> = plan
        .core_changes()
        .map(|(driver, skill)| json!({"kind": "skill", "driver": driver.name(), "path": skill.target}))
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
        "integrations": plan.hooks.iter().map(|hooks| json!({
            "provider": hooks.provider, "current": !hooks.change.changed(),
            "settingsPath": hooks.change.path, "launcher": hooks.launcher
        })).collect::<Vec<_>>(),
        "agents": plan.detections.iter().map(|(driver, detection)| json!({"name": driver.name(), "state": state(detection)})).collect::<Vec<_>>(),
        "plan": changes,
        "applied": applied,
    })
}

pub(super) fn run(
    drivers: &Registry,
    environment: &SetupEnvironment,
    yes: bool,
    mode: OutputMode,
    output: &mut Stream<impl Write>,
    terminal: Terminal,
) -> Result<(), Failure> {
    let env = ProviderEnvironment::capture().map_err(failure)?;
    let global = ConfigPaths::discover().map_err(Failure::from)?.global_dir;
    // An unreadable record is preserved and stops setup before any change.
    record::read(&global).map_err(failure)?;
    let plan = plan(drivers, environment, &env, &global)?;
    if !mode.json {
        present(output, terminal, &plan).map_err(failure)?;
    }
    if plan.is_current() {
        if mode.json {
            writeln!(output, "{}", document(&plan, false)).map_err(failure)?;
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
    apply(&plan, &env, &global, output, terminal, mode)?;
    if mode.json {
        writeln!(output, "{}", document(&plan, true)).map_err(failure)?;
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
    env: &ProviderEnvironment,
    global: &std::path::Path,
    output: &mut impl Write,
    terminal: Terminal,
    mode: OutputMode,
) -> Result<(), Failure> {
    let mut done = |text: String| -> Result<(), Failure> {
        if mode.json {
            Ok(())
        } else {
            message::success(&mut *output, terminal, &text).map_err(failure)
        }
    };
    let mut installed: Vec<&str> = Vec::new();
    for (driver, _) in plan.core_changes() {
        if installed.contains(&driver.name()) {
            continue;
        }
        skill_installation::install(env, global, Some(driver.name()), None, false)
            .map_err(|failure| Failure::new("SETUP_ERROR", failure.to_string(), 1))?;
        installed.push(driver.name());
        done(format!("Installed skills for {}", driver.name()))?;
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
    // Hooks already present exactly are adopted into the record too.
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
