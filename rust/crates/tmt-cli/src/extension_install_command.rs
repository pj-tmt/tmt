//! `tmt extension install|upgrade|uninstall|list`: consented installation of
//! the official extensions over the native installer. The product table is
//! fixed; nothing here discovers an extension from archive data or PATH.

mod install;
mod list_upgrade;
mod repair;
mod skills;
pub(crate) mod upgrade_all;

use install::install;
use list_upgrade::listing;
#[cfg(test)]
use list_upgrade::shadowing_in;
use repair::repair_extension;
use skills::{global_dir, settle_skills};
#[cfg(test)]
use skills::{published_document, published_lines};
#[cfg(test)]
use std::os::unix::fs::PermissionsExt;
#[cfg(test)]
use tmt_adapters::skill_installation::OwnedReport;

use crate::{
    consent,
    invocation::{ExtensionInstallRequest, OutputMode},
    output::Failure,
};
use serde_json::{Value, json};
use std::{
    env, fs, io,
    io::Write,
    path::{Path, PathBuf},
};
use tmt_adapters::{
    interrupt::Interrupt,
    native_install::{self, Product},
    skill_installation,
};
use tmt_cli_style::{
    Token,
    list::Section,
    table::{Cell, Column, Table},
    value,
};

const CONSENT: &str = "EXTENSION_CONSENT_REQUIRED";

// Historical products stay recognizable for receipt recovery and removal.
const INSTALLABLE_EXTENSIONS: &[Product] = &[Product::Squad, Product::Remote, Product::Colab];

pub(crate) fn require_installable(product: Product) -> Result<(), Failure> {
    if INSTALLABLE_EXTENSIONS.contains(&product) {
        return Ok(());
    }
    Err(Failure::new(
        "EXTENSION_FROZEN",
        format!(
            "{} is frozen; installation and upgrades are unavailable. Existing installations can still be listed and removed with tmt extension ls and tmt extension rm {}.",
            product.as_str(),
            product.as_str()
        ),
        1,
    ))
}

/// Consent for one change to the user's installation.
fn ask(yes: bool, mode: OutputMode, action: &str) -> Result<bool, Failure> {
    ask_declining(yes, mode, action, "No changes made.")
}

fn ask_declining(
    yes: bool,
    mode: OutputMode,
    action: &str,
    declined: &str,
) -> Result<bool, Failure> {
    consent::ask(
        &mut tmt_cli_style::stream::stdout(mode.json),
        yes,
        mode,
        consent::Consent {
            code: CONSENT,
            refusal: &format!("{action} requires explicit --yes; no changes were made."),
            question: action,
            declined,
        },
        |error| {
            Failure::new("EXTENSION_IO_ERROR", "Could not ask for consent.", 1).caused_by(error)
        },
    )
}

pub fn execute(request: ExtensionInstallRequest, mode: OutputMode) -> io::Result<u8> {
    match run(request, mode) {
        Ok(Some((document, human))) => {
            let mut stdout = tmt_cli_style::stream::stdout(mode.json);
            let terminal = stdout.terminal();
            if mode.json {
                writeln!(stdout, "{document}")?;
            } else {
                human.write(&mut stdout, terminal)?;
            }
            Ok(0)
        }
        Ok(None) => Ok(0),
        Err(error) => error.publish(mode),
    }
}

/// Recognized extension products, including frozen historical installations.
fn extension(name: &str) -> Result<Product, Failure> {
    Product::ALL
        .into_iter()
        .filter(|product| *product != Product::Cli)
        .find(|product| product.as_str() == name)
        .ok_or_else(|| {
            Failure::new(
                "EXTENSION_UNKNOWN",
                format!(
                    "Unknown extension '{name}'. Official extensions: {}.",
                    names()
                ),
                1,
            )
        })
}

