//! `tmt uninstall`: plan every removal read-only, ask once, then remove in a
//! fixed order with the running CLI last: a running Office service is stopped,
//! then hooks, skills, extensions, the CLI, the setup record, and the data
//! directory only with `--purge`. Anything that
//! is no longer what TMT wrote is kept and reported. A failed step stops the
//! run; running it again resumes, because every step is idempotent.

use crate::{invocation::OutputMode, output::Failure};
use serde_json::json;
use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
};
use tmt_adapters::{
    config::ConfigPaths,
    drivers::Registry,
    native_install::{self, Product, ProductRemoval},
    setup::{
        record,
        removal::{self, HookStep},
    },
    skill_installation::{self, ProviderEnvironment, SkillsRemoval},
};
use tmt_cli_style::{
    list::{self, Section},
    message,
    table::{Cell, Column, Table},
    value,
};

/// Extensions first, then the CLI that is running this command.
const PRODUCTS: [Product; 3] = [Product::Office, Product::Squad, Product::Cli];

pub fn execute(purge: bool, yes: bool, prefix: Option<&str>, mode: OutputMode) -> io::Result<u8> {
    match run(purge, yes, prefix, mode) {
        Ok(()) => Ok(0),
        Err(error) => error.publish(mode),
    }
}

fn failure(error: impl std::error::Error + 'static) -> Failure {
    Failure::new("UNINSTALL_ERROR", error.to_string(), 1).caused_by(error)
}

struct Plan {
    /// A running local Office service, stopped before any file is removed.
    office_running: bool,
    hooks: Vec<HookStep>,
    skills: SkillsRemoval,
    products: Vec<(Product, ProductRemoval)>,
    data: PathBuf,
    data_exists: bool,
    purge: bool,
}

impl Plan {
    fn removals(&self) -> Vec<(&'static str, PathBuf)> {
        let mut rows = Vec::new();
        if self.office_running {
            rows.push(("stop", PathBuf::from("the local Office service")));
        }
        for step in &self.hooks {
            if let HookStep::Remove(planned) = step {
                rows.push(("hooks", planned.change.path.clone()));
            }
        }
        rows.extend(
            self.skills
                .removed
                .iter()
                .map(|path| ("skill", path.clone())),
        );
        for (_, product) in &self.products {
            rows.extend(product.removed.iter().map(|path| ("command", path.clone())));
        }
        if self.purge && self.data_exists {
            rows.push(("data", self.data.clone()));
        }
        rows
    }

    /// What stays, with why. Settings backups are listed as they are on disk
    /// now, so a call after removal includes the ones it wrote.
    fn kept(&self) -> Vec<(PathBuf, String)> {
        let mut rows = Vec::new();
        for step in &self.hooks {
            if let HookStep::Keep {
                settings, reason, ..
            } = step
            {
                rows.push((settings.clone(), reason.clone()));
            }
        }
        rows.extend(
            self.skills
                .kept
                .iter()
                .map(|path| (path.clone(), "it is no longer TMT's link".into())),
        );
        for (_, product) in &self.products {
            rows.extend(
                product
                    .kept
                    .iter()
                    .map(|path| (path.clone(), "it is not TMT's command".into())),
            );
        }
        rows.extend(self.backups().into_iter().map(|path| {
            (
                path,
                "backup of your settings before TMT removed its hooks".into(),
            )
        }));
        rows
    }

    /// `settings.tmt-backup-*.json` beside each hooks file.
    fn backups(&self) -> Vec<PathBuf> {
        let mut directories: Vec<PathBuf> = self
            .hooks
            .iter()
            .filter_map(|step| match step {
                HookStep::Remove(planned) => planned.change.path.parent().map(Path::to_path_buf),
                HookStep::Forget { settings, .. } | HookStep::Keep { settings, .. } => {
                    settings.parent().map(Path::to_path_buf)
                }
            })
            .collect();
        directories.sort();
        directories.dedup();
        let mut backups: Vec<PathBuf> = directories
            .iter()
            .filter_map(|directory| fs::read_dir(directory).ok())
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_name().to_str().is_some_and(|name| {
                    name.starts_with("settings.tmt-backup-") && name.ends_with(".json")
                })
            })
            .map(|entry| entry.path())
            .collect();
        backups.sort();
        backups
    }

    fn is_empty(&self) -> bool {
        self.removals().is_empty()
            && !self
                .hooks
                .iter()
                .any(|step| matches!(step, HookStep::Forget { .. }))
    }
}

