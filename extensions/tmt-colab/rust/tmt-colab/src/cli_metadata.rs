//! Root-local question and creator commands. Routing references are resolved before the existing
//! conditional content publisher runs; selecting an identity never changes writer authority.
use crate::cli_management::{fail, resolve_page};
use clap::{Arg, ArgMatches, Command};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use tmt_cli_style::Example;
use tmt_colab::{
    Result,
    core::{self, IdentityLookup},
    decoder::{Decoder, StructuredKind},
    keyring::{Keyring, Layout},
    page::{self, Fault},
    store::Store,
};

fn spec(name: &'static str, summary: &'static str, details: &'static str) -> Command {
    let examples: &'static [Example] = match name {
        "add" => &[Example {
            command: "tmt colab question add 10000000-0000-4000-8000-000000000001 --id 10000000-0000-4000-8000-000000000001 --agent NAME --json",
            note: "Register a question before writing its source",
        }],
        "handle" => &[Example {
            command: "tmt colab question handle 10000000-0000-4000-8000-000000000001 10000000-0000-4000-8000-000000000001 --json",
            note: "Mark a registered question handled",
        }],
        "reopen" => &[Example {
            command: "tmt colab question reopen 10000000-0000-4000-8000-000000000001 10000000-0000-4000-8000-000000000001 --json",
            note: "Reopen a handled question",
        }],
        "creator" => &[Example {
            command: "tmt colab page creator 10000000-0000-4000-8000-000000000001 NAME --json",
            note: "Change the creator used by new drafts",
        }],
        _ => &[Example {
            command: "tmt colab question ls 10000000-0000-4000-8000-000000000001 --json",
            note: "Inspect registered questions",
        }],
    };
    crate::cli_work::spec(name, summary, details, examples)
}

fn revision() -> Arg {
    Arg::new("expected-revision")
        .long("expected-revision")
        .value_parser(crate::revision_token)
        .help("Refuse unless this revision is still current; otherwise use the revision just read")
}
pub fn question_command() -> Command {
    let mut command = spec("question", "Register and handle a page's questions",
        "Question registration and handling are editor-owned content. Answered state comes from verified answers, independently of Ask delivery. Changes use the current verified revision and never open a browser.")
        .subcommand_required(true)
        .subcommand(spec("ls", "List registered questions", "Reads the current registry without writing.").alias("list").arg(crate::cli_grammar::page()));
    for (name, summary) in [
        ("add", "Register a question or update its routing and title"),
        ("handle", "Mark a registered question handled"),
        ("reopen", "Reopen a handled question"),
    ] {
        let mut action = spec(
            name,
            summary,
            "An invalid question, identity, task or stale revision refuses the entire change.",
        )
        .arg(crate::cli_grammar::page())
        .arg(revision());
        if name == "add" {
            action = action
                .arg(Arg::new("id").long("id").required(true).value_name("UUID"))
                .arg(
                    Arg::new("agent")
                        .long("agent")
                        .required(true)
                        .value_name("NAME|UUID"),
                )
                .arg(Arg::new("title").long("title").value_name("TITLE"))
                .arg(Arg::new("group").long("group").value_name("UUID"));
        } else {
            action = action.arg(Arg::new("id").index(2).required(true).value_name("UUID"));
            if name == "handle" {
                action = action.arg(Arg::new("task").long("task").value_name("T-N"));
            }
        }
        command = command.subcommand(action);
    }
    command
}
pub fn creator_command() -> Command {
    spec("creator", "Change the page creator used by new question drafts",
        "Selects a stable identity reference, attributed to the current editor. It changes neither the original author nor already-sent messages.")
        .arg(crate::cli_grammar::page()).arg(Arg::new("agent").index(2).required(true).value_name("NAME|UUID")).arg(revision())
}