fn names() -> String {
    INSTALLABLE_EXTENSIONS
        .iter()
        .map(|product| product.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn prefix(prefix: Option<&str>) -> Result<PathBuf, Failure> {
    match prefix {
        Some(prefix) => Ok(PathBuf::from(prefix)),
        None => native_install::default_install_prefix().map_err(|error| {
            Failure::new("EXTENSION_INSTALL_FAILED", error.to_string(), 1).caused_by(error)
        }),
    }
}

fn repair_required(error: &io::Error) -> Option<&native_install::RepairRequired> {
    error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<native_install::RepairRequired>())
}

fn repair_command(required: &native_install::RepairRequired) -> String {
    let mut command = format!(
        "tmt extension install {} --repair --yes --prefix {}",
        required.product.as_str(),
        crate::output::shell_word(&required.prefix.to_string_lossy())
    );
    if required.requires_archive {
        command.push_str(" --archive '<original-archive>' --manifest '<matching-manifest>'");
    }
    command
}

fn failure(code: &'static str, error: io::Error) -> Failure {
    if let Some(required) = repair_required(&error) {
        let command = repair_command(required);
        return Failure::new("EXTENSION_REPAIR_REQUIRED", format!(
            "Managed {} release {} failed verification: {} No files were changed. Repair this release with: {command}",
            required.product.as_str(), required.version, required), 1).caused_by(error);
    }
    Failure::new(
        code,
        format!("{error} Inspect with: tmt extension ls"),
        if error.kind() == io::ErrorKind::Interrupted {
            130
        } else {
            1
        },
    )
    .caused_by(error)
}

/// A missing command link beside a retained activation is recoverable
/// partial removal, not proof that the extension is gone.
fn installed(product: Product, prefix: &Path) -> Result<bool, Failure> {
    let executable = prefix.join("bin").join(product.executable());
    match fs::symlink_metadata(&executable) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match fs::symlink_metadata(prefix.join(product.namespace()).join("current")) {
                Ok(_) => Err(Failure::new(
                    "EXTENSION_INSTALLATION_INVALID",
                    format!(
                        "{} has an activation but no command link. Finish removal with: tmt extension rm {} --yes",
                        product.as_str(),
                        product.as_str()
                    ),
                    1,
                )),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(failure("EXTENSION_INSTALLATION_INVALID", error)),
            }
        }
        Err(error) => Err(failure("EXTENSION_INSTALLATION_INVALID", error)),
    }
}

fn interruptible(message: &'static str) -> Result<impl FnMut() -> io::Result<()>, Failure> {
    let interrupt = Interrupt::install().map_err(|error| {
        Failure::new("EXTENSION_INSTALL_FAILED", error.to_string(), 1).caused_by(error)
    })?;
    Ok(move || {
        if interrupt.is_interrupted() {
            Err(io::Error::new(io::ErrorKind::Interrupted, message))
        } else {
            Ok(())
        }
    })
}

/// Keep outcomes and aligned lists typed until the style owner renders them.
struct Human(Vec<HumanLine>);

enum HumanLine {
    Plain(String),
    Success(String),
    List(Section<'static>),
}

impl Human {
    fn done(text: String) -> Self {
        let mut lines = text.lines();
        Self(
            std::iter::once(HumanLine::Success(lines.next().unwrap_or_default().into()))
                .chain(lines.map(|line| HumanLine::Plain(line.into())))
                .collect(),
        )
    }

    fn plain(text: String) -> Self {
        Self(
            text.lines()
                .map(|line| HumanLine::Plain(line.into()))
                .collect(),
        )
    }

    fn push(&mut self, line: String) {
        self.0
            .extend(line.lines().map(|line| HumanLine::Plain(line.into())));
    }

    fn push_success(&mut self, line: String) {
        self.0.push(HumanLine::Success(line));
    }