/// `--prefix`, else the prefix of the running managed installation, else the
/// default prefix.
fn prefix(requested: Option<&str>) -> io::Result<PathBuf> {
    if let Some(requested) = requested {
        return Ok(std::env::current_dir()?.join(requested));
    }
    if let Ok(installation) = native_install::inspect(&std::env::current_exe()?) {
        return Ok(installation.prefix().to_path_buf());
    }
    native_install::default_install_prefix()
}

fn run(purge: bool, yes: bool, requested: Option<&str>, mode: OutputMode) -> Result<(), Failure> {
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let environment = ProviderEnvironment::capture().map_err(failure)?;
    let drivers = Registry::builtin();
    let prefix = prefix(requested).map_err(failure)?;
    // Read-only: an unreadable record stops here, before anything changes.
    // A service whose state cannot be confirmed stops the plan: removing its
    // files underneath it could leave a process without its installation.
    let office_running =
        crate::office_facade::service_control::running(&paths).map_err(|error| {
            Failure::new(
                "UNINSTALL_ERROR",
                format!("Could not confirm whether local Office is running: {error}"),
                1,
            )
            .suggestion("run tmt office stop, then tmt uninstall again".into())
        })?;
    let plan = Plan {
        office_running,
        hooks: removal::plan_hook_removal(&paths.global_dir, &environment, &drivers)
            .map_err(failure)?,
        skills: skill_installation::plan_uninstall(&environment, &paths.global_dir)
            .map_err(failure)?,
        products: PRODUCTS
            .into_iter()
            .map(|product| {
                Ok((
                    product,
                    native_install::plan_product_removal(&prefix, product)?,
                ))
            })
            .collect::<io::Result<_>>()
            .map_err(failure)?,
        data_exists: paths.global_dir.is_dir(),
        data: paths.global_dir.clone(),
        purge,
    };
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    let terminal = output.terminal();
    if plan.is_empty() {
        if mode.json {
            writeln!(output, "{}", report(&plan, &[], false)).map_err(failure)?;
        } else {
            writeln!(output, "TMT is not installed here; nothing to remove.").map_err(failure)?;
        }
        return Ok(());
    }
    if !mode.json {
        present(&mut output, terminal, &plan).map_err(failure)?;
    }
    let question = if purge {
        "Remove TMT and delete its data"
    } else {
        "Remove TMT"
    };
    if !crate::consent::ask(
        &mut output,
        yes,
        mode,
        crate::consent::Consent {
            code: "UNINSTALL_CONSENT_REQUIRED",
            refusal: "Uninstalling needs explicit --yes; nothing was removed.",
            question,
            declined: "Nothing was removed.",
        },
        failure,
    )? {
        return Ok(());
    }
    let mut removed = Vec::new();
    let result = remove(&plan, &paths, &environment, &prefix, &mut removed);
    let deleted = result.as_ref().is_ok_and(|deleted| *deleted);
    if let Err(error) = result {
        return Err(Failure::new(
            "UNINSTALL_ERROR",
            format!(
                "Uninstall stopped after removing {} item(s): {error}",
                removed.len()
            ),
            1,
        )
        .suggestion(
            "fix the cause, then run tmt uninstall again; completed steps are not repeated".into(),
        ));
    }
    if mode.json {
        writeln!(output, "{}", report(&plan, &removed, deleted)).map_err(failure)?;
        return Ok(());
    }
    message::success(&mut output, terminal, "Uninstalled TMT").map_err(failure)?;
    let mut stderr = tmt_cli_style::stream::stderr();
    let warnings = stderr.terminal();
    for (path, reason) in plan.kept() {
        let _ = message::warning(
            &mut stderr,
            warnings,
            &format!("Kept {}: {reason}", path.display()),
            None,
        );
    }
    if plan.data_exists && !deleted {
        message::hint(
            &mut output,
            terminal,
            &format!(
                "your data is still in {}; tmt uninstall --purge deletes it",
                display(&plan.data)
            ),
        )
        .map_err(failure)?;
    }
    Ok(())
}

