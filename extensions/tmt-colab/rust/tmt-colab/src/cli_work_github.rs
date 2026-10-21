//! `tmt colab task github push|sync` (#4053): the GitHub driver for page tasks. `push` publishes
//! one task to a GitHub issue of the space's repository and records `#N` as its external ID; it is
//! never part of `sync`. `sync` marks a linked task `done` once its issue was closed by a merged
//! pull request, asking GitHub about every open linked task with one GraphQL call per 100 issues.
//! Both run `gh` on the owner's machine (`work_github.rs`). A failure writes nothing to the page.
use crate::{
    cli_management::{ManagementFault, active_page_ids, fail, resolve_page},
    cli_work::{read_page_work, spec, task_arg, write_work},
};
use clap::{ArgMatches, Command};
use serde_json::{Value, json};
use std::{io::Write, path::Path};
use tmt_colab::{
    Result,
    decoder::Decoder,
    keyring::{Keyring, Layout},
    page::{self, Fault},
    settings,
    store::Store,
    work_github::{self, Answer, Gh, Process, Repo, Standing},
    work_index,
};

macro_rules! example {
    ($command:literal, $note:literal) => {
        &[tmt_cli_style::Example {
            command: $command,
            note: $note,
        }]
    };
}

pub fn command() -> Command {
    spec(
        "github",
        "Push a task to a GitHub issue, or sync linked tasks from GitHub",
        "Runs the GitHub CLI (gh) on this machine with your own sign-in; the repository is the space's github-repo setting (tmt colab settings github-repo OWNER/REPO). Nothing edited on GitHub is read back, and a failure writes nothing to the page.",
        example!(
            "tmt colab task github sync --json",
            "Mark linked tasks done once a merged pull request closed their issue"
        ),
    )
    .subcommand_required(true)
    .subcommand(
        spec(
            "push",
            "Publish one task to a GitHub issue and link it",
            "Creates an issue in the space's repository from the task and records its number (#N) as the task's external ID, printing the repository and the issue's URL. This publishes the task's title, detail and acceptance to GitHub, possibly to a public repository, so run it for one task you mean to share. #N and GH-N references in the text are broken so a page cannot link or close other issues, and the detail is fenced so no mention or link in it acts. An issue this task already pushed (found by a hidden marker in its body) is adopted instead of created again, which makes a repeat after a failed record safe. A task that already has an external ID is refused.",
            example!(
                "tmt colab task github push 10000000-0000-4000-8000-000000000001 T-3 --json",
                "Publish task T-3 as an issue and link it"
            ),
        )
        .arg(crate::cli_grammar::page())
        .arg(task_arg()),
    )
    .subcommand(
        spec(
            "sync",
            "Mark linked tasks done when a merged pull request closed their issue",
            "For every task with an external ID of the form #N that is not done (on one page, or on every page of the space), asks GitHub in one GraphQL call per 100 issues, at most 200 issues a run; the rest are reported unchecked, so run it again. A task becomes done only when its issue is closed and a merged pull request closed it (GitHub's own Fixes #N). An issue closed by hand, or still open, leaves the task alone. Each row says done, open, closed (without a merged pull request), missing, unchecked or failed; any failed row makes the command fail, and a failed check never moves a task.",
            example!(
                "tmt colab task github sync 10000000-0000-4000-8000-000000000001 --json",
                "Check one page's linked tasks"
            ),
        )
        .arg(crate::cli_grammar::page_optional(
            "Only this page; without it every page of the space",
        )),
    )
}

fn invalid(message: &str) -> Box<dyn std::error::Error + Send + Sync> {
    fail("COLAB_INPUT_INVALID", message)
}
fn gh_fault(fault: work_github::Fault) -> Box<dyn std::error::Error + Send + Sync> {
    fail(fault.code(), &fault.message)
}
fn repo(root: &Path) -> Result<Repo> {
    settings::read_or_default(root).github_repo().cloned().ok_or_else(|| {
        fail(
            work_github::REPO_UNSET,
            "No GitHub repository is set for this space; run: tmt colab settings github-repo OWNER/REPO",
        )
    })
}

pub fn run(root: &Path, args: &ArgMatches) -> Result<()> {
    let (name, args) = args.subcommand().expect("required github command");
    let gh = Process::from_env();
    if name == "push" {
        push(root, &gh, args)
    } else {
        sync(root, &gh, args)
    }
}