/// A transient lookup observation; only the stable ID and observed label enter content metadata.
pub(crate) struct SelectedAgent {
    pub agent: Value,
    pub unverified: bool,
}
pub(crate) fn select_agent(
    selector: &str,
    stored: &[Value],
    retry: &str,
    uuid_fix: &str,
) -> Result<SelectedAgent> {
    select_observed(
        selector,
        stored,
        core::identity_lookup(selector),
        retry,
        uuid_fix,
    )
}
fn select_observed(
    selector: &str,
    stored: &[Value],
    lookup: IdentityLookup,
    retry: &str,
    uuid_fix: &str,
) -> Result<SelectedAgent> {
    match lookup {
        IdentityLookup::Found(caller) => Ok(SelectedAgent {
            agent: json!({"id":caller.agent_id.expect("resolved ID"),"name":caller.name}),
            unverified: false,
        }),
        IdentityLookup::NotFound => Err(fail(
            "COLAB_QUESTION_AGENT_UNKNOWN",
            &format!(
                "The identity {selector:?} was not found. Nothing was written. Select a known identity and retry: {retry}"
            ),
        )),
        IdentityLookup::Unavailable => {
            let matching: Vec<_> = stored
                .iter()
                .filter(|agent| agent["id"] == selector || agent["name"] == selector)
                .collect();
            if let Some(agent) = matching.first()
                && matching.iter().all(|other| other["id"] == agent["id"])
            {
                return Ok(SelectedAgent {
                    agent: (*agent).clone(),
                    unverified: true,
                });
            }
            if tmt_colab_model::values::core_id(selector).is_ok() {
                return Ok(SelectedAgent {
                    agent: json!({"id":selector,"name":selector}),
                    unverified: true,
                });
            }
            Err(fail(
                "COLAB_IDENTITY_UNVERIFIED",
                &format!(
                    "Core identity lookup is unavailable, so this new name cannot be bound to an ID. Nothing was written. Replace UUID with the identity's canonical UUID: {uuid_fix}. Or retry when lookup is available: {retry}"
                ),
            ))
        }
    }
}
/// Captured once before source admission or local initialization.
pub(crate) struct CreateMetadata {
    pub caller: Option<core::CallerSnapshot>,
    pub selected: SelectedAgent,
    pub template: Option<String>,
}
pub(crate) fn creation(args: &ArgMatches, template: Option<&str>) -> Result<CreateMetadata> {
    let caller = core::caller_snapshot();
    let selected = if let Some(selector) = args.get_one::<String>("creator") {
        let option = tmt_colab::repair::option;
        let title = option("title", args.get_one::<String>("title").expect("title"));
        // These commands bind the identity before a create. The original source/layout options
        // remain on the command the caller retries; no source is silently dropped.
        let prefix = format!("tmt colab page create {title}");
        let mut suffix = String::new();
        if let Some(template) = template {
            suffix.push_str(&format!(" {}", option("template", template)));
        } else {
            if let Some(layout) = args.get_one::<String>("layout") {
                suffix.push_str(&format!(" {}", option("layout", layout)));
            }
            if let Some(reason) = args.get_one::<String>("reason") {
                suffix.push_str(&format!(" {}", option("reason", reason)));
            }
            if let Some(file) = args.get_one::<std::path::PathBuf>("file") {
                suffix.push_str(&format!(" {}", option("file", &file.to_string_lossy())));
            }
        }
        select_agent(
            selector,
            &[],
            &format!("{prefix} {}{suffix}", option("creator", selector)),
            &format!("{prefix} --creator UUID{suffix}"),
        )?
    } else {
        default_creator(caller.as_ref())?
    };
    Ok(CreateMetadata {
        caller,
        selected,
        template: template.map(str::to_owned),
    })
}
pub(crate) fn default_creator(caller: Option<&core::CallerSnapshot>) -> Result<SelectedAgent> {
    caller.and_then(|caller| caller.agent_id.as_ref().map(|id| SelectedAgent {
        agent: json!({"id":id,"name":caller.name}), unverified:false,
    })).ok_or_else(|| fail("COLAB_CREATOR_REQUIRED", "This page needs a stable creator identity. Nothing was created. Retry the create command with --creator UUID, replacing UUID with a canonical identity UUID; or retry from an identified agent."))
}
pub(crate) fn admit_create_file(
    source: &str,
    args: &ArgMatches,
    metadata: &CreateMetadata,
) -> Result<()> {
    let report = tmt_colab::page_check::check(source).map_err(|_| Fault::Capacity)?;
    if report.question_sources.is_empty() {
        return Ok(());
    }
    let quote = tmt_colab::repair::argument;
    let option = tmt_colab::repair::option;
    let mut create = format!(
        "tmt colab page create {} --creator {}",
        option("title", args.get_one::<String>("title").expect("title")),
        quote(metadata.selected.agent["id"].as_str().expect("creator"))
    );
    for field in ["layout", "reason"] {
        if let Some(value) = args.get_one::<String>(field) {
            create.push_str(&format!(" {}", option(field, value)));
        }
    }
    let mut lines = vec![create + " --no-open"];
    for question in &report.question_sources {
        if !tmt_colab::page_check::valid_question_id(&question.id) {
            continue;
        }
        let agent = if question.agent.trim().is_empty() {
            metadata.selected.agent["id"].as_str().expect("creator")
        } else if tmt_colab::decoder::valid_publisher_agent(question.agent.trim()) {
            question.agent.trim()
        } else {
            "AGENT_UUID"
        };
        let mut register = format!(
            "tmt colab question add PAGE --id {} {}",
            quote(&question.id),
            option("agent", agent)
        );
        if let Some(accepted) = report
            .questions
            .iter()
            .find(|value| value.id == question.id)
        {
            register.push_str(&format!(" {}", option("title", &accepted.title)));
            if let Some(group) = &accepted.group {
                register.push_str(&format!(" --group {}", quote(group)));
            }
        }
        lines.push(register);
    }
    let file = args
        .get_one::<std::path::PathBuf>("file")
        .expect("authored file");
    lines.push(format!(
        "tmt colab page write PAGE {}",
        option("file", &file.to_string_lossy())
    ));
    Err(fail(
        "COLAB_QUESTION_UNREGISTERED",
        &format!(
            "Authored question blocks must be registered before their first source write. Nothing was created. Run the create command, replace PAGE below with its returned page ID, then register and write (for stdin, save the source in a file first):\n{}",
            lines.join("\n")
        ),
    ))
}

fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}

pub fn run(root: &Path, group: &str, args: &ArgMatches) -> Result<()> {
    let (name, args) = if group == "question" {
        args.subcommand().expect("required question command")
    } else {
        ("creator", args)
    };
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let lock = if name == "ls" {
        None
    } else {
        crate::write_owner(&layout)?
    };
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let page_id = resolve_page(&store, &key, args.get_one::<String>("page").expect("page"))?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let current = page::read(&store, &key, &page_id, &mut decoder)?;
    if name == "ls" {
        store.close()?;
        let value = json!({"pageId":page_id,"revision":current.revision,"questions":current.question_states});
        let mut output = tmt_cli_style::stream::stdout(args.get_flag("json"));
        if args.get_flag("json") {
            writeln!(output, "{value}")?;
        } else {
            let terminal = output.terminal();
            let rows: Vec<_> = value["questions"]
                .as_array()
                .expect("registry")
                .iter()
                .map(|entry| {
                    (
                        entry["id"].as_str().unwrap_or_default(),
                        format!(
                            "{} · {} · {}",
                            entry["state"].as_str().unwrap_or_default(),
                            entry["agent"]["name"].as_str().unwrap_or_default(),
                            entry["title"].as_str().unwrap_or_default()
                        ),
                    )
                })
                .collect();
            tmt_cli_style::detail::write(&mut output, terminal, "QUESTIONS", &rows)?;
        }
        return Ok(());
    }
    let mut body = json!({});
    let mut identity_verified = None;
    let (kind, action) = if name == "creator" {
        (StructuredKind::Creator, "creator.set")
    } else {
        (
            StructuredKind::Questions,
            match name {
                "add" => "question.add",
                "handle" => "question.handle",
                "reopen" => "question.reopen",
                _ => unreachable!(),
            },
        )
    };
    if name != "creator" {
        body["id"] = json!(args.get_one::<String>("id").expect("id"));
        for field in if name == "add" {
            &["title", "group"][..]
        } else if name == "handle" {
            &["task"][..]
        } else {
            &[][..]
        } {
            if let Some(value) = args.get_one::<String>(field) {
                body[*field] = json!(value);
            }
        }
    }
    if matches!(name, "add" | "creator") {
        let mut stored = Vec::new();
        if let Some(creator) = &current.creator {
            stored.push(creator["agent"].clone());
        } else if let Some(recipient) = &current.creation_recipient {
            stored.push(json!({"id":recipient.agent_id,"name":current.original_author.as_deref().unwrap_or(&recipient.agent_id)}));
        }
        if let Some(entries) = current.questions.as_ref().and_then(Value::as_array) {
            stored.extend(entries.iter().map(|entry| entry["agent"].clone()));
        }
        let selector = args.get_one::<String>("agent").expect("agent");
        let quote = tmt_colab::repair::argument;
        let prefix = if name == "creator" {
            format!("tmt colab page creator {page_id}")
        } else {
            format!(
                "tmt colab question add {page_id} --id {}",
                quote(body["id"].as_str().expect("id"))
            )
        };
        let mut suffix = String::new();
        for field in ["title", "group"] {
            if let Some(value) = body.get(field).and_then(Value::as_str) {
                suffix.push_str(&format!(" {}", tmt_colab::repair::option(field, value)));
            }
        }
        if let Some(revision) = args.get_one::<String>("expected-revision") {
            suffix.push_str(&format!(" --expected-revision {}", quote(revision)));
        }
        let (retry, uuid_fix) = if name == "creator" {
            // Positional names may begin with a hyphen; flags belong before the terminator.
            (
                format!("{prefix}{suffix} -- {}", quote(selector)),
                format!("{prefix}{suffix} -- UUID"),
            )
        } else {
            (
                format!(
                    "{prefix} {}{suffix}",
                    tmt_colab::repair::option("agent", selector)
                ),
                format!("{prefix} --agent UUID{suffix}"),
            )
        };
        let selected = select_agent(selector, &stored, &retry, &uuid_fix)?;
        body["agent"] = selected.agent;
        identity_verified = Some(!selected.unverified);
    }
    let display_agent = core::caller_snapshot().map(|caller| {
        let mut agent = json!({"name":caller.name});
        if let Some(id) = caller.agent_id {
            agent["id"] = json!(id);
        }
        agent
    });
    let expected = args
        .get_one::<String>("expected-revision")
        .map(String::as_str)
        .unwrap_or(&current.revision);
    let prepared = page::prepare_structured_publication(
        &store,
        &key,
        &page_id,
        page::StructuredAction {
            kind,
            action,
            body: &body,
            expected_revision: expected,
            display_agent,
            operation_id: None,
        },
        &mut decoder,
        &now,
    );
    let closed = store.close();
    let prepared = prepared?;
    closed?;
    let receipt = crate::publish_structured_owned(
        &layout,
        &key,
        &page_id,
        prepared,
        &mut decoder,
        now,
        lock.as_ref(),
    )?;
    let mut output = tmt_cli_style::stream::stdout(args.get_flag("json"));
    if args.get_flag("json") {
        let mut value = serde_json::to_value(receipt)?;
        if let Some(verified) = identity_verified {
            value["identityVerified"] = json!(verified);
        }
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        let mut rows = vec![("page", receipt.page_id), ("revision", receipt.revision)];
        if identity_verified == Some(false) {
            rows.push((
                "identity",
                "unverified (Core lookup unavailable)".to_owned(),
            ));
        }
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            if receipt.changed {
                "METADATA UPDATED"
            } else {
                "METADATA UNCHANGED"
            },
            &rows,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_lookup_reuses_references_accepts_only_explicit_new_ids_and_offers_both_repairs()
    {
        let id = "50000000-0000-1000-8000-000000000001";
        let stored = vec![json!({"id":id,"name":"known"})];
        for selector in ["known", id] {
            let selected = select_observed(
                selector,
                &stored,
                IdentityLookup::Unavailable,
                "retry",
                "uuid fix",
            )
            .unwrap();
            assert_eq!(selected.agent, stored[0]);
            assert!(selected.unverified);
        }
        let explicit =
            select_observed(id, &[], IdentityLookup::Unavailable, "retry", "uuid fix").unwrap();
        assert_eq!(explicit.agent["id"], id);
        assert!(explicit.unverified);
        let error = select_observed(
            "new",
            &stored,
            IdentityLookup::Unavailable,
            "plain-retry",
            "canonical-uuid-fix",
        )
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("plain-retry") && error.contains("canonical-uuid-fix"));
        assert!(
            select_observed("known", &stored, IdentityLookup::NotFound, "retry", "fix").is_err()
        );
        let ambiguous = vec![
            json!({"id":id,"name":"reused"}),
            json!({"id":"50000000-0000-1000-8000-000000000002","name":"reused"}),
        ];
        assert!(
            select_observed(
                "reused",
                &ambiguous,
                IdentityLookup::Unavailable,
                "retry",
                "uuid fix"
            )
            .is_err()
        );
    }
}
