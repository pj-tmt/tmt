//! Record-first proposal publication and explicit ID recovery. No agent dispatch.
use clap::{Arg, ArgMatches, Command};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result, core,
    decoder::{ContentEdit, Decoder},
    discussion,
    keyring::{Keyring, Layout},
    page::{self, Fault},
    store::Store,
    threads::{Proposal, Proposer},
};

macro_rules! command_spec {
    ($name:literal,$summary:literal,$example:literal,$details:literal) => {
        tmt_cli_style::command(&CommandSpec {
            name: $name,
            summary: $summary,
            examples: &[Example {
                command: $example,
                note: $summary,
            }],
            outputs: OutputModes::HumanAndJson,
            details: $details,
        })
    };
}
pub fn command() -> Command {
    command_spec!("proposal","Add, list or resolve page proposals","tmt colab proposal ls 10000000-0000-4000-8000-000000000001 --json","Authenticated discussion records; labels are display text. Append placement only. Decisions and resolution are independent. Agent commands never dispatch notifications.")
        .mut_arg("json",|arg|arg.global(true)).subcommand_required(true)
        .subcommand(command_spec!("add","Publish a proposal and append its placeholder","tmt colab proposal add 10000000-0000-4000-8000-000000000001 --title 'Review this' --body 'Please review the page.'","Publishes the record first, then appends through the page revision fence. --id retains a UUID for recovery: read ls and the page before repeating; retry only the missing step. A partial or uncertain result includes proposalId and placed:false. New records require canonical caller and running Remote machine provenance. --after is not supported in this slice.")
            .arg(crate::cli_grammar::page()).arg(Arg::new("title").long("title").required(true)).arg(Arg::new("body").long("body").required(true))
            .arg(Arg::new("id").long("id").help("Retained proposal UUID for explicit recovery").value_parser(|v:&str|tmt_colab_model::values::generated_id(v).map(|_|v.to_owned()).map_err(|e|e.to_string()))))
        .subcommand(command_spec!("ls","Read authenticated proposals","tmt colab proposal ls 10000000-0000-4000-8000-000000000001 --json","Includes retained decision and independent resolution. Does not inspect HTML for authority or publish anything.").alias("list").arg(crate::cli_grammar::page()))
        .subcommand(command_spec!("resolve","Resolve a live proposal without notifying agents","tmt colab proposal resolve 10000000-0000-4000-8000-000000000001 20000000-0000-4000-8000-000000000001","Uses existing owner-device admission. Already resolved is a no-op; absent, ambiguous or deleted proposals are refused.").arg(crate::cli_grammar::page()).arg(Arg::new("id").required(true).index(2).value_parser(|v:&str|tmt_colab_model::values::generated_id(v).map(|_|v.to_owned()).map_err(|e|e.to_string()))))
}
#[derive(Debug)]
pub struct RecoveryFault {
    pub correlation: Value,
    pub cause: Box<dyn std::error::Error + Send + Sync>,
}
impl std::fmt::Display for RecoveryFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Proposal {}: {}. Retain this ID; read the proposal and page before explicit recovery with --id. Placement is not confirmed.",
            self.correlation["proposalId"].as_str().unwrap_or(""),
            self.cause
        )
    }
}
impl std::error::Error for RecoveryFault {}
fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}
fn read(
    layout: &Layout,
    key: &Keyring,
    id: &str,
    decoder: &mut Decoder,
) -> Result<(discussion::View, page::Page)> {
    let store = Store::read(layout)?;
    store.require_current_schema()?;
    let view = discussion::read(&store, key, id, decoder)?;
    let source = page::read(&store, key, id, decoder)?;
    store.close()?;
    Ok((view, source))
}
fn add(
    layout: &Layout,
    key: &Keyring,
    id: &str,
    args: &ArgMatches,
    decoder: &mut Decoder,
    proposal_id: &str,
) -> Result<bool> {
    let (view, _) = read(layout, key, id, decoder)?;
    let matching: Vec<_> = view
        .conversations
        .threads
        .iter()
        .filter(|t| {
            t.proposal
                .as_ref()
                .is_some_and(|p| p.proposal_id == proposal_id)
        })
        .collect();
    let title = args.get_one::<String>("title").unwrap();
    let body = args.get_one::<String>("body").unwrap();
    let proposal = if matching.is_empty() {
        let caller = core::caller_snapshot().ok_or("Canonical caller identity is unavailable")?;
        let machine_id = crate::door::creation_observation()
            .machine_id
            .ok_or("Canonical Remote machine provenance is unavailable; start the Remote door")?;
        Proposal {
            proposal_id: proposal_id.into(),
            title: title.clone(),
            body: body.clone(),
            proposer: Proposer {
                machine_id,
                agent_id: caller
                    .agent_id
                    .ok_or("Canonical caller agent UUID is unavailable")?,
                label: caller.name,
            },
        }
    } else {
        if matching.len() != 1 || matching[0].deleted {
            return Err(Fault::Invalid.into());
        }
        let retained = matching[0].proposal.as_ref().unwrap();
        if retained.title != *title || retained.body != *body {
            return Err("Retained proposal metadata differs; refusing recovery".into());
        }
        retained.clone()
    };
    let store = Store::read(layout)?;
    let prepared = discussion::prepare_proposal(&store, key, id, &proposal, decoder, now()?);
    store.close()?;
    if let Some(frozen) = prepared? {
        let published = crate::publish_write(layout, key, id, &frozen, decoder, now()?)?;
        page::publication_receipt(frozen.job(), &published.record)?;
    }
    // Re-read after the record publication: never place an unconfirmed record.
    let (view, current) = read(layout, key, id, decoder)?;
    if !view
        .conversations
        .threads
        .iter()
        .any(|t| !t.deleted && t.proposal.as_ref() == Some(&proposal))
    {
        return Err("Proposal record readback did not confirm publication".into());
    }
    let placeholder = format!("<tmt-proposal data-id=\"{proposal_id}\"></tmt-proposal>");
    match current.source.matches(&placeholder).count() {
        1 => return Ok(true),
        0 => {}
        _ => return Ok(false), // Duplicates are detached; recovery never guesses which to move.
    }
    let source = format!("{}\n{}\n", current.source, placeholder);
    let store = Store::read(layout)?;
    let publisher = core::publisher_agent();
    let preparation = page::prepare_publication_with_clock(
        &store,
        key,
        id,
        ContentEdit {
            source: &source,
            publisher_agent: publisher.as_deref(),
            attachments: None,
        },
        Some(current.revision.as_str()),
        decoder,
        &now,
    );
    store.close()?;
    if let page::PublicationPreparation::Write(frozen) = preparation? {
        let published = crate::publish_write(layout, key, id, &frozen, decoder, now()?)?;
        page::publication_receipt(frozen.job(), &published.record)?;
    }
    let (_, confirmed) = read(layout, key, id, decoder)?;
    Ok(confirmed.source.matches(&placeholder).count() == 1)
}
pub fn run(root: &Path, args: &ArgMatches) -> Result<()> {
    let (mode, args) = args.subcommand().expect("required subcommand");
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let id =
        crate::cli_management::resolve_page(&store, &key, args.get_one::<String>("page").unwrap())?;
    store.close()?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let mut output = tmt_cli_style::stream::stdout(args.get_flag("json"));
    let value = match mode {
        "add" => {
            let proposal_id = args
                .get_one::<String>("id")
                .cloned()
                .map(Ok)
                .unwrap_or_else(page::fresh_id)?;
            let correlation =
                json!({"spaceId":key.space_id,"pageId":id,"proposalId":proposal_id,"placed":false});
            let placed =
                add(&layout, &key, &id, args, &mut decoder, &proposal_id).map_err(|cause| {
                    RecoveryFault {
                        correlation: correlation.clone(),
                        cause,
                    }
                })?;
            json!({"spaceId":key.space_id,"pageId":id,"proposalId":proposal_id,"placed":placed})
        }
        "ls" => {
            let (view, _) = read(&layout, &key, &id, &mut decoder)?;
            let proposals: Vec<_> = view
                .conversations
                .threads
                .into_iter()
                .filter(|t| t.proposal.is_some())
                .collect();
            json!({"spaceId":key.space_id,"pageId":id,"epoch":view.conversations.epoch,"revision":view.revision,"proposals":proposals})
        }
        "resolve" => {
            let proposal_id = args.get_one::<String>("id").unwrap();
            let (view, _) = read(&layout, &key, &id, &mut decoder)?;
            let matches: Vec<_> = view
                .conversations
                .threads
                .iter()
                .filter(|t| {
                    t.proposal
                        .as_ref()
                        .is_some_and(|p| p.proposal_id == *proposal_id)
                })
                .collect();
            if matches.len() != 1 || matches[0].deleted {
                return Err(Fault::Invalid.into());
            }
            let caller = core::publisher_agent();
            let store = Store::read(&layout)?;
            let prepared = discussion::prepare_status(
                &store,
                &key,
                &id,
                discussion::StatusEdit {
                    thread: &matches[0].id,
                    resolved: true,
                    agent_name: caller.as_deref(),
                },
                &mut decoder,
                now()?,
            );
            store.close()?;
            let prepared = prepared?;
            let changed = prepared.is_some();
            if let Some((frozen, _)) = prepared {
                let published =
                    crate::publish_write(&layout, &key, &id, &frozen, &mut decoder, now()?)?;
                page::publication_receipt(frozen.job(), &published.record)?;
            }
            json!({"spaceId":key.space_id,"pageId":id,"proposalId":proposal_id,"resolved":true,"changed":changed})
        }
        _ => unreachable!(),
    };
    if args.get_flag("json") {
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            "PAGE PROPOSALS",
            &[
                ("page", id),
                ("result", serde_json::to_string_pretty(&value)?),
            ],
        )?;
    }
    Ok(())
}
