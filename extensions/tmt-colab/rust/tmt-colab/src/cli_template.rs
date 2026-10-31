//! Page templates (#3928, #5066): starting pages for common kinds, so an agent starts from markup
//! that already follows the frame and says where each rule is taught. A template is either a
//! compiled one or a page of the space marked as a template (`template save`). `template ls` lists
//! both, `template show` reads one's rules, `template default` picks what `page create` starts from
//! when none is named, and `page create --template NAME|ID` creates a page from one: a copy of its
//! current content that is independent of the template from then on. Every block of a compiled
//! template carries one HTML comment naming its rule and the skill section (the comments are not
//! rendered), and a template sets no width or gap: only the two declarations (`data-width`,
//! `data-gap`) and the layout inside its blocks.
use crate::{cli_management::fail, cli_metadata, core};
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
use tmt_cli_style::{
    CommandSpec, Example, OutputModes,
    list::Section,
    message,
    palette::{Terminal, Token},
    table::{Cell, Column, Table},
};
pub use tmt_colab::templates::{TEMPLATES, Template, find};
use tmt_colab::{
    Result,
    decoder::{Decoder, StructuredKind},
    inspection,
    keyring::{Keyring, Layout},
    page,
    store::Store,
    template_catalog::{self, Catalog, Found, User},
    template_marks,
};

#[cfg(test)]
use crate::SKILL_REFERENCES;
#[cfg(test)]
use tmt_colab::decoder::LayoutKind;

/// The template `page create` was asked to start from: a name, or the ID of a template page.
pub fn chosen(args: &ArgMatches) -> Option<&String> {
    args.get_one::<String>("template")
}

/// `--template` on `page create`: it brings its own layout and source, so it excludes the layout
/// and source options rather than silently overriding them.
pub fn create_arg(command: Command) -> Command {
    command.arg(
        Arg::new("template")
            .long("template")
            .value_name("NAME|ID")
            .conflicts_with_all(["layout", "reason", "file"])
            .help("Start from a template, which sets the layout and the source (tmt colab template ls)"),
    )
}

const TEMPLATE: CommandSpec = CommandSpec {
    name: "template",
    summary: "List, read, save and choose page templates",
    examples: &[
        Example {
            command: "tmt colab template ls",
            note: "See the templates, which one is the default and the layout each sets",
        },
        Example {
            command: "tmt colab template save 10000000-0000-4000-8000-000000000001 --name weekly-review",
            note: "Make a page a template of this space",
        },
    ],
    outputs: OutputModes::HumanAndJson,
    details: "A template is a starting page for a common kind: TMT's own (starter, report, review, compare, board, gallery) or a page of this space that someone saved. page create --template NAME|ID copies the template's current content and layout into a new page, then the page is independent of it, so choose a template instead of --layout and --file. A page created without --template starts from the space default (starter until template default says otherwise). Each block of a template has an HTML comment that names its rule; the comments are not shown on the page. Replace the placeholder text and keep the structure and the comments' rules.",
};

