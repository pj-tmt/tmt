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
    command_spec!("proposal","Add, list or resolve page proposals","tmt colab proposal ls 10000000-0000-4000-8000-000000000001 --json","Proposals are checklist items an agent adds to a page for the person to approve, decline or follow up on. They are added at the end of the page. Deciding does not resolve a proposal, and these commands never notify agents.")
        .mut_arg("json",|arg|arg.global(true)).subcommand_required(true)
        .subcommand(command_spec!("add","Add a proposal at the end of a page","tmt colab proposal add 10000000-0000-4000-8000-000000000001 --title 'Review this' --body 'Please review the page.'","Saves the proposal, then adds its place at the end of the page. If the command stops partway, it prints the proposal ID: run `tmt colab proposal ls PAGE` to check, then rerun add with `--id <ID>` to finish only the missing step.")
            .arg(crate::cli_grammar::page()).arg(Arg::new("title").long("title").required(true)).arg(Arg::new("body").long("body").required(true))
            .arg(Arg::new("id").long("id").help("Finish an interrupted add with its proposal ID or unique prefix").value_parser(proposal_prefix)))
        .subcommand(command_spec!("ls","List a page's proposals","tmt colab proposal ls 10000000-0000-4000-8000-000000000001 --json","Shows each proposal's decision and whether it is resolved. Reads only; changes nothing.").alias("list").arg(crate::cli_grammar::page()))
        .subcommand(command_spec!("resolve","Mark a proposal resolved","tmt colab proposal resolve 10000000-0000-4000-8000-000000000001 20000000-0000-4000-8000-000000000001","Does not notify agents. Resolving one that is already resolved is not an error. Unknown or deleted proposals are refused.").arg(crate::cli_grammar::page()).arg(Arg::new("id").required(true).index(2).help("Proposal ID or unique lowercase UUID prefix (at least 8 characters; see proposal ls)").value_parser(proposal_prefix)))
}
fn proposal_prefix(value: &str) -> std::result::Result<String, &'static str> {
    if tmt_colab::short_links::valid_prefix(value)
        && (value.len() < 36 || tmt_colab_model::values::generated_id(value).is_ok())
    {
        Ok(value.to_owned())
    } else {
        Err("Use a full proposal UUID or a lowercase UUID prefix of at least 8 characters")
    }
}
fn resolve_proposal(view: &discussion::View, prefix: &str) -> Result<String> {
    let ids: Vec<_> = view
        .conversations
        .threads
        .iter()
        .filter_map(|row| row.proposal.as_ref().map(|p| p.proposal_id.clone()))
        .collect();
    let matches = tmt_colab::short_links::matches(prefix, &ids);
    match matches.as_slice() {
        [id] => Ok((*id).into()),
        [] => Err(format!("No proposal matches {prefix}. Run `tmt colab proposal ls PAGE` to check its ID.").into()),
        _ => Err(format!("Proposal ID {prefix} matches more than one proposal. Use a longer ID from `tmt colab proposal ls PAGE`.").into()),
    }
}
#[derive(Debug)]
pub struct RecoveryFault {
    pub correlation: Value,
    pub cause: Box<dyn std::error::Error + Send + Sync>,
    pub saved: bool,
}
impl std::fmt::Display for RecoveryFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let id = self.correlation["proposalId"].as_str().unwrap_or("");
        if self.saved {
            write!(
                f,
                "Proposal {id} was saved, but its place on the page is not confirmed. {} Run `tmt colab proposal ls PAGE`, then rerun add with `--id {id}`.",
                self.cause
            )
        } else {
            write!(
                f,
                "{} Proposal ID: {id}. Run `tmt colab proposal ls PAGE` to check, then rerun add with `--id {id}` to finish only the missing step.",
                self.cause
            )
        }
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
        let caller = core::caller_snapshot().ok_or("Could not tell which agent is running this command. Run it from an agent session started through tmt.")?;
        let machine_id = crate::door::creation_observation()
            .machine_id
            .ok_or("Remote is not running. Start it with `tmt colab serve`, then try again.")?;
        Proposal {
            proposal_id: proposal_id.into(),
            title: title.clone(),
            body: body.clone(),
            proposer: Proposer {
                machine_id,
                agent_id: caller
                    .agent_id
                    .ok_or("Could not tell which agent is running this command. Run it from an agent session started through tmt.")?,
                label: caller.name,
            },
        }
    } else {
        if matching.len() != 1 || matching[0].deleted {
            return Err(Fault::Invalid.into());
        }
        let retained = matching[0].proposal.as_ref().unwrap();
        if retained.title != *title || retained.body != *body {
            return Err(format!("Proposal {proposal_id} already exists with a different title or body. Nothing was changed.").into());
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
        return Err(format!("Could not confirm the proposal was saved. Run `tmt colab proposal ls PAGE`; if it is missing, rerun add with `--id {proposal_id}`.").into());
    }
    place(layout,key,id,decoder,proposal_id,current).map_err(|cause|RecoveryFault {
        correlation:json!({"spaceId":key.space_id,"pageId":id,"proposalId":proposal_id,"placed":false}),cause,saved:true,
    }.into())
}
fn place(
    layout: &Layout,
    key: &Keyring,
    id: &str,
    decoder: &mut Decoder,
    proposal_id: &str,
    current: page::Page,
) -> Result<bool> {
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
    let mut rows = vec![("page", id.clone())];
    let value = match mode {
        "add" => {
            let proposal_id = match args.get_one::<String>("id") {
                Some(prefix) if prefix.len() < 36 => {
                    let (view, _) = read(&layout, &key, &id, &mut decoder)?;
                    resolve_proposal(&view, prefix)?
                }
                Some(id) => id.clone(),
                None => page::fresh_id()?,
            };
            let correlation =
                json!({"spaceId":key.space_id,"pageId":id,"proposalId":proposal_id,"placed":false});
            let placed =
                add(&layout, &key, &id, args, &mut decoder, &proposal_id).map_err(|cause| {
                    if cause.is::<RecoveryFault>() {
                        cause
                    } else {
                        Box::new(RecoveryFault {
                            correlation: correlation.clone(),
                            cause,
                            saved: false,
                        }) as Box<dyn std::error::Error + Send + Sync>
                    }
                })?;
            rows.push(("proposal", proposal_id.clone()));
            rows.push(("placed", if placed { "yes" } else { "no" }.into()));
            if !placed {
                rows.push(("next",format!("Run `tmt colab proposal ls {id}` to check, then finish with `tmt colab proposal add {id} --id {proposal_id}` using the same title and body.")));
            }
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
            let ids: Vec<_> = proposals
                .iter()
                .map(|t| t.proposal.as_ref().unwrap().proposal_id.clone())
                .collect();
            for thread in &proposals {
                let proposal = thread.proposal.as_ref().unwrap();
                rows.push((
                    "proposal",
                    format!(
                        "{} · {} · {} · {}",
                        tmt_colab::short_links::shortest_id(&proposal.proposal_id, &ids),
                        proposal.title,
                        thread
                            .decision
                            .as_ref()
                            .map_or("open", |d| d.decision.as_str()),
                        if thread.deleted {
                            "deleted"
                        } else if thread.resolved {
                            "resolved"
                        } else {
                            "not resolved"
                        }
                    ),
                ));
            }
            if proposals.is_empty() {
                rows.push(("proposals", "none".into()));
            }
            json!({"spaceId":key.space_id,"pageId":id,"epoch":view.conversations.epoch,"revision":view.revision,"proposals":proposals})
        }
        "resolve" => {
            let (view, _) = read(&layout, &key, &id, &mut decoder)?;
            let proposal_id = resolve_proposal(&view, args.get_one::<String>("id").unwrap())?;
            let matches: Vec<_> = view
                .conversations
                .threads
                .iter()
                .filter(|t| {
                    t.proposal
                        .as_ref()
                        .is_some_and(|p| p.proposal_id == proposal_id)
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
            rows.push(("proposal", proposal_id.clone()));
            rows.push((
                "status",
                if changed {
                    "resolved"
                } else {
                    "already resolved"
                }
                .into(),
            ));
            json!({"spaceId":key.space_id,"pageId":id,"proposalId":proposal_id,"resolved":true,"changed":changed})
        }
        _ => unreachable!(),
    };
    if args.get_flag("json") {
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::detail::write(&mut output, terminal, "PAGE PROPOSALS", &rows)?;
    }
    Ok(())
}