    fn write(&self, output: &mut impl Write, terminal: tmt_cli_style::Terminal) -> io::Result<()> {
        for line in &self.0 {
            match line {
                HumanLine::Plain(line) => writeln!(output, "{line}")?,
                HumanLine::Success(line) => {
                    tmt_cli_style::message::success(output, terminal, line)?
                }
                HumanLine::List(section) => section.write(output, terminal)?,
            }
        }
        Ok(())
    }
}

type Outcome = Option<(Value, Human)>;

struct InstalledExtension {
    document: Value,
    human: Human,
    executable: PathBuf,
    previous: Vec<String>,
    changed: bool,
}

fn run(request: ExtensionInstallRequest, mode: OutputMode) -> Result<Outcome, Failure> {
    match request {
        ExtensionInstallRequest::Install {
            name,
            prefix: selected,
            channel,
            archive,
            manifest,
            yes,
            skills,
            repair,
        } => {
            let product = extension(&name)?;
            require_installable(product)?;
            let prefix = prefix(selected.as_deref())?;
            let action = if repair {
                format!("Repair the {name} extension; retain the damaged release")
            } else {
                format!("Install the verified {name} extension")
            };
            if !ask(yes, mode, &action)? {
                return Ok(None);
            }
            let InstalledExtension {
                mut document,
                human,
                executable,
                previous,
                changed,
            } = if repair {
                repair_extension(product, &prefix, archive.as_deref(), manifest.as_deref())?
            } else {
                install(
                    product,
                    &prefix,
                    channel,
                    archive.as_deref(),
                    manifest.as_deref(),
                )?
            };
            let mut human = human;
            if !repair || changed || skills {
                settle_skills(
                    product,
                    &executable,
                    &previous,
                    skills,
                    Some(mode),
                    &mut document,
                    &mut human,
                )?;
            }
            Ok(Some((document, human)))
        }
        ExtensionInstallRequest::Upgrade {
            name,
            prefix: selected,
            channel,
            to,
            unpin,
            yes,
        } => list_upgrade::upgrade_extension(name, selected, channel, to, unpin, yes, mode),
        ExtensionInstallRequest::Uninstall {
            name,
            prefix: selected,
            yes,
        } => {
            let product = extension(&name)?;
            let prefix = prefix(selected.as_deref())?;
            let global = global_dir()?;
            let environment = skill_installation::ProviderEnvironment::capture()
                .map_err(|error| failure("EXTENSION_SKILLS_FAILED", error))?;
            // Every skill the extension's owner holds goes with its commands:
            // a skill that points at a removed command is broken guidance.
            let owned = skill_installation::owned_by(Some(&environment), &global, product.as_str())
                .map_err(|error| failure("EXTENSION_SKILLS_FAILED", error))?;
            let mut question =
                format!("Remove the {name} extension's commands (releases and data are kept)");
            if !owned.is_empty() {
                question.push_str(&format!(
                    " and its agent skills {} from {}",
                    owned.keys().cloned().collect::<Vec<_>>().join(", "),
                    owned
                        .values()
                        .flatten()
                        .map(|target| target.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if !ask(yes, mode, &question)? {
                return Ok(None);
            }
            let changed = native_install::uninstall_extension(&prefix, product)
                .map_err(|error| failure("EXTENSION_UNINSTALL_FAILED", error))?;
            let skills = skill_installation::remove_owned(Some(&environment), &global, product.as_str(), None)
                .map_err(|failure| {
                    Failure::new(
                        "EXTENSION_SKILLS_FAILED",
                        format!(
                            "Removed the {name} extension's commands, but not all of its agent skills: {failure}"
                        ),
                        1,
                    )
                })?;
            let changed = changed || !skills.removed.is_empty();
            let kept = kept(product, &prefix);
            let mut lines = std::iter::once(if changed {
                format!("Removed the {name} extension's commands.")
            } else {
                format!("{name} was not installed; its commands were already gone.")
            })
            .chain(
                skills
                    .removed
                    .iter()
                    .map(|target| format!("Removed agent skill {}", target.display())),
            )
            .chain(skills.kept.iter().map(|target| {
                format!(
                    "Left {} alone: it no longer points at the {name} skill",
                    target.display()
                )
            }))
            .collect::<Vec<_>>();
            lines.push("Kept:".into());
            lines.extend(kept.iter().map(|(_, how)| format!("  - {how}")));
            let text = lines.join("\n");
            let human = if changed {
                Human::done(text)
            } else {
                Human::plain(text)
            };
            Ok(Some((
                json!({"extension": name, "installed": false, "changed": changed,
                    "skillsRemoved": skills.removed, "skillsKept": skills.kept,
                    "kept": kept.iter().map(|(what, _)| *what).collect::<Vec<_>>()}),
                human,
            )))
        }
        ExtensionInstallRequest::List {
            prefix: selected,
            check,
        } => listing(
            &prefix(selected.as_deref())?,
            check,
            &env::var_os("PATH").unwrap_or_default(),
        )
        .map(|(document, human)| Some((document, human))),
    }
}

/// What uninstall keeps, with how to remove each explicitly. This command
/// never deletes data; the extension's own agent skills are not data.
fn kept(product: Product, prefix: &Path) -> Vec<(&'static str, String)> {
    let name = product.as_str();
    let mut kept = vec![
        (
            "releases",
            format!(
                "releases in {}; delete that folder to reclaim space",
                prefix.join(product.namespace()).join("releases").display()
            ),
        ),
        (
            "hookConsent",
            format!("lifecycle hook consent; withdraw it with: tmt extension hooks disable {name}"),
        ),
    ];
    if product == Product::Office {
        kept.push((
            "officeData",
            "Office data (office.db) and its backups; see their paths with: tmt office storage status, and delete them there only if you no longer need them".to_owned(),
        ));
    }
    kept
}

/// Presentation values from the same installation observation used for JSON.
struct ExtensionListRow {
    name: String,
    status: String,
    update: Option<&'static str>,
    shadowed: Vec<String>,
    /// Dim detail lines under the row, such as the repair for a bad entry.
    notes: Vec<String>,
}

fn extension_list(rows: &[ExtensionListRow], home: Option<&Path>) -> Human {
    let show_update = rows.iter().any(|row| row.update.is_some());
    let mut columns = vec![Column::Name, Column::Detail];
    if show_update {
        columns.push(Column::Fixed);
    }
    let mut table = Table::new(&columns);
    for row in rows {
        let mut cells = vec![Cell::from(&row.name), Cell::from(&row.status)];
        if show_update {
            cells.push(Cell::from(
                row.update
                    .map(|update| format!("update: {update}"))
                    .unwrap_or_default(),
            ));
        }
        table.row(cells);
        for note in &row.notes {
            let mut cells = vec![Cell::from(""), Cell::styled(note, Token::Dim)];
            if show_update {
                cells.push(Cell::from(""));
            }
            table.row(cells);
        }
        for path in &row.shadowed {
            let mut cells = vec![
                Cell::from(""),
                Cell::styled(
                    format!(
                        "shadowed on PATH by {}",
                        value::home_path(Path::new(path), home)
                    ),
                    Token::Dim,
                ),
            ];
            if show_update {
                cells.push(Cell::from(""));
            }
            table.row(cells);
        }
    }
    Human(vec![HumanLine::List(Section {
        title: "extensions",
        count: Some(rows.len()),
        rows: table,
        note: None,
        hint: None,
    })])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn only_official_extensions_are_named_and_the_cli_is_not_one() {
        assert_eq!(extension("squad").unwrap(), Product::Squad);
        assert_eq!(extension("remote").unwrap(), Product::Remote);
        assert_eq!(extension("colab").unwrap(), Product::Colab);
        assert_eq!(extension("office").unwrap(), Product::Office);
        for name in ["cli", "tmt", "sq", "unknown"] {
            let error = extension(name).unwrap_err();
            assert_eq!(error.code, "EXTENSION_UNKNOWN");
            assert!(
                error
                    .message
                    .ends_with("Official extensions: squad, remote, colab.")
            );
        }
    }

    #[test]
    fn shadowing_reports_other_commands_by_canonical_path_without_running_them() {
        let root = std::env::temp_dir().join(format!("tmt-shadow-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let prefix = root.join("prefix");
        let release = prefix.join("lib/tmt-squad/current");
        fs::create_dir_all(&release).unwrap();
        fs::create_dir_all(prefix.join("bin")).unwrap();
        let binary = release.join("tmt-squad");
        // A marker the test can check: executing it would create this file.
        let marker = root.join("executed");
        fs::write(&binary, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        for link in Product::Squad.links() {
            symlink(&binary, prefix.join("bin").join(link)).unwrap();
        }
        // The same file reached through another directory is not shadowing.
        let alias = root.join("alias");
        fs::create_dir_all(&alias).unwrap();
        symlink(prefix.join("bin/tmt-squad"), alias.join("tmt-squad")).unwrap();
        let other = root.join("other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("tmt-sq"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(other.join("tmt-sq"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(other.join("tmt-squad"), "not executable").unwrap();
        let search = env::join_paths([other.clone(), alias, prefix.join("bin")]).unwrap();
        assert_eq!(
            shadowing_in(Product::Squad, &prefix, &search),
            [other.join("tmt-sq").display().to_string()]
        );
        assert!(!marker.exists(), "nothing was executed");
        let _ = fs::remove_dir_all(&root);
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("tmt-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("bin")).unwrap();
        root
    }

    #[test]
    fn an_unmanaged_command_degrades_its_own_entry_and_names_the_path() {
        let prefix = scratch("unmanaged-listing");
        let binary = prefix.join("bin/tmt-remote");
        fs::write(&binary, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        let (document, human) = listing(&prefix, false, prefix.join("empty").as_os_str())
            .expect("one bad entry never fails the whole listing");
        let rows = document["extensions"].as_array().unwrap();
        let names: Vec<_> = rows
            .iter()
            .map(|row| row["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["squad", "remote", "colab"]);
        let remote = &rows[1];
        assert_eq!(remote["status"], "unmanaged");
        assert_eq!(remote["installed"], false);
        assert_eq!(remote["path"], binary.to_string_lossy().as_ref());
        let hint = remote["hint"].as_str().unwrap();
        assert!(hint.contains(&*binary.to_string_lossy()));
        assert!(hint.contains("tmt extension install remote --yes"));
        assert!(!hint.contains("extension ls"), "the repair is not circular");
        for other in [&rows[0], &rows[2]] {
            assert_eq!(other["installed"], false);
            assert!(other.get("status").is_none());
        }
        let mut output = Vec::new();
        human
            .write(&mut output, tmt_cli_style::Terminal::PLAIN)
            .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains(&*binary.to_string_lossy()));
        assert!(text.contains("unmanaged"));
        let _ = fs::remove_dir_all(&prefix);
    }

    #[test]
    fn an_activation_that_cannot_be_read_is_invalid_with_a_removal_hint() {
        let prefix = scratch("invalid-listing");
        let release = prefix.join("lib/tmt-remote");
        fs::create_dir_all(release.join("releases")).unwrap();
        fs::write(release.join("current"), "not a receipt").unwrap();
        let binary = prefix.join("bin/tmt-remote");
        fs::write(&binary, "#!/bin/sh\n").unwrap();
        let (document, _) = listing(&prefix, false, prefix.join("empty").as_os_str())
            .expect("an unreadable entry never fails the whole listing");
        let remote = &document["extensions"][1];
        assert_eq!(remote["status"], "invalid");
        assert_eq!(remote["path"], binary.to_string_lossy().as_ref());
        let hint = remote["hint"].as_str().unwrap();
        assert!(hint.contains("tmt extension rm remote --yes --prefix"));
        assert!(!hint.contains("extension ls"));
        let _ = fs::remove_dir_all(&prefix);
    }

    #[test]
    fn uninstall_names_what_stays_and_how_to_remove_it() {
        let prefix = Path::new("/p");
        let squad = kept(Product::Squad, prefix);
        assert_eq!(
            squad.iter().map(|(what, _)| *what).collect::<Vec<_>>(),
            ["releases", "hookConsent"],
            "the extension's agent skills are removed, not kept"
        );
        assert!(squad[0].1.contains("/p/lib/tmt-squad/releases"));
        assert!(squad[1].1.contains("tmt extension hooks disable squad"));
        let office = kept(Product::Office, prefix);
        assert_eq!(office.last().unwrap().0, "officeData");
        assert!(
            office
                .last()
                .unwrap()
                .1
                .contains("tmt office storage status")
        );
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    use tmt_adapters::skill_installation::OwnedTarget;

    fn published_report() -> OwnedReport {
        OwnedReport {
            published: vec![
                OwnedTarget {
                    name: "tmt-squad".into(),
                    agent: None,
                    target: "/home/ada/.agents/skills/tmt-squad".into(),
                    changed: true,
                    backup: Some("/home/ada/.agents/.tmt-skill-backups/previous".into()),
                },
                OwnedTarget {
                    name: "squad-playbook".into(),
                    agent: None,
                    target: "/opt/agents/skills/squad-playbook".into(),
                    changed: false,
                    backup: None,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn published_skill_json_bytes() {
        let document = json!(published_document(&published_report())).to_string();
        insta::assert_snapshot!(document);
    }

    #[test]
    fn extension_list_json_bytes() {
        // No installed product, network or inherited PATH participates.
        let root = env::temp_dir().join(format!(
            "tmt-extension-list-json-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let result = listing(&root, false, root.join("empty-bin").as_os_str());
        fs::remove_dir_all(&root).unwrap();
        let (document, _) = result.unwrap();
        insta::assert_snapshot!(document.to_string());
    }

    fn render(human: Human, terminal: tmt_cli_style::Terminal) -> String {
        let mut output = Vec::new();
        human.write(&mut output, terminal).unwrap();
        String::from_utf8(output).unwrap()
    }

    fn mixed_extensions() -> Vec<ExtensionListRow> {
        vec![
            ExtensionListRow {
                name: "office".into(),
                status: "not installed".into(),
                update: Some("available"),
                shadowed: Vec::new(),
                notes: Vec::new(),
            },
            ExtensionListRow {
                name: "squad".into(),
                status: "0.1.0-alpha.1 (pinned 0.1.0-alpha.1)".into(),
                update: Some("current"),
                shadowed: vec![
                    "/home/ada/.local/other/bin/tmt-squad".into(),
                    "/opt/other/bin/tmt-sq".into(),
                ],
                notes: Vec::new(),
            },
        ]
    }

    #[test]
    fn extension_list_through_a_pipe() {
        insta::assert_snapshot!(render(
            extension_list(&mixed_extensions(), Some(Path::new("/home/ada"))),
            tmt_cli_style::Terminal::PLAIN
        ));
    }

    #[test]
    fn extension_list_on_a_narrow_terminal() {
        let text = render(
            extension_list(&mixed_extensions(), Some(Path::new("/home/ada"))),
            tmt_cli_style::Terminal {
                color: false,
                width: Some(48),
                theme: None,
            },
        );
        assert!(text.lines().all(|line| line.chars().count() <= 48));
        insta::assert_snapshot!(text);
    }

    #[test]
    fn published_skills_have_success_marks_and_home_paths() {
        let report = published_report();
        let mut human = Human::done("Installed squad 0.1.0-alpha.1.".into());
        for line in published_lines(&report, Some(Path::new("/home/ada"))) {
            human.push_success(line);
        }
        insta::assert_snapshot!(render(human, tmt_cli_style::Terminal::PLAIN));
    }

    #[test]
    fn published_skill_paths_are_escaped_by_the_style_owner() {
        let mut report = published_report();
        report.published[0].target = "/home/ada/skills/hidden\nline".into();
        let mut human = Human::plain(String::new());
        for line in published_lines(&report, Some(Path::new("/home/ada"))) {
            human.push_success(line);
        }
        let text = render(human, tmt_cli_style::Terminal::PLAIN);
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("~/skills/hidden\\nline"));
        assert!(text.lines().all(|line| line.starts_with("✓ ")));
    }
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "Office data (office.db) and its backups; see their paths with: tmt office storage status, and delete them there only if you no longer need them",
        &[", and"],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "lifecycle hook consent; withdraw it with: tmt extension hooks disable {name}",
        &[""],
        &[
            ("{name}", "squad"),
            ("{}", "squad"),
            ("{SUGGESTED_EXTENSION}", "squad"),
        ],
    ),
    crate::cli_style_tests::HintSpec::core(
        "tmt extension install {} --repair --yes --prefix {}",
        &[""],
        &[
            ("tmt extension install {}", "tmt extension install squad"),
            ("{}", "/tmp/hint-prefix"),
        ],
    ),
    crate::cli_style_tests::HintSpec::core(
        "{error} Inspect with: tmt extension ls",
        &[""],
        &[
            ("{name}", "squad"),
            ("{}", "squad"),
            ("{SUGGESTED_EXTENSION}", "squad"),
        ],
    ),
    crate::cli_style_tests::HintSpec::core(
        "{} has an activation but no command link. Finish removal with: tmt extension rm {} --yes",
        &[""],
        &[
            ("{name}", "squad"),
            ("{}", "squad"),
            ("{SUGGESTED_EXTENSION}", "squad"),
        ],
    ),
    crate::cli_style_tests::HintSpec::core(
        "{} is frozen; installation and upgrades are unavailable. Existing installations can still be listed and removed with tmt extension ls and tmt extension rm {}.",
        &[" and tmt ", "."],
        &[
            ("{name}", "squad"),
            ("{}", "squad"),
            ("{SUGGESTED_EXTENSION}", "squad"),
        ],
    ),
];

#[cfg(test)]
pub(crate) use list_upgrade::PRINTED_HINTS as LIST_UPGRADE_HINTS;

#[cfg(test)]
pub(crate) use skills::PRINTED_HINTS as SKILLS_HINTS;

#[cfg(test)]
pub(crate) use upgrade_all::PRINTED_HINTS as UPGRADE_ALL_HINTS;