fn push(root: &Path, gh: &dyn Gh, args: &ArgMatches) -> Result<()> {
    let repo = repo(root)?;
    let slug = repo.slug();
    let page = args.get_one::<String>("page").expect("required page");
    let task_id = args.get_one::<String>("task").expect("required task");
    let (page_id, read) = read_page_work(root, page)?;
    let task = read.work["tasks"].get(task_id.as_str()).ok_or_else(|| {
        invalid(&format!(
            "This page has no task {task_id}; tmt colab task ls shows its tasks."
        ))
    })?;
    // A task that follows a page takes its state from that page; an issue would be a second owner.
    if task.get("child").is_some() {
        return Err(invalid(&format!(
            "{task_id} follows a page, so it is not pushed to GitHub; stop it following the page first (tmt colab task unlink)."
        )));
    }
    if let Some(linked) = task["externalId"].as_str().filter(|id| !id.is_empty()) {
        return Err(invalid(&format!(
            "{task_id} is already linked to {linked}; it is not pushed again."
        )));
    }
    // An issue this task already pushed is adopted, so a record that failed after the issue was
    // created never makes a second one.
    let marker = work_github::marker(&page_id, task_id);
    let search = format!("{marker} in:body");
    let listed = gh
        .run(&[
            "issue",
            "list",
            "--repo",
            &slug,
            "--search",
            &search,
            "--state",
            "all",
            "--json",
            "number,url,body",
            "--limit",
            "10",
        ])
        .map_err(gh_fault)?;
    if !listed.success {
        return Err(gh_fault(work_github::refusal(&listed)));
    }
    let existing = serde_json::from_str::<Value>(&listed.stdout)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|issue| {
            issue["body"]
                .as_str()
                .is_some_and(|body| body.contains(&marker))
        })
        .filter_map(|issue| Some((issue["number"].as_u64()?, issue["url"].as_str()?.to_owned())))
        .min_by_key(|(number, _)| *number);
    let (number, url, created) = match existing {
        Some((number, url)) => (number, url, false),
        None => {
            let made = gh
                .run(&[
                    "issue",
                    "create",
                    "--repo",
                    &slug,
                    "--title",
                    &work_github::issue_title(task),
                    "--body",
                    &work_github::issue_body(&page_id, task_id, task),
                ])
                .map_err(gh_fault)?;
            if !made.success {
                return Err(gh_fault(work_github::refusal(&made)));
            }
            let url = made.stdout.trim().to_owned();
            let number = work_github::created_number(&repo, &url).ok_or_else(|| {
                fail(
                    work_github::REFUSED,
                    "gh answered without the new issue's URL; check the repository for the issue.",
                )
            })?;
            (number, url, true)
        }
    };
    // Record the link: the task's whole editable part again, with its external ID.
    let mut body =
        json!({"taskId":task_id,"title":task["title"],"externalId":format!("#{number}")});
    for key in ["detail", "acceptance", "needsSignoff"] {
        if let Some(value) = task.get(key) {
            body[key] = value.clone();
        }
    }
    let written =
        write_work(root, &page_id, "task.set", &body, Some(&read.revision)).inspect_err(|_| {
            let mut err = tmt_cli_style::stream::stderr();
            let terminal = err.terminal();
            let _ = tmt_cli_style::message::warning(
                &mut err,
                terminal,
                &format!("The issue {url} exists, but its number was not recorded on {task_id}."),
                Some("Run the same push again: it adopts that issue instead of creating another"),
            );
        })?;
    let receipt = written.receipt;
    let json_output = args.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        let mut value = serde_json::to_value(&receipt)?;
        value["taskId"] = json!(task_id);
        value["repo"] = json!(slug);
        value["issue"] = json!({"number": number, "url": url});
        value["created"] = json!(created);
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            if created {
                "TASK PUSHED"
            } else {
                "TASK LINKED"
            },
            &[
                ("page", receipt.page_id.clone()),
                ("task", task_id.clone()),
                ("repo", slug),
                ("issue", url),
                (
                    "result",
                    if created { "created" } else { "adopted" }.to_owned(),
                ),
                ("revision", receipt.revision.clone()),
            ],
        )?;
    }
    Ok(())
}

/// A linked task that is not done, in the order sync asks about it.
struct Candidate {
    page_id: String,
    task: String,
    number: u64,
}

fn candidates(root: &Path, page: Option<&String>) -> Result<Vec<Candidate>> {
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let pages = match page {
        Some(operand) => vec![resolve_page(&store, &key, operand)?],
        None => active_page_ids(&store, &key)?,
    };
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let found = work_index::scan(&layout, &store, &key, &mut decoder, &pages);
    let mut out = Vec::new();
    let result = found.and_then(|found| {
        for page_id in &pages {
            // Only a page with a task that is not done can have anything to mark.
            let open = found
                .get(page_id)
                .and_then(Option::as_ref)
                .is_some_and(|work| {
                    work.derived["done"].as_u64().unwrap_or(0)
                        < work.derived["total"].as_u64().unwrap_or(0)
                });
            if !open {
                continue;
            }
            let read = page::read_work(&store, &key, page_id, &mut decoder)?;
            for id in read.work["order"].as_array().into_iter().flatten() {
                let id = id.as_str().unwrap_or_default();
                let task = &read.work["tasks"][id];
                if let Some(number) = task["externalId"]
                    .as_str()
                    .and_then(work_github::issue_number)
                    .filter(|_| task["state"] != "done")
                {
                    out.push(Candidate {
                        page_id: page_id.clone(),
                        task: id.to_owned(),
                        number,
                    });
                }
            }
        }
        Ok(())
    });
    let closed = store.close();
    result?;
    closed?;
    Ok(out)
}

