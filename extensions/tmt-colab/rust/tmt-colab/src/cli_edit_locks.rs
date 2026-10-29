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
    details: "Publishes a signed identity-bound advisory edit lock. Give --find TEXT (one exact source passage) or --whole. One identity has one effective lock per page; a new lock replaces its previous target. The holder's successful patch on this page releases it after the content commits; a failed patch keeps this explicitly announced lock. Offline or ended holder presence hides a lock, unknown presence keeps it. A missing local-writer holder is retired; an absent foreign holder stays unknown. People are never blocked. Locks do not expire, open a browser or dispatch requests.",
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
            if matches!(
                tmt_colab::edit_lock::holder_state(lock, &view.writer, &presence),
                LockHolderState::Offline | LockHolderState::Ended
            ) {
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
            if stale
                && tmt_colab::edit_lock::holder_state(lock, &view.writer, &presence)
                    == LockHolderState::Active
            {
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

pub struct PatchAcquisition {
    pub revision: String,
    pub lock_id: String,
}

/// Announce an agent's edit before evaluating it. A caller-supplied revision is checked
/// before this own publication; the returned revision fences its confirmed acquisition.
pub fn acquire_patch(
    layout: &Layout,
    key: &Keyring,
    page: &str,
    patch: &page::patch::Patch,
    expected: Option<&str>,
    decoder: &mut Decoder,
) -> Result<Option<PatchAcquisition>> {
    if !is_agent() {
        return Ok(None);
    }
    let Some(snapshot) = core::caller_snapshot() else {
        return Ok(None);
    };
    let Some(id) = snapshot.agent_id else {
        return Ok(None);
    };
    let holder = Holder {
        id,
        name: snapshot.name,
    };
    let store = Store::read(layout)?;
    let view = edit_locks::read(&store, key, page, decoder);
    let closed = store.close();
    let view = view?;
    closed?;
    if expected.is_some_and(|expected| expected != view.revision) {
        return Err(Fault::StaleBase.into());
    }
    if view
        .locks
        .iter()
        .any(|lock| !lock.released && lock.holder.id == holder.id)
    {
        return Ok(None);
    }
    // A single named find is already the quote. Several pairs, line ranges, diffs,
    // and quotes over the existing byte bound use the conservative whole-page target.
    let target = match patch {
        page::patch::Patch::Replace(pairs)
            if pairs.len() == 1
                && !pairs[0].0.is_empty()
                && pairs[0].0.len() <= tmt_colab::limits::COMMENT_BODY_BYTES =>
        {
            Target::Quote(QuoteSelector {
                exact: pairs[0].0.clone(),
                prefix: String::new(),
                suffix: String::new(),
            })
        }
        _ => Target::Whole(Whole { whole: true }),
    };
    let (actions, receipt) = publish(
        layout,
        key,
        page,
        &view.revision,
        &[Change::Acquire { holder, target }],
        decoder,
    )?;
    Ok(receipt.map(|receipt| PatchAcquisition {
        revision: receipt.revision,
        lock_id: actions[0].lock_id.clone(),
    }))
}

fn is_agent() -> bool {
    std::env::var_os("TMT_AGENT").is_some_and(|value| !value.is_empty())
}

/// Capture before committing the source, so a later lock never becomes this patch's cleanup.
/// Agent patches fence the checked own state as well as source; retries recheck both.
/// The locks come from the same read as the patched source, so one snapshot answers both.
pub struct PatchLocks {
    pub revision: Option<String>,
    pub release: Vec<String>,
}
pub fn patch_locks(base: &page::Page, new: &str) -> Result<PatchLocks> {
    let (page, old) = (base.page_id.as_str(), base.source.as_str());
    let agent = is_agent();
    let mut captured = PatchLocks {
        revision: agent.then(|| base.revision.clone()),
        release: vec![],
    };
    if base.edit_locks.is_empty() {
        return Ok(captured);
    }
    let holder = core::caller_snapshot().and_then(|caller| caller.agent_id);
    let overlaps = base
        .edit_locks
        .iter()
        .filter(|lock| tmt_colab::edit_lock::overlaps(&lock.target, old, new))
        .collect::<Vec<_>>();
    if agent {
        let foreign = overlaps
            .iter()
            .copied()
            .filter(|lock| holder.as_deref() != Some(lock.holder.id.as_str()))
            .collect::<Vec<_>>();
        let ids = foreign
            .iter()
            .map(|lock| lock.holder.id.clone())
            .collect::<Vec<_>>();
        let presence = core::lock_presence(&ids).ok();
        for lock in foreign {
            let keeps = presence.as_ref().is_none_or(|observed| {
                tmt_colab::edit_lock::holder_state(lock, &base.local_writer, observed).keeps_lock()
            });
            if keeps {
                let target = match &lock.target {
                    Target::Whole(_) => "the whole page".to_owned(),
                    Target::Quote(quote) => format!("the quote {:?}", quote.exact),
                };
                return Err(fail(
                    "COLAB_REGION_LOCKED",
                    &format!(
                        "{} is editing {target}. Wait for it to save, patch a different part, or run tmt colab page unlock {page} {} --stale if the holder is gone; then retry the patch.",
                        lock.holder.name, lock.lock_id
                    ),
                ));
            }
        }
    }
    captured.release = base
        .edit_locks
        .iter()
        .filter(|lock| holder.as_deref() == Some(lock.holder.id.as_str()))
        .filter(|lock| agent || tmt_colab::edit_lock::overlaps(&lock.target, old, new))
        .map(|lock| lock.lock_id.clone())
        .collect();
    Ok(captured)
}
#[derive(Debug, serde::Serialize)]
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
    let release = release_ids(layout, key, page, ids, decoder);
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

/// Releases only captured IDs. Both successful edits and failed command-owned edits
/// use this owner; replacement locks and uncertain publications are never replayed.
fn release_ids(
    layout: &Layout,
    key: &Keyring,
    page: &str,
    ids: &[String],
    decoder: &mut Decoder,
) -> Result<(Vec<Action>, Option<page::Receipt>)> {
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
}

#[derive(Debug)]
pub struct PatchFailure {
    pub cause: Box<dyn std::error::Error + Send + Sync>,
    pub release: PatchRelease,
}
impl std::fmt::Display for PatchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}",
            self.cause,
            self.release.warning.as_deref().unwrap_or_default()
        )
    }
}
impl std::error::Error for PatchFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.cause.as_ref())
    }
}

/// Failure unwinds the command's automatic acquisitions only. Keep the original
/// error and its structured details, adding a recovery warning if cleanup is uncertain.
pub fn release_failed_patch(
    layout: &Layout,
    key: &Keyring,
    page: &str,
    ids: &[String],
    decoder: &mut Decoder,
    cause: Box<dyn std::error::Error + Send + Sync>,
) -> Box<dyn std::error::Error + Send + Sync> {
    if ids.is_empty() {
        return cause;
    }
    match release_ids(layout, key, page, ids, decoder) {
        Ok(_) => cause,
        Err(error) => Box::new(PatchFailure {
            cause,
            release: PatchRelease {
                confirmed: false,
                released: vec![],
                operation_id: error
                    .downcast_ref::<page::OutcomeUnknown>()
                    .map(|error| error.operation_id.clone()),
                warning: Some(format!(
                    "The automatic lock release was not confirmed. Read the page, then run {} if its old lock remains. Do not repeat the patch to recover its lock release.",
                    ids.iter()
                        .map(|id| format!("`tmt colab page unlock {page} {id}`"))
                        .collect::<Vec<_>>()
                        .join("; ")
                )),
            },
        }),
    }
}