/// The fixed order. Returns whether the data directory was deleted.
fn remove(
    plan: &Plan,
    paths: &ConfigPaths,
    environment: &ProviderEnvironment,
    prefix: &Path,
    removed: &mut Vec<PathBuf>,
) -> io::Result<bool> {
    if plan.office_running {
        crate::office_facade::service_control::stop(paths)?;
    }
    for step in &plan.hooks {
        removal::remove_hooks(&plan.data, step)?;
        if let HookStep::Remove(planned) = step {
            removed.push(planned.change.path.clone());
        }
    }
    removed.extend(skill_installation::uninstall(environment, &plan.data)?.removed);
    for (product, _) in &plan.products {
        removed.extend(native_install::remove_product(prefix, *product)?.removed);
    }
    for file in [
        record::path(&plan.data),
        plan.data.join("setup-record.lock"),
    ] {
        match fs::remove_file(&file) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            result => result?,
        }
    }
    if plan.purge && plan.data_exists {
        purge(&plan.data)?;
        removed.push(plan.data.clone());
        return Ok(true);
    }
    Ok(false)
}

/// Deletes the data directory, refusing anything that is not a real
/// directory named like TMT's.
fn purge(data: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(data)?;
    let named = data
        .file_name()
        .is_some_and(|name| name == "tmux-team" || name == ".tmux-team");
    if !metadata.is_dir() || !named {
        return Err(io::Error::other(format!(
            "{} is not TMT's data directory; it was not deleted.",
            data.display()
        )));
    }
    fs::remove_dir_all(data)
}

fn display(path: &Path) -> String {
    value::home_path(path, std::env::home_dir().as_deref())
}

fn present(
    output: &mut impl Write,
    terminal: tmt_cli_style::Terminal,
    plan: &Plan,
) -> io::Result<()> {
    let removals = plan.removals();
    let mut rows = Table::new(&[Column::Fixed, Column::Detail]);
    for (kind, path) in &removals {
        rows.row([
            Cell::styled(*kind, tmt_cli_style::Token::Dim),
            Cell::from(display(path)),
        ]);
    }
    let kept = plan.kept();
    let mut kept_rows = Table::new(&[Column::Detail, Column::Detail]);
    for (path, reason) in &kept {
        kept_rows.row([Cell::from(display(path)), Cell::from(reason)]);
    }
    let data_note = if plan.purge {
        format!(
            "{} and everything in it will be deleted",
            display(&plan.data)
        )
    } else {
        format!(
            "your identities, messages and notes stay in {}",
            display(&plan.data)
        )
    };
    let mut sections = vec![Section {
        title: "remove",
        count: Some(removals.len()),
        rows,
        note: plan.data_exists.then_some(data_note.as_str()),
        hint: None,
    }];
    if !kept.is_empty() {
        sections.push(Section {
            title: "keep",
            count: Some(kept.len()),
            rows: kept_rows,
            note: None,
            hint: None,
        });
    }
    list::write(output, terminal, &sections)
}

fn report(plan: &Plan, removed: &[PathBuf], deleted: bool) -> serde_json::Value {
    json!({
        "removed": removed,
        "kept": plan.kept().into_iter().map(|(path, reason)| json!({"path": path, "reason": reason})).collect::<Vec<_>>(),
        "officeStopped": plan.office_running,
        "data": {"path": plan.data, "deleted": deleted},
    })
}