const LS: CommandSpec = CommandSpec {
    name: "ls",
    summary: "List every template with its layout",
    examples: &[Example {
        command: "tmt colab template ls --json",
        note: "Each template's name, layout, summary and whether it is the default",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Prints TMT's templates and this space's, each with the layout it sets and what it is for, and marks the default. Reads only; without a local space it lists TMT's templates.",
};

const SHOW: CommandSpec = CommandSpec {
    name: "show",
    summary: "Show one template: its layout, source and rules",
    examples: &[Example {
        command: "tmt colab template show weekly-review",
        note: "Read the rules to follow before writing a page from it",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Prints the template's layout, where it comes from, how many pages were created from it and its numbered rules: the comments of its source, in order. Read the rules before writing a page from the template; a user template's page source is the full text (tmt colab page read PAGE).",
};

const DEFAULT: CommandSpec = CommandSpec {
    name: "default",
    summary: "Show or set the template page create starts from",
    examples: &[
        Example {
            command: "tmt colab template default weekly-review",
            note: "Start pages from weekly-review when no template is named",
        },
        Example {
            command: "tmt colab template default --reset",
            note: "Go back to TMT's starter",
        },
    ],
    outputs: OutputModes::HumanAndJson,
    details: "One default for the space, kept on this machine (there is no per-project default). Without arguments it shows the current default. Removing the default template, or deleting its page, returns the default to starter and says so.",
};

const SAVE: CommandSpec = CommandSpec {
    name: "save",
    summary: "Make a page a template of this space",
    examples: &[Example {
        command: "tmt colab template save 10000000-0000-4000-8000-000000000001 --name weekly-review --description 'Shipped, open, risks, next week'",
        note: "Save the page as weekly-review",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Marks the page as a template; its template ID is the page's ID. The name is 1 to 40 lowercase letters, digits and hyphens, unique in the space and not a TMT template name. Saving a page again under its own template renames it or changes its description. The page stays an ordinary page (it can be commented on and edited); only a person who can edit it can save or remove it. Its questions must be registered, as for a write. A dashboard cannot be a template.",
};

const REMOVE: CommandSpec = CommandSpec {
    name: "remove",
    summary: "Stop a page being a template",
    examples: &[Example {
        command: "tmt colab template remove weekly-review",
        note: "The page stays; pages created from it are unchanged",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Removes the template mark only: the page and every page created from it stay as they are. If it was the space default, the default returns to starter. TMT's own templates cannot be removed.",
};

fn selector() -> Arg {
    Arg::new("template")
        .required(true)
        .index(1)
        .value_name("NAME|ID")
        .help("A template name, or the ID of its page (see template ls)")
}

pub fn command() -> Command {
    tmt_cli_style::command(&TEMPLATE)
        .mut_arg("json", |arg| arg.global(true))
        .subcommand_required(true)
        .subcommand(tmt_cli_style::command(&LS).alias("list"))
        .subcommand(tmt_cli_style::command(&SHOW).arg(selector()))
        .subcommand(
            tmt_cli_style::command(&DEFAULT)
                .arg(selector().required(false))
                .arg(
                    Arg::new("reset")
                        .long("reset")
                        .action(ArgAction::SetTrue)
                        .conflicts_with("template")
                        .help("Return the default to TMT's starter"),
                ),
        )
        .subcommand(
            tmt_cli_style::command(&SAVE)
                .arg(crate::cli_grammar::page())
                .arg(
                    Arg::new("name")
                        .long("name")
                        .required(true)
                        .value_name("NAME")
                        .help("1 to 40 lowercase letters, digits and hyphens"),
                )
                .arg(
                    Arg::new("description")
                        .long("description")
                        .value_name("TEXT")
                        .help("One line of at most 200 bytes; defaults to the current one, else the page title"),
                ),
        )
        .subcommand(tmt_cli_style::command(&REMOVE).arg(selector()))
}

/// What `page create` starts from, resolved before any state is initialized or any source is
/// read.
pub struct Start {
    /// The template's name, as the receipt says it.
    pub name: String,
    /// The template's page, for a user template.
    pub page: Option<String>,
    pub layout: tmt_colab::decoder::PageLayout,
    /// The template was not named: it is the space default.
    pub by_default: bool,
    source: StartSource,
}
enum StartSource {
    BuiltIn(&'static Template),
    /// The template page's source, as read.
    User(String),
}
impl Start {
    /// The new page's source for its creator, when it has one: the template with its agents set to
    /// that creator, or unassigned for an anonymous page.
    pub(crate) fn source(&self, metadata: &crate::cli_metadata::CreateMetadata) -> Result<String> {
        match &self.source {
            StartSource::BuiltIn(template) => {
                crate::cli_metadata::template_source(template, metadata)
            }
            StartSource::User(source) => {
                let creator = metadata
                    .selected
                    .as_ref()
                    .and_then(|selected| selected.agent["name"].as_str());
                tmt_colab::templates::rewrite_question_agents(source, creator).ok_or_else(|| {
                    template_catalog::refusal(
                        "COLAB_TEMPLATE_UNREADABLE",
                        format!(
                            "The template {} has question blocks that cannot be copied; nothing was created",
                            self.name
                        ),
                        Some(format!("tmt colab template show {}", self.name)),
                    )
                    .into()
                })
            }
        }
    }
}

/// The template `page create` starts from: the one named, else, when no layout or file is given,
/// the space default. `None` leaves the layout and source to the other options.
pub fn start(root: &Path, args: &ArgMatches) -> Result<Option<Start>> {
    let named = chosen(args);
    let by_default = named.is_none()
        && ["layout", "reason", "file"]
            .iter()
            .all(|option| args.try_get_raw(option).ok().flatten().is_none());
    if named.is_none() && !by_default {
        return Ok(None);
    }
    let settings = tmt_colab::settings::read_or_default(root);
    let from_builtin = |template: &'static Template| Start {
        name: template.name.to_owned(),
        page: None,
        layout: template.layout(),
        by_default,
        source: StartSource::BuiltIn(template),
    };
    // A compiled template needs no state, so a first create works before the space exists.
    match (named, settings.default_template()) {
        (Some(name), _) if find(name).is_some() => {
            return Ok(Some(from_builtin(find(name).expect("found above"))));
        }
        (None, None) => return Ok(Some(from_builtin(find("starter").expect("compiled")))),
        _ => {}
    }
    let mut space = Space::at(root.to_path_buf())?;
    let catalog = space.catalog()?;
    let (found, gone) = match named {
        Some(name) => (catalog.find(name)?, None),
        None => {
            let default = template_catalog::default(&catalog, settings.default_template())?;
            (default.found, default.gone)
        }
    };
    if let Some(gone) = &gone {
        let mut warnings = tmt_cli_style::stream::stderr();
        let terminal = warnings.terminal();
        gone_warning(&mut warnings, terminal, gone)?;
    }
    match found {
        Found::BuiltIn(template) => Ok(Some(from_builtin(template))),
        Found::User(user) => {
            let open = space
                .open
                .as_mut()
                .expect("a user template comes from a space");
            let content =
                template_catalog::read_user(&open.store, &open.key, &mut open.decoder, &user)?;
            let layout = content.layout.ok_or_else(|| {
                template_catalog::refusal(
                    "COLAB_TEMPLATE_NEEDS_LAYOUT",
                    format!("The template {} names no layout", user.name),
                    Some(format!(
                        "tmt colab page layout {} column",
                        catalog.short_id(&user.page)
                    )),
                )
            })?;
            Ok(Some(Start {
                name: user.name,
                page: Some(user.page),
                layout,
                by_default,
                source: StartSource::User(content.source),
            }))
        }
    }
}

/// A page that is gone can no longer be the space default template: the setting returns to starter.
/// The effect already happened, so a setting that cannot be written changes nothing the caller
/// reports; the next `page create` falls back to starter on its own and says so. True when the
/// default was this page and is now `starter`.
pub fn forget_default(root: &Path, page: &str) -> bool {
    let settings = tmt_colab::settings::read_or_default(root);
    settings.default_template() == Some(page)
        && tmt_colab::settings::set_default_template(root, None).is_ok()
}

/// The space a template command reads: its place, and what it holds, when there is one.
struct Space {
    root: PathBuf,
    open: Option<Open>,
}
struct Open {
    layout: Layout,
    key: Keyring,
    store: Store,
    decoder: Decoder,
}
impl Space {
    /// Resolves the data root through Core; a command that only reads falls back to TMT's templates
    /// when there is no space, and says so when Core cannot be reached.
    fn read(needed: bool) -> Result<(Self, Option<String>)> {
        let root = match core::data_root(&AtomicBool::new(false)) {
            Ok(root) => root,
            Err(error) if needed => return Err(error),
            Err(error) => {
                return Ok((
                    Self {
                        root: PathBuf::new(),
                        open: None,
                    },
                    Some(error.to_string()),
                ));
            }
        };
        Ok((Self::at(root)?, None))
    }

    /// The space under a data root, when one exists: nothing is created.
    fn at(root: PathBuf) -> Result<Self> {
        let open = match Layout::existing(&root)? {
            None => None,
            Some(layout) => {
                let key = Keyring::read(&layout)?;
                let store = Store::read(&layout)?;
                store.require_current_schema()?;
                Some(Open {
                    layout,
                    key,
                    store,
                    decoder: Decoder::new(std::env::current_exe()?)?,
                })
            }
        };
        Ok(Self { root, open })
    }

    fn catalog(&mut self) -> Result<Catalog> {
        match &mut self.open {
            None => Ok(Catalog::empty()),
            Some(open) => {
                template_catalog::catalog(&open.layout, &open.store, &open.key, &mut open.decoder)
            }
        }
    }

    fn settings(&self) -> tmt_colab::settings::ColabSettings {
        if self.root.as_os_str().is_empty() {
            Default::default()
        } else {
            tmt_colab::settings::read_or_default(&self.root)
        }
    }
}

/// The space for a command that writes, opened under the write owner's lock so that what it checks
/// (a name is free, a page is the template it is said to be) is what it then writes.
fn write_space() -> Result<(Space, Option<nix::fcntl::Flock<std::fs::File>>)> {
    let root = core::data_root(&AtomicBool::new(false))?;
    let layout = Layout::existing(&root)?.ok_or(page::Fault::Missing)?;
    let lock = crate::write_owner(&layout)?;
    Ok((Space::at(root)?, lock))
}

pub fn run(args: &ArgMatches) -> Result<()> {
    let (name, args) = args.subcommand().expect("required template command");
    match name {
        "ls" => ls(args),
        "show" => show(args),
        "default" => default(args),
        "save" => save(args),
        "remove" => remove(args),
        _ => unreachable!("clap admits only the declared template commands"),
    }
}

fn layout_name(found: &Found) -> &str {
    match found {
        Found::BuiltIn(template) => template.kind.as_str(),
        Found::User(user) => &user.layout,
    }
}

/// Indented `key  value` rows under a line of their own, as the receipts of this command draw them.
fn rows(output: &mut impl Write, terminal: Terminal, fields: &[(&str, String)]) -> Result<()> {
    let mut table = Table::new(&[Column::Fixed, Column::Detail]);
    for (key, value) in fields {
        table.row([Cell::styled(key, Token::Dim), value.into()]);
    }
    table.write(output, terminal)?;
    Ok(())
}

/// The warning that the chosen default is gone, once, where a command resolves the default.
fn gone_warning(output: &mut impl Write, terminal: Terminal, gone: &str) -> Result<()> {
    message::warning(
        output,
        terminal,
        &format!("the default template {gone} no longer exists; page create uses starter"),
        Some("tmt colab template default --reset"),
    )?;
    Ok(())
}

fn ls(args: &ArgMatches) -> Result<()> {
    let json_output = args.get_flag("json");
    let (mut space, unavailable) = Space::read(false)?;
    let catalog = space.catalog()?;
    let settings = space.settings();
    let default = template_catalog::default(&catalog, settings.default_template())?;
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        let mut templates: Vec<Value> = TEMPLATES
            .iter()
            .map(|template| {
                json!({
                    "name": template.name,
                    "layout": template.kind.as_str(),
                    "summary": template.summary,
                    "source": "builtIn",
                    "default": default.found.name() == template.name,
                    "usedBy": catalog.used_by(template.name),
                })
            })
            .collect();
        templates.extend(catalog.user_templates().map(|user| {
            let mut value = json!({
                "name": user.name,
                "id": user.page,
                "layout": user.layout,
                "summary": user.description,
                "source": "space",
                "default": matches!(&default.found, Found::User(found) if found.page == user.page),
                "usedBy": catalog.used_by(&user.name),
            });
            if let Some(author) = &user.author {
                value["author"] = json!(author);
            }
            value
        }));
        let mut value = json!({"default": default.found.name(), "templates": templates});
        if let Some(gone) = &default.gone {
            value["defaultGone"] = json!(gone);
        }
        writeln!(output, "{value}")?;
        return Ok(());
    }
    let terminal = output.terminal();
    let columns = [Column::Fixed, Column::Fixed, Column::Detail, Column::Fixed];
    let mut built_in = Table::new(&columns);
    for template in &TEMPLATES {
        built_in.row([
            Cell::styled(template.name, Token::Title),
            Cell::styled(template.kind.as_str(), Token::Dim),
            Cell::from(template.summary),
            if default.found.name() == template.name {
                Cell::styled("default", Token::Accent)
            } else {
                Cell::from("")
            },
        ]);
    }
    let mut space_rows = Table::new(&columns);
    let mut users = 0;
    for user in catalog.user_templates() {
        users += 1;
        let by = user
            .author
            .as_deref()
            .map_or(String::new(), |author| format!(" · by {author}"));
        space_rows.row([
            Cell::styled(&user.name, Token::Title),
            Cell::styled(&user.layout, Token::Dim),
            Cell::from(user.description.as_str()),
            if matches!(&default.found, Found::User(found) if found.page == user.page) {
                Cell::styled("default", Token::Accent)
            } else {
                Cell::styled(format!("{}{by}", catalog.short_id(&user.page)), Token::Dim)
            },
        ]);
    }
    tmt_cli_style::list::write(
        &mut output,
        terminal,
        &[
            Section {
                title: "built-in",
                count: Some(TEMPLATES.len()),
                rows: built_in,
                note: None,
                hint: None,
            },
            Section {
                title: "this space",
                count: Some(users),
                rows: space_rows,
                note: (users == 0).then_some("none yet: tmt colab template save PAGE --name NAME"),
                hint: Some("tmt colab page create --title \"…\" --template NAME"),
            },
        ],
    )?;
    let mut warnings = tmt_cli_style::stream::stderr();
    let terminal = warnings.terminal();
    if let Some(gone) = &default.gone {
        gone_warning(&mut warnings, terminal, gone)?;
    }
    if let Some(reason) = unavailable {
        message::warning(
            &mut warnings,
            terminal,
            &format!("this space's templates are not listed: {reason}"),
            None,
        )?;
    }
    Ok(())
}

/// How long ago, as `show` writes it.
fn age(updated_ms: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(updated_ms, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        });
    crate::cli_management::last_edit_text(updated_ms, now)
}

fn show(args: &ArgMatches) -> Result<()> {
    let json_output = args.get_flag("json");
    let (mut space, unavailable) = Space::read(false)?;
    let wanted = args
        .get_one::<String>("template")
        .expect("required template");
    let catalog = space.catalog()?;
    let found = match catalog.find(wanted) {
        Err(error) if unavailable.is_some() => {
            return Err(fail(
                "COLAB_TEMPLATE_UNKNOWN",
                &format!(
                    "{error}; the space's own templates could not be read: {}",
                    unavailable.unwrap_or_default()
                ),
            ));
        }
        found => found?,
    };
    let default = template_catalog::default(&catalog, space.settings().default_template())?;
    let is_default = default.found.name() == found.name();
    let (source_text, origin, id, last_edit) = match (&found, &mut space.open) {
        (Found::BuiltIn(template), _) => {
            (template.source()?, "built in (TMT)".to_owned(), None, None)
        }
        (Found::User(user), Some(open)) => {
            let content =
                template_catalog::read_user(&open.store, &open.key, &mut open.decoder, user)?;
            let row = inspection::catalog(&open.store, &open.key)?["pages"]
                .as_array()
                .and_then(|pages| {
                    pages
                        .iter()
                        .find(|page| page["pageId"] == user.page.as_str())
                })
                .cloned();
            let last_edit = match row {
                Some(row) => {
                    inspection::detail(&open.store, &open.key, &row)?["lastUpdateAtMs"].as_u64()
                }
                None => None,
            };
            let by = user
                .author
                .as_deref()
                .map_or(String::new(), |author| format!(" · by {author}"));
            (
                content.source,
                format!("this space{by}"),
                Some(user.page.clone()),
                last_edit,
            )
        }
        (Found::User(_), None) => unreachable!("a user template comes from an open space"),
    };
    let comments = template_catalog::rules(&source_text);
    let used = catalog.used_by(found.name());
    if json_output {
        let mut value = json!({
            "name": found.name(),
            "layout": layout_name(&found),
            "source": if id.is_some() { "space" } else { "builtIn" },
            "default": is_default,
            "usedBy": used,
            "rules": comments,
        });
        if let Found::User(user) = &found {
            value["id"] = json!(user.page);
            value["summary"] = json!(user.description);
            if let Some(author) = &user.author {
                value["author"] = json!(author);
            }
        }
        if let Found::BuiltIn(template) = &found {
            value["summary"] = json!(template.summary);
        }
        value["revision"] = json!(template_marks::revision_of(&source_text));
        if let Some(ms) = last_edit {
            value["lastUpdateAtMs"] = json!(ms);
        }
        let mut output = tmt_cli_style::stream::stdout(true);
        writeln!(output, "{value}")?;
        return Ok(());
    }
    let mut output = tmt_cli_style::stream::stdout(false);
    let terminal = output.terminal();
    writeln!(
        output,
        "{}",
        terminal.paint(Token::Title, &tmt_cli_style::table::escape(found.name()))
    )?;
    let mut fields: Vec<(&str, String)> = Vec::new();
    if let Found::User(user) = &found {
        fields.push(("id", catalog.short_id(&user.page).to_owned()));
    }
    fields.push(("layout", layout_name(&found).to_owned()));
    fields.push((
        "source",
        match last_edit {
            Some(ms) => format!("{origin} · {}", age(ms)),
            None => origin,
        },
    ));
    fields.push((
        "used by",
        format!("{used} {}", if used == 1 { "page" } else { "pages" }),
    ));
    if comments.is_empty() {
        fields.push(("rules", "none".to_owned()));
    }
    for (index, comment) in comments.iter().enumerate() {
        fields.push((
            if index == 0 { "rules" } else { "" },
            format!("{} {}", index + 1, shorten(comment)),
        ));
    }
    rows(&mut output, terminal, &fields)?;
    match &found {
        Found::User(user) => message::hint(
            &mut output,
            terminal,
            &format!(
                "tmt colab page read {} shows the full source",
                catalog.short_id(&user.page)
            ),
        )?,
        Found::BuiltIn(template) => message::hint(
            &mut output,
            terminal,
            &format!(
                "tmt colab page create --title \"…\" --template {}",
                template.name
            ),
        )?,
    }
    if let Some(gone) = &default.gone {
        let mut warnings = tmt_cli_style::stream::stderr();
        let terminal = warnings.terminal();
        gone_warning(&mut warnings, terminal, gone)?;
    }
    Ok(())
}

/// A rule on one line of a screen: long comments end in an ellipsis; `page read` has the whole.
fn shorten(comment: &str) -> String {
    const LIMIT: usize = 110;
    if comment.chars().count() <= LIMIT {
        return comment.to_owned();
    }
    let mut cut: String = comment.chars().take(LIMIT - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

fn default(args: &ArgMatches) -> Result<()> {
    let json_output = args.get_flag("json");
    let wanted = args.get_one::<String>("template");
    let reset = args.get_flag("reset");
    let (mut space, _) = Space::read(wanted.is_some() || reset)?;
    let catalog = space.catalog()?;
    let mut output = tmt_cli_style::stream::stdout(json_output);
    let terminal = output.terminal();
    if wanted.is_none() && !reset {
        let default = template_catalog::default(&catalog, space.settings().default_template())?;
        if json_output {
            let mut value = json!({"default": default.found.name()});
            if let Found::User(user) = &default.found {
                value["id"] = json!(user.page);
            }
            if let Some(gone) = &default.gone {
                value["defaultGone"] = json!(gone);
            }
            writeln!(output, "{value}")?;
        } else {
            let source = match &default.found {
                Found::BuiltIn(_) => "TMT".to_owned(),
                Found::User(user) => format!("this space · {}", catalog.short_id(&user.page)),
            };
            rows(
                &mut output,
                terminal,
                &[("default", format!("{} ({source})", default.found.name()))],
            )?;
            if let Some(gone) = &default.gone {
                gone_warning(&mut output, terminal, gone)?;
            }
        }
        return Ok(());
    }
    let found = match wanted {
        Some(wanted) => catalog.find(wanted)?,
        None => Found::BuiltIn(find("starter").expect("starter is compiled")),
    };
    // The setting names a user template by its page, which a rename keeps; a compiled one by name,
    // and `starter` is the absence of a setting.
    let setting = match &found {
        Found::BuiltIn(template) if template.name == "starter" => None,
        Found::BuiltIn(template) => Some(template.name),
        Found::User(user) => Some(user.page.as_str()),
    };
    tmt_colab::settings::set_default_template(&space.root, setting)?;
    if json_output {
        let mut value = json!({"default": found.name(), "changed": true});
        if let Found::User(user) = &found {
            value["id"] = json!(user.page);
        }
        writeln!(output, "{value}")?;
        return Ok(());
    }
    message::success(
        &mut output,
        terminal,
        &format!(
            "Set the space default template to {}{}",
            found.name(),
            if setting.is_none() { " (TMT)" } else { "" }
        ),
    )?;
    if setting.is_some() {
        message::hint(
            &mut output,
            terminal,
            "tmt colab template default --reset returns to starter",
        )?;
    }
    Ok(())
}

/// A description for a page being saved: its title, folded into one trimmed line within the bound.
fn fit_description(title: &str) -> String {
    let line = title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut end = line.len().min(template_marks::DESCRIPTION_BYTES);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].trim().to_owned()
}

/// A structured template edit on one page through the owner: the same publication as every other
/// metadata command, a no-op when the page already says it.
fn commit(
    mut space: Space,
    lock: Option<nix::fcntl::Flock<std::fs::File>>,
    page_id: &str,
    action: &str,
    body: &Value,
    expected: &str,
) -> Result<page::Receipt> {
    let open = space.open.as_mut().expect("a template edit has a space");
    let display_agent = core::caller_snapshot().map(|caller| {
        let mut agent = json!({"name": caller.name});
        if let Some(id) = caller.agent_id {
            agent["id"] = json!(id);
        }
        agent
    });
    let prepared = page::prepare_structured_publication(
        &open.store,
        &open.key,
        page_id,
        page::StructuredAction {
            kind: StructuredKind::Template,
            action,
            body,
            expected_revision: expected,
            display_agent,
            operation_id: None,
        },
        &mut open.decoder,
        &cli_metadata::now,
    );
    let Open {
        layout,
        key,
        store,
        mut decoder,
    } = space.open.take().expect("a template edit has a space");
    let closed = store.close();
    let prepared = prepared?;
    closed?;
    crate::publish_structured_owned(
        &layout,
        &key,
        page_id,
        prepared,
        &mut decoder,
        cli_metadata::now,
        lock.as_ref(),
    )
}

fn save(args: &ArgMatches) -> Result<()> {
    let json_output = args.get_flag("json");
    let (mut space, lock) = write_space()?;
    let name = args.get_one::<String>("name").expect("required name");
    let operand = args.get_one::<String>("page").expect("required page");
    let catalog = space.catalog()?;
    let open = space.open.as_mut().ok_or(page::Fault::Missing)?;
    let page_id = crate::cli_management::resolve_page(&open.store, &open.key, operand)?;
    template_catalog::admit_name(&catalog, name, &page_id)?;
    let content = template_catalog::read_page(&open.store, &open.key, &mut open.decoder, &page_id)?;
    // A saved template is register-clean, as a write is: its copies register what it registered.
    tmt_colab::question_registry::admit_source(&content.source, &content.meta, &page_id, None)?;
    // A page created from the template takes its layout, and every created page has one.
    if content.layout.is_none() {
        return Err(template_catalog::refusal(
            "COLAB_TEMPLATE_NEEDS_LAYOUT",
            format!(
                "Page {} names no layout, so a page created from it would have none",
                catalog.short_id(&page_id)
            ),
            Some(format!(
                "tmt colab page layout {} column",
                catalog.short_id(&page_id)
            )),
        ));
    }
    let description = match args.get_one::<String>("description") {
        Some(description) => description.clone(),
        None => template_marks::mark_of_meta(&content.meta)
            .map(|(_, current)| current.to_owned())
            .unwrap_or_else(|| fit_description(&content.title)),
    };
    if !template_marks::valid_description(&description) {
        return Err(template_catalog::refusal(
            "COLAB_TEMPLATE_DESCRIPTION_INVALID",
            format!(
                "A description is one trimmed line of at most {} bytes",
                template_marks::DESCRIPTION_BYTES
            ),
            None,
        ));
    }
    let rules = template_catalog::rules(&content.source).len();
    let layout = content
        .layout
        .as_ref()
        .map_or("freeform", |layout| layout.kind.as_str())
        .to_owned();
    let short = catalog.short_id(&page_id).to_owned();
    let body = json!({"name": name, "description": description});
    let receipt = commit(
        space,
        lock,
        &page_id,
        "template.set",
        &body,
        &content.revision,
    )?;
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(
            output,
            "{}",
            json!({
                "pageId": page_id,
                "name": name,
                "description": description,
                "layout": layout,
                "rules": rules,
                "revision": receipt.revision,
                "changed": receipt.changed,
            })
        )?;
        return Ok(());
    }
    let terminal = output.terminal();
    message::success(
        &mut output,
        terminal,
        &if receipt.changed {
            format!("Saved page {short} as template {name}")
        } else {
            format!("Page {short} is already template {name}")
        },
    )?;
    rows(
        &mut output,
        terminal,
        &[
            (
                "rules",
                format!(
                    "{rules} block {}",
                    if rules == 1 { "comment" } else { "comments" }
                ),
            ),
            ("layout", layout),
        ],
    )?;
    message::hint(
        &mut output,
        terminal,
        &format!("tmt colab page create --title \"Week 43\" --template {name}"),
    )?;
    Ok(())
}

fn remove(args: &ArgMatches) -> Result<()> {
    let json_output = args.get_flag("json");
    let (mut space, lock) = write_space()?;
    let wanted = args
        .get_one::<String>("template")
        .expect("required template");
    let catalog = space.catalog()?;
    let user: User = match catalog.find(wanted)? {
        Found::User(user) => user,
        Found::BuiltIn(template) => {
            return Err(template_catalog::refusal(
                "COLAB_TEMPLATE_BUILT_IN",
                format!(
                    "{} is a built-in template and cannot be removed",
                    template.name
                ),
                Some("tmt colab template default --reset".to_owned()),
            ));
        }
    };
    let open = space.open.as_mut().ok_or(page::Fault::Missing)?;
    let content = template_catalog::read_user(&open.store, &open.key, &mut open.decoder, &user)?;
    let used = catalog.used_by(&user.name);
    let short = catalog.short_id(&user.page).to_owned();
    let settings = space.settings();
    let root = space.root.clone();
    let was_default = settings
        .default_template()
        .is_some_and(|chosen| chosen == user.page || chosen == user.name);
    let receipt = commit(
        space,
        lock,
        &user.page,
        "template.clear",
        &json!({}),
        &content.revision,
    )?;
    if was_default {
        tmt_colab::settings::set_default_template(&root, None)?;
    }
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(
            output,
            "{}",
            json!({
                "pageId": user.page,
                "name": user.name,
                "usedBy": used,
                "defaultReset": was_default,
                "revision": receipt.revision,
                "changed": receipt.changed,
            })
        )?;
        return Ok(());
    }
    let terminal = output.terminal();
    message::success(
        &mut output,
        terminal,
        &format!(
            "Removed template {}; page {short} stays as a page",
            user.name
        ),
    )?;
    if was_default {
        message::warning(
            &mut output,
            terminal,
            &format!(
                "{} was the space default; the default is now starter",
                user.name
            ),
            None,
        )?;
    }
    rows(
        &mut output,
        terminal,
        &[("pages created from it", format!("{used}, unchanged"))],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_page_check::{Attachments, inspect};

    fn reference(topic: &str) -> &'static str {
        SKILL_REFERENCES
            .iter()
            .find(|(name, _)| *name == topic)
            .unwrap_or_else(|| panic!("no skill topic {topic}"))
            .1
    }

    #[test]
    fn the_catalog_is_the_six_documented_templates_with_their_layouts() {
        let listed: Vec<(&str, &str)> = TEMPLATES
            .iter()
            .map(|template| (template.name, template.kind.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("starter", "column"),
                ("report", "column"),
                ("review", "column"),
                ("compare", "column"),
                ("board", "full"),
                ("gallery", "column")
            ]
        );
        for template in &TEMPLATES {
            assert!(find(template.name).is_some());
            assert!(template.layout().reason.is_none());
        }
        assert!(find("freeform").is_none());
    }

    #[test]
    fn every_template_renders_checks_clean_and_sets_no_width_or_gap() {
        for template in &TEMPLATES {
            let source = template.source().unwrap();
            assert!(
                !source.contains("{{"),
                "{} has an open placeholder",
                template.name
            );
            let report = inspect(&source, Attachments::NewPage).unwrap();
            assert!(
                report.findings.is_empty(),
                "{} fails page check: {:?}",
                template.name,
                report.findings
            );
            assert!(
                source.len() < 8 * 1024,
                "{} is a starting point",
                template.name
            );
            // The base look is the page-style starter, minus the rule that frames a canvas page:
            // on a column or full page Colab sets the width and the gaps.
            assert!(
                source.contains("@layer tmt-starter"),
                "{} has the base look in the starter layer",
                template.name
            );
            // Type and rhythm are the frame's editorial layer (#5794); the starter adds its glue.
            assert!(
                source.contains(".muted { color: var(--tmt-ui-color-secondary); }"),
                "{} has the starter's glue",
                template.name
            );
            assert!(!source.contains("max-width: 1280px"), "{}", template.name);
            // Only the two declarations change the frame; the CSS inside a block is the author's.
            // The comments may name these properties to say they are not for the author to set.
            let own: String = template
                .source
                .lines()
                .filter(|line| !line.starts_with("<!--"))
                .collect();
            for forbidden in ["max-width", "min-width", "margin", "width:", "gap:"] {
                assert!(
                    !own.contains(forbidden),
                    "{} sets {forbidden}",
                    template.name
                );
            }
        }
    }

    #[test]
    fn a_page_gets_fresh_ids_each_time_and_one_id_per_placeholder_name() {
        for template in &TEMPLATES {
            let source =
                tmt_colab::templates::source_for_creator(&template.source().unwrap(), None);
            assert!(tmt_colab::templates::admits(template.name, &source, None));
            let report = tmt_colab::page_check::check(&source).unwrap();
            assert!(
                report
                    .question_sources
                    .iter()
                    .all(|question| question.agent.is_empty())
            );
        }
        let review = find("review").unwrap();
        let ids = |source: &str| -> Vec<String> {
            source
                .split("data-id=\"")
                .skip(1)
                .map(|rest| rest.split('"').next().unwrap().to_owned())
                .collect()
        };
        let first = ids(&review.source().unwrap());
        let second = ids(&review.source().unwrap());
        assert_eq!(first.len(), 4);
        for id in &first {
            tmt_colab_model::values::generated_id(id).unwrap();
        }
        let mut unique = first.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), first.len(), "an ID is unique on its page");
        assert!(first.iter().all(|id| !second.contains(id)), "IDs are fresh");
        // A name that appears twice would render as one ID; the catalog test above forbids open
        // placeholders, and this checks the pairing directly.
        let twice = Template {
            name: "x",
            summary: "x",
            kind: LayoutKind::Column,
            source: "{{uuid:a}} {{uuid:b}} {{uuid:a}}",
        };
        let parts: Vec<String> = twice
            .source()
            .unwrap()
            .split(' ')
            .map(str::to_owned)
            .collect();
        assert_eq!(parts[0], parts[2]);
        assert_ne!(parts[0], parts[1]);
    }

    #[test]
    fn every_block_has_one_comment_that_points_at_real_skill_sections() {
        for template in &TEMPLATES {
            let source = template.source().unwrap();
            let mut lines = source.lines();
            // The first comment is the page's: its layout, which has no block of its own.
            let header = lines.next().unwrap();
            // The e2e spec reads the layout from this line, so it must be the catalog's.
            assert!(
                header.starts_with(&format!("<!-- layout: {}.", template.kind.as_str())),
                "{}: {header}",
                template.name
            );
            let mut comments = vec![header];
            let mut previous_was_comment = false;
            let mut blocks = 0;
            for line in lines {
                // A block, or a style rule, starts at column 0; indented lines are inside one.
                if !line.starts_with('<') || line.starts_with("</") {
                    continue;
                }
                if line.starts_with("<!--") {
                    assert!(
                        !previous_was_comment,
                        "{}: two comments in a row before {line}",
                        template.name
                    );
                    assert!(line.ends_with("-->"), "{}: one-line comment", template.name);
                    previous_was_comment = true;
                    comments.push(line);
                } else {
                    assert!(
                        previous_was_comment,
                        "{}: a block without its comment: {line}",
                        template.name
                    );
                    previous_was_comment = false;
                    blocks += 1;
                }
            }
            assert!(blocks >= 3, "{} is a starting page", template.name);
            assert!(
                !previous_was_comment,
                "{}: a comment with no block",
                template.name
            );
            for comment in comments {
                let comment = comment
                    .trim_start_matches("<!--")
                    .trim_end_matches("-->")
                    .trim();
                assert!(
                    comment.contains("tmt colab skill "),
                    "{}: {comment} names no skill topic",
                    template.name
                );
                for pointer in comment.split("tmt colab skill ").skip(1) {
                    // `topic`, or `topic, section Name`, or `topic, sections A and B`, then a stop.
                    let pointer = pointer.split(". ").next().unwrap().trim();
                    let (topic, sections) =
                        pointer.split_once(", section").unwrap_or((pointer, ""));
                    let text = reference(topic);
                    for heading in sections.trim_start_matches('s').split(" and ") {
                        let heading = heading.trim();
                        assert!(
                            heading.is_empty()
                                || text.lines().any(|line| line == format!("## {heading}")),
                            "{}: no section `{heading}` in tmt colab skill {topic}",
                            template.name
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod registry_creation_tests {
    use super::*;
    #[test]
    fn only_the_exact_compiled_template_may_register_initial_questions() {
        let creator = "agent & \"quoted\"";
        for creator in [creator, "agent {{uuid:part}}", "agent {{style}}"] {
            assert!(tmt_colab::decoder::valid_publisher_agent(creator));
            for template in &TEMPLATES {
                let source =
                    tmt_colab::templates::creator_source(&template.source().unwrap(), creator);
                assert!(
                    tmt_colab::templates::admits(template.name, &source, Some(creator)),
                    "{}",
                    template.name
                );
                assert!(!tmt_colab::templates::admits(
                    template.name,
                    &(source.clone() + "<p>extra</p>"),
                    Some(creator)
                ));
                if source.contains("{{uuid:part}}") {
                    let substituted =
                        source.replace("{{uuid:part}}", "70000000-0000-4000-8000-000000000001");
                    assert!(!tmt_colab::templates::admits(
                        template.name,
                        &substituted,
                        Some(creator),
                    ));
                }
                let report = tmt_colab::page_check::check(&source).unwrap();
                for question in report.questions {
                    assert_eq!(question.agent, creator);
                }
            }
        }
        let review = find("review").unwrap();
        let source = tmt_colab::templates::creator_source(&review.source().unwrap(), creator);
        assert!(!tmt_colab::templates::admits(
            "review",
            &source,
            Some("another agent")
        ));
        assert!(!tmt_colab::templates::admits(
            "user-authored",
            &source,
            Some(creator)
        ));
    }
}