/// One graphql call for up to `BATCH` issues: their standings, or the fault that kept it.
fn ask(
    gh: &dyn Gh,
    repo: &Repo,
    numbers: &[u64],
    calls: &mut usize,
) -> std::result::Result<std::collections::BTreeMap<u64, Standing>, work_github::Fault> {
    *calls += 1;
    let query = format!("query={}", work_github::query(repo, numbers));
    let answer: Answer = gh.run(&["api", "graphql", "-f", &query])?;
    work_github::standings(&answer, numbers)
}

fn row(candidate: &Candidate, outcome: &str, error: Option<(&str, &str)>) -> Value {
    let mut row = json!({
        "page": candidate.page_id,
        "task": candidate.task,
        "externalId": format!("#{}", candidate.number),
        "outcome": outcome,
    });
    if let Some((code, message)) = error {
        row["error"] = json!({"code": code, "message": message});
    }
    row
}

fn sync(root: &Path, gh: &dyn Gh, args: &ArgMatches) -> Result<()> {
    let repo = repo(root)?;
    let linked = candidates(root, args.get_one::<String>("page"))?;
    // One question per distinct issue, the first `RUN_CAP` in task order; the rest wait for a rerun.
    let mut numbers: Vec<u64> = Vec::new();
    for candidate in &linked {
        if !numbers.contains(&candidate.number) {
            numbers.push(candidate.number);
        }
    }
    let (batches, _) = work_github::plan(&numbers);
    let checked: usize = batches.iter().map(|batch| batch.len()).sum();
    let mut calls = 0;
    let mut standing = std::collections::BTreeMap::new();
    let mut faults: std::collections::BTreeMap<u64, work_github::Fault> =
        std::collections::BTreeMap::new();
    for batch in batches {
        match ask(gh, &repo, batch, &mut calls) {
            Ok(found) => standing.extend(found),
            // Not signed in is the same for every call: say so once, spend nothing more.
            Err(fault) if fault.code() == work_github::UNAVAILABLE => return Err(gh_fault(fault)),
            Err(fault) => {
                for number in batch {
                    faults.insert(
                        *number,
                        work_github::Fault {
                            code: fault.code,
                            message: fault.message.clone(),
                        },
                    );
                }
            }
        }
    }
    let mut rows = Vec::new();
    let mut failed = 0;
    for candidate in &linked {
        if let Some(fault) = faults.get(&candidate.number) {
            failed += 1;
            rows.push(row(candidate, "failed", Some((fault.code, &fault.message))));
            continue;
        }
        let Some(found) = standing.get(&candidate.number) else {
            rows.push(row(candidate, "unchecked", None));
            continue;
        };
        rows.push(match found {
            Standing::Open => row(candidate, "open", None),
            Standing::ClosedOther => row(candidate, "closed", None),
            Standing::Missing => row(candidate, "missing", None),
            Standing::ClosedByMerge => {
                let body = json!({"taskId":candidate.task,"state":"done"});
                // A page written since this pass read it is read again once.
                let mut written = write_work(root, &candidate.page_id, "task.state", &body, None);
                if written
                    .as_ref()
                    .err()
                    .is_some_and(|error| crate::error_code(error.as_ref()) == "COLAB_STALE_BASE")
                {
                    written = write_work(root, &candidate.page_id, "task.state", &body, None);
                }
                match written {
                    Ok(_) => row(candidate, "done", None),
                    Err(error) => {
                        failed += 1;
                        let code = crate::error_code(error.as_ref());
                        row(candidate, "failed", Some((code, &error.to_string())))
                    }
                }
            }
        });
    }
    let report = json!({
        "repo": repo.slug(),
        "calls": calls,
        "checked": checked,
        "rows": rows,
    });
    let json_output = args.get_flag("json");
    if failed > 0 {
        let message = format!(
            "{failed} of {} linked tasks could not be checked or marked; see their rows.",
            rows.len()
        );
        if !json_output {
            show(&report, false)?;
        }
        return Err(Box::new(ManagementFault {
            code: work_github::REFUSED,
            message,
            correlation: report,
            source: None,
        }));
    }
    show(&report, json_output)
}

fn show(report: &Value, json_output: bool) -> Result<()> {
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(output, "{report}")?;
        return Ok(());
    }
    let terminal = output.terminal();
    let mut rows = vec![
        (
            "repo",
            report["repo"].as_str().unwrap_or_default().to_owned(),
        ),
        ("calls", report["calls"].to_string()),
    ];
    for row in report["rows"].as_array().into_iter().flatten() {
        let mut said = format!(
            "{}  {}  {}",
            row["task"].as_str().unwrap_or_default(),
            row["externalId"].as_str().unwrap_or_default(),
            row["outcome"].as_str().unwrap_or_default()
        );
        if let Some(message) = row["error"]["message"].as_str() {
            said.push_str(&format!("  {message}"));
        }
        rows.push(("task", said));
    }
    tmt_cli_style::detail::write(&mut output, terminal, "GITHUB SYNC", &rows)?;
    Ok(())
}
