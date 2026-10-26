//! Explicit local agent lock/unlock; reads and signing use public Core and Colab owners.
use crate::cli_management::{fail, resolve_page};
use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command};
use serde_json::json;
use std::{
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::threads::QuoteSelector;
use tmt_colab::{
    Result, core,
    decoder::Decoder,
    edit_lock::{Action, Holder, LockHolderState, Target, Whole},
    edit_locks::{self, Change},
    keyring::{Keyring, Layout},
    page::{self, Fault},
    store::Store,
};

const LOCK: CommandSpec = CommandSpec {
    name: "lock",
    summary: "Lock a page range for one agent edit",
    examples: &[Example {
        command: "tmt colab page lock 10000000-0000-4000-8000-000000000001 --find 'Project notes' --json",
        note: "Lock the exact passage before a multi-step edit",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Publishes a signed identity-bound advisory edit lock. Give --find TEXT (one exact source passage) or --whole. One identity has one effective lock per page; a new lock replaces its previous target. The holder's overlapping patch releases it after the content commits. Offline or ended holder presence hides a lock, unknown presence keeps it. People are never blocked. Locks do not expire, open a browser or dispatch requests.",
};
const UNLOCK: CommandSpec = CommandSpec {
    name: "unlock",
    summary: "Release a finished or stale agent edit lock",
    examples: &[Example {
        command: "tmt colab page unlock 10000000-0000-4000-8000-000000000001 --json",
        note: "Release this identity's lock",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Without an ID, releases this caller identity's effective lock. LOCK accepts a unique UUID prefix of at least eight characters. --stale permits anyone to clear matching locks whose holders are not active in a fresh public presence observation; an unavailable observation clears nothing. Without LOCK, --stale clears all eligible locks within one bounded batch. An active foreign holder cannot be unlocked. Repeated unlock is a no-op. A lost publication reply keeps the original operation and never retries or falls back.",
};
pub fn lock_command() -> Command {
    tmt_cli_style::command(&LOCK)
        .arg(crate::cli_grammar::page())
        .arg(Arg::new("find").long("find").value_name("TEXT"))
        .arg(Arg::new("whole").long("whole").action(ArgAction::SetTrue))
        .group(
            ArgGroup::new("target")
                .args(["find", "whole"])
                .required(true)
                .multiple(false),
        )
}
pub fn unlock_command() -> Command {
    tmt_cli_style::command(&UNLOCK)
        .arg(crate::cli_grammar::page())
        .arg(Arg::new("lock").index(2).value_name("LOCK"))
        .arg(Arg::new("stale").long("stale").action(ArgAction::SetTrue))
}
fn caller() -> Result<Holder> {
    let caller = core::caller_snapshot().ok_or_else(|| {
        fail(
            "COLAB_IDENTITY_REQUIRED",
            "Could not identify this agent. Run the command from an agent started through tmt.",
        )
    })?;
    Ok(Holder {
        id: caller.agent_id.ok_or_else(|| {
            fail(
                "COLAB_IDENTITY_REQUIRED",
                "A canonical agent identity is required.",
            )
        })?,
        name: caller.name,
    })
}
fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}
fn quote(source: &str, find: &str) -> Result<Target> {
    if find.is_empty() {
        return Err(fail(
            "COLAB_INPUT_INVALID",
            "The lock passage must not be empty.",
        ));
    }
    let mut matches = source.match_indices(find);
    let at = matches.next().map(|m| m.0);
    if at.is_none() || matches.next().is_some() {
        return Err(fail(
            "COLAB_INPUT_INVALID",
            "The lock passage must occur exactly once. Read the current source and choose a unique passage.",
        ));
    }
    let at = at.expect("exactly one match");
    let prefix: String = source[..at]
        .chars()
        .rev()
        .take(32)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let suffix = source[at + find.len()..].chars().take(32).collect();
    let target = Target::Quote(QuoteSelector {
        exact: find.into(),
        prefix,
        suffix,
    });
    target.validate()?;
    Ok(target)
}
fn publish(
    layout: &Layout,
    key: &Keyring,
    page: &str,
    revision: &str,
    changes: &[Change],
    decoder: &mut Decoder,
) -> Result<(Vec<Action>, Option<page::Receipt>)> {
    let store = Store::read(layout)?;
    store.require_current_schema()?;
    let prepared = edit_locks::prepare(&store, key, page, revision, changes, decoder, now()?);
    let closed = store.close();
    let prepared = prepared?;
    closed?;
    let Some((frozen, actions)) = prepared else {
        return Ok((vec![], None));
    };
    let published = crate::publish_write(layout, key, page, &frozen, decoder, now()?)?;
    Ok((
        actions,
        Some(page::publication_receipt(frozen.job(), &published.record)?),
    ))
}
pub fn run(root: &Path, mode: &str, args: &ArgMatches) -> Result<()> {
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let page = resolve_page(
        &store,
        &key,
        args.get_one::<String>("page").expect("required page"),
    )?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let view = edit_locks::read(&store, &key, &page, &mut decoder)?;
    let mut changes = Vec::new();
    if mode == "lock" {
        let holder = caller()?;
        let target = if args.get_flag("whole") {
            Target::Whole(Whole { whole: true })
        } else {
            quote(
                &page::read(&store, &key, &page, &mut decoder)?.source,
                args.get_one::<String>("find").expect("target selected"),
            )?
        };
        let writer = &view.writer;
        let own: Vec<_> = view
            .locks
            .iter()
            .filter(|lock| {
                !lock.released && lock.sender_device == *writer && lock.holder.id != holder.id
            })
            .collect();
        let presence = core::lock_presence(
            &own.iter()
                .map(|lock| lock.holder.id.clone())
                .collect::<Vec<_>>(),
        )?;
        for lock in own {
            if presence
                .get(&lock.holder.id)
                .is_some_and(|p| matches!(p, LockHolderState::Offline | LockHolderState::Ended))
            {
                changes.push(Change::Release {
                    holder_id: lock.holder.id.clone(),
                    lock_id: lock.lock_id.clone(),
                });
            }
        }
        changes.push(Change::Acquire { holder, target });
    } else {
        let stale = args.get_flag("stale");
        let caller = if stale { None } else { Some(caller()?) };
        let selector = args.get_one::<String>("lock");
        let mut locks: Vec<_> = view
            .locks
            .iter()
            .filter(|lock| selector.is_none_or(|id| lock.lock_id.starts_with(id)))
            .collect();
        if let Some(selector) = selector
            && (selector.len() < 8
                || !selector.bytes().all(|b| {
                    b.is_ascii_hexdigit() && (!b.is_ascii_alphabetic() || b.is_ascii_lowercase())
                        || b == b'-'
                })
                || locks.len() != 1)
        {
            return Err(fail(
                "COLAB_INPUT_INVALID",
                "Use a unique lock UUID prefix of at least eight characters.",
            ));
        }
        if !stale {
            locks.retain(|lock| {
                caller
                    .as_ref()
                    .is_some_and(|caller| caller.id == lock.holder.id)
            });
            if selector.is_some() && locks.is_empty() {
                return Err(fail(
                    "COLAB_LOCK_HELD",
                    "That lock belongs to another active identity. Use --stale only after its holder is no longer active.",
                ));
            }
        }
        let ids = locks
            .iter()
            .filter(|lock| !lock.released)
            .map(|lock| lock.holder.id.clone())
            .collect::<Vec<_>>();
        let presence = if stale {
            core::lock_presence(&ids)?
        } else {
            Default::default()
        };
        for lock in locks {
            if lock.released {
                continue;
            }
            if stale && presence.get(&lock.holder.id) == Some(&LockHolderState::Active) {
                if selector.is_some() {
                    return Err(fail(
                        "COLAB_LOCK_HELD",
                        "The lock holder is still active. Wait for its patch or unlock.",
                    ));
                }
                continue;
            }
            changes.push(Change::Release {
                holder_id: lock.holder.id.clone(),
                lock_id: lock.lock_id.clone(),
            });
        }
    }
    store.close()?;
    let (actions, receipt) = publish(&layout, &key, &page, &view.revision, &changes, &mut decoder)?;
    let json = args.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json);
    if json {
        writeln!(
            output,
            "{}",
            json!({"spaceId":key.space_id,"pageId":page,"changed":!actions.is_empty(),"locks":actions,"operationId":receipt.as_ref().and_then(|r|r.publication.as_ref()).map(|p|&p.operation_id),"revision":receipt.as_ref().map(|r|r.revision.as_str()).unwrap_or(&view.revision)})
        )?;
    } else {
        let terminal = output.terminal();
        let mut rows = vec![
            ("page", page),
            ("changed", (!actions.is_empty()).to_string()),
        ];
        for action in actions {
            rows.push((
                "lock",
                format!(
                    "{} · {} · {}",
                    action.lock_id,
                    action.holder.name,
                    if action.released {
                        "released"
                    } else {
                        "locked"
                    }
                ),
            ));
        }
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            if mode == "lock" {
                "PAGE LOCKED"
            } else {
                "PAGE UNLOCKED"
            },
            &rows,
        )?;
    }
    Ok(())
}

/// Capture before committing the source, so a later lock never becomes this patch's cleanup.
pub fn patch_locks(
    store: &Store,
    key: &Keyring,
    page: &str,
    old: &str,
    new: &str,
    decoder: &mut Decoder,
) -> Result<Vec<String>> {
    let view = edit_locks::read(store, key, page, decoder)?;
    if !view.locks.iter().any(|lock| !lock.released) {
        return Ok(vec![]);
    }
    let Some(holder) = core::caller_snapshot().and_then(|caller| caller.agent_id) else {
        return Ok(vec![]);
    };
    Ok(view
        .locks
        .iter()
        .filter(|lock| {
            !lock.released
                && lock.holder.id == holder
                && tmt_colab::edit_lock::overlaps(&lock.target, old, new)
        })
        .map(|lock| lock.lock_id.clone())
        .collect())
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchRelease {
    pub confirmed: bool,
    pub released: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}
/// The content is already durable. Failure only asks for unlock; it never changes
/// the source result, retries a patch or falls back from an uncertain publication.
pub fn release_patch(
    layout: &Layout,
    key: &Keyring,
    page: &str,
    ids: &[String],
    decoder: &mut Decoder,
    receipt: &mut page::Receipt,
) -> Option<PatchRelease> {
    if ids.is_empty() {
        return None;
    }
    let release = (|| -> Result<(Vec<Action>, Option<page::Receipt>)> {
        let store = Store::read(layout)?;
        let view = edit_locks::read(&store, key, page, decoder);
        let closed = store.close();
        let view = view?;
        closed?;
        let changes = view
            .locks
            .iter()
            .filter(|lock| !lock.released && ids.contains(&lock.lock_id))
            .map(|lock| Change::Release {
                holder_id: lock.holder.id.clone(),
                lock_id: lock.lock_id.clone(),
            })
            .collect::<Vec<_>>();
        publish(layout, key, page, &view.revision, &changes, decoder)
    })();
    match release {
        Ok((actions, result)) => {
            if let Some(result) = &result {
                receipt.revision = result.revision.clone();
            }
            Some(PatchRelease {
                confirmed: true,
                released: actions.into_iter().map(|action| action.lock_id).collect(),
                operation_id: result.and_then(|r| r.publication.map(|p| p.operation_id)),
                warning: None,
            })
        }
        Err(error) => Some(PatchRelease {
            confirmed: false,
            released: vec![],
            operation_id: error
                .downcast_ref::<page::OutcomeUnknown>()
                .map(|error| error.operation_id.clone()),
            warning: Some(format!(
                "The patch is saved, but its lock release was not confirmed. Read the page, then run {} if its old lock remains. Do not repeat the patch.",
                ids.iter()
                    .map(|id| format!("`tmt colab page unlock {page} {id}`"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )),
        }),
    }
}
