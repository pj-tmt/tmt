//! The GitHub driver for page tasks (#4053): a task is pushed to a GitHub issue on request, and a
//! sync pass marks a linked task `done` once GitHub's own `Fixes #N` closed its issue through a
//! merged pull request. It runs `gh` on the owner's machine with the owner's authentication, argv
//! only, and the program is `gh` on PATH or the file named by `TMT_COLAB_GH`, so a test puts a
//! scripted fake there and nothing in a test reaches GitHub. Edits made on GitHub are never read
//! back. This module holds what needs no page or store: the repo, the `gh` runner, the issue body,
//! the batched query and its answer.
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// The environment variable naming the `gh` program; `gh` on PATH when unset.
pub const PROGRAM_ENV: &str = "TMT_COLAB_GH";
/// The issues one GraphQL call asks about.
pub const BATCH: usize = 100;
/// The issues one sync run asks about in all (two calls); the rest are reported as unchecked.
pub const RUN_CAP: usize = 2 * BATCH;
/// The largest issue number a task's external ID may name.
const MAX_NUMBER: u64 = 999_999_999;
const TIMEOUT: Duration = Duration::from_secs(60);
const OUTPUT_LIMIT: u64 = 1 << 20;
const MESSAGE_LIMIT: usize = 400;

pub const UNAVAILABLE: &str = "COLAB_GITHUB_UNAVAILABLE";
pub const REFUSED: &str = "COLAB_GITHUB_REFUSED";
pub const REPO_UNSET: &str = "COLAB_GITHUB_REPO_UNSET";

/// A driver failure with its stable code. Nothing is written to a page when one occurs.
#[derive(Debug, PartialEq, Eq)]
pub struct Fault {
    pub code: &'static str,
    pub message: String,
}
impl Fault {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Fault {}

/// `OWNER/REPO`, the one repository the space's tasks are pushed to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    owner: String,
    name: String,
}
impl Repo {
    pub fn parse(text: &str) -> Option<Self> {
        let (owner, name) = text.split_once('/')?;
        // Letters, digits, `-`, `_` and `.`, never leading with `-` (an option) or `.`; so the slug
        // is safe as one argument and inside a quoted GraphQL string.
        let part = |part: &str| {
            (1..=100).contains(&part.len())
                && !part.starts_with(['-', '.'])
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        };
        (part(owner) && part(name)).then(|| Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// What a `gh` run answered. A non-zero exit is data here: a GraphQL answer with an error for one
/// issue still carries the others.
#[derive(Debug)]
pub struct Answer {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// The one door to GitHub; a test supplies the fake.
pub trait Gh {
    fn run(&self, args: &[&str]) -> Result<Answer, Fault>;
}

/// The owner's `gh`, run as a child process.
pub struct Process {
    program: PathBuf,
}
impl Process {
    pub fn from_env() -> Self {
        Self {
            program: std::env::var_os(PROGRAM_ENV)
                .filter(|program| !program.is_empty())
                .map_or_else(|| PathBuf::from("gh"), PathBuf::from),
        }
    }
}
fn bounded(mut stream: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = (&mut stream).take(OUTPUT_LIMIT).read_to_end(&mut bytes);
        // Whatever is left is drained, so a chatty child never blocks on a full pipe.
        let _ = std::io::copy(&mut stream, &mut std::io::sink());
        bytes
    })
}
impl Gh for Process {
    fn run(&self, args: &[&str]) -> Result<Answer, Fault> {
        let mut child = Command::new(&self.program)
            .args(args)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                Fault::new(
                    UNAVAILABLE,
                    format!(
                        "gh could not be run ({error}); install the GitHub CLI and sign in with gh auth login."
                    ),
                )
            })?;
        let stdout = bounded(child.stdout.take().expect("piped stdout"));
        let stderr = bounded(child.stderr.take().expect("piped stderr"));
        let deadline = Instant::now() + TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Fault::new(
                        REFUSED,
                        format!("gh did not answer within {} seconds.", TIMEOUT.as_secs()),
                    ));
                }
                Err(error) => return Err(Fault::new(REFUSED, format!("gh failed: {error}"))),
            }
        };
        Ok(Answer {
            success: status.success(),
            stdout: String::from_utf8_lossy(&stdout.join().unwrap_or_default()).into_owned(),
            stderr: String::from_utf8_lossy(&stderr.join().unwrap_or_default()).into_owned(),
        })
    }
}

/// What a refused `gh` run says, bounded; a sign-in problem is its own code.
pub fn refusal(answer: &Answer) -> Fault {
    let said = answer.stderr.trim();
    let message: String = said.chars().take(MESSAGE_LIMIT).collect();
    if said.contains("gh auth login") || said.contains("not logged in") {
        return Fault::new(
            UNAVAILABLE,
            "gh is not signed in; run gh auth login, then try again.",
        );
    }
    Fault::new(
        REFUSED,
        if message.is_empty() {
            "gh refused the request.".to_owned()
        } else {
            format!("gh refused the request: {message}")
        },
    )
}

/// `#N` to the issue number N; any other external ID belongs to something else.
pub fn issue_number(external_id: &str) -> Option<u64> {
    let digits = external_id.strip_prefix('#')?;
    (!digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse::<u64>().ok())
        .flatten()
        .filter(|number| *number <= MAX_NUMBER)
}

/// The text a pushed issue carries, hidden in a comment, that finds it again if its number was never
/// recorded. One alphanumeric token, so GitHub's search matches it exactly.
pub fn marker(page_id: &str, task_id: &str) -> String {
    let page: String = page_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    let task: String = task_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    format!("tmtcolab{page}{task}")
}

/// A page's text with every `#N` and `GH-N` reference broken by a zero-width space, so it can neither
/// cross-link nor close another issue.
pub fn neutralise(text: &str) -> String {
    const BREAK: char = '\u{200b}';
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (at, &c) in chars.iter().enumerate() {
        out.push(c);
        let next_digit = chars.get(at + 1).is_some_and(char::is_ascii_digit);
        let after_gh = at >= 2
            && c == '-'
            && chars[at - 2].eq_ignore_ascii_case(&'g')
            && chars[at - 1].eq_ignore_ascii_case(&'h');
        if next_digit && (c == '#' || after_gh) {
            out.push(BREAK);
        }
    }
    out
}

/// `text` in a code fence longer than any run of backticks inside it, so nothing in it renders:
/// no mention notifies anyone and no link is made.
fn fenced(text: &str) -> String {
    let mut run = 0;
    let mut longest = 0;
    for c in text.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}text\n{text}\n{fence}")
}

/// The issue's title: the task's title, neutralised.
pub fn issue_title(task: &Value) -> String {
    neutralise(task["title"].as_str().unwrap_or_default())
}

/// The issue's body for a task of a page. The page itself is not linked: it stays private.
pub fn issue_body(page_id: &str, task_id: &str, task: &Value) -> String {
    let mut body = format!(
        "<!-- {} -->\nPushed from a Colab page task by `tmt colab task github push`.\n",
        marker(page_id, task_id)
    );
    if let Some(detail) = task["detail"].as_str().filter(|detail| !detail.is_empty()) {
        body.push_str("\n**Detail**\n\n");
        body.push_str(&fenced(&neutralise(detail)));
        body.push('\n');
    }
    let criteria: Vec<String> = task["acceptance"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|criterion| match criterion["kind"].as_str() {
            Some("test") => format!("test: {}", criterion["name"].as_str().unwrap_or_default()),
            Some("check") => format!(
                "check: {}",
                criterion["command"].as_str().unwrap_or_default()
            ),
            _ => format!("person: {}", criterion["who"].as_str().unwrap_or_default()),
        })
        .collect();
    if !criteria.is_empty() {
        body.push_str("\n**Acceptance**\n\n");
        body.push_str(&fenced(&neutralise(&criteria.join("\n"))));
        body.push('\n');
    }
    body
}

/// The issue number a `gh issue create` URL names, when it is an issue of this repo.
pub fn created_number(repo: &Repo, url: &str) -> Option<u64> {
    let url = url.trim();
    let (_, path) = url.split_once("://")?.1.split_once('/')?;
    let number = path.strip_prefix(&format!("{}/issues/", repo.slug()))?;
    (!number.starts_with('0') && number.bytes().all(|b| b.is_ascii_digit()))
        .then(|| number.parse::<u64>().ok())
        .flatten()
        .filter(|number| *number <= MAX_NUMBER)
}

/// How an issue stands for a task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    Open,
    /// Closed, but not by a merged pull request: by hand, or by one that never merged.
    ClosedOther,
    /// Closed by a pull request that merged: GitHub's own `Fixes #N`.
    ClosedByMerge,
    Missing,
}

/// The calls one sync run makes for these issues: each asks about at most `BATCH` of them, the run
/// asks about at most `RUN_CAP`, and the rest are left for a rerun.
pub fn plan(numbers: &[u64]) -> (Vec<&[u64]>, &[u64]) {
    let (asked, rest) = numbers.split_at(numbers.len().min(RUN_CAP));
    (asked.chunks(BATCH).collect(), rest)
}

/// One GraphQL call for these issues of the repo (at most `BATCH`), each under an alias of its number.
pub fn query(repo: &Repo, numbers: &[u64]) -> String {
    let issues: String = numbers
        .iter()
        .map(|n| {
            format!(
                " i{n}:issue(number:{n}){{state closedByPullRequestsReferences(first:5){{nodes{{merged}}}}}}"
            )
        })
        .collect();
    format!(
        "query{{repository(owner:\"{}\",name:\"{}\"){{{issues} }}}}",
        repo.owner, repo.name
    )
}

/// What the answer says of each asked issue. An answer that does not hold the repository at all
/// (gh refused, or the repository is not there) is a fault; an issue that is not there is `Missing`.
pub fn standings(answer: &Answer, numbers: &[u64]) -> Result<BTreeMap<u64, Standing>, Fault> {
    let parsed: Option<Value> = serde_json::from_str(&answer.stdout).ok();
    let Some(repository) = parsed
        .as_ref()
        .map(|value| &value["data"]["repository"])
        .filter(|repository| repository.is_object())
    else {
        return Err(if answer.success {
            Fault::new(REFUSED, "gh answered without the repository's issues.")
        } else {
            refusal(answer)
        });
    };
    Ok(numbers
        .iter()
        .map(|&n| {
            let issue = &repository[format!("i{n}")];
            let standing = match issue["state"].as_str() {
                None => Standing::Missing,
                Some("CLOSED") => {
                    if issue["closedByPullRequestsReferences"]["nodes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|pull| pull["merged"] == true)
                    {
                        Standing::ClosedByMerge
                    } else {
                        Standing::ClosedOther
                    }
                }
                Some(_) => Standing::Open,
            };
            (n, standing)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_repo_is_owner_slash_name_and_never_an_option() {
        assert_eq!(Repo::parse("pj-tmt/tmt").unwrap().slug(), "pj-tmt/tmt");
        assert!(Repo::parse("a.b/c_d-e").is_some());
        for bad in [
            "", "tmt", "a/", "/b", "a/b/c", "-a/b", "a/-b", ".a/b", "a/..", "a b/c", "a/b\"c",
        ] {
            assert!(Repo::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn only_hash_numbers_name_an_issue() {
        assert_eq!(issue_number("#4053"), Some(4053));
        for other in [
            "",
            "#",
            "#0",
            "#012",
            "GH-4050",
            "4050",
            "#12a",
            "#-1",
            "#1000000000",
        ] {
            assert_eq!(issue_number(other), None, "{other}");
        }
    }

    #[test]
    fn references_are_broken_so_a_page_cannot_cross_link() {
        assert_eq!(
            neutralise("Fixes #12 and gh-7, GH-8"),
            "Fixes #\u{200b}12 and gh-\u{200b}7, GH-\u{200b}8"
        );
        // Other text is left exactly as it is.
        assert_eq!(
            neutralise("# heading, #tag, C-3, 2026-10-21"),
            "# heading, #tag, C-3, 2026-10-21"
        );
        assert_eq!(neutralise("日本語 #5"), "日本語 #\u{200b}5");
    }

    #[test]
    fn a_body_fences_the_page_text_and_carries_the_marker() {
        let task = json!({
            "title": "Close #9",
            "detail": "Use ``` carefully @ada #3",
            "acceptance": [
                {"kind": "test", "name": "guide builds"},
                {"kind": "check", "command": "pnpm docs:check"},
                {"kind": "person", "who": "lead"}
            ]
        });
        let body = issue_body("10000000-0000-4000-8000-000000000001", "T-3", &task);
        assert!(body.starts_with("<!-- tmtcolab10000000000040008000000000000001T3 -->\n"));
        // The detail's own run of three backticks cannot end the fence: this one is four.
        assert!(body.contains("````text\nUse ``` carefully @ada #\u{200b}3\n````"));
        assert!(body.contains("test: guide builds\ncheck: pnpm docs:check\nperson: lead"));
        assert_eq!(issue_title(&task), "Close #\u{200b}9");
        assert!(
            !body.contains("10000000-0000"),
            "the page's ID is not published as such"
        );
    }

    #[test]
    fn a_created_url_names_its_issue_only_in_this_repo() {
        let repo = Repo::parse("pj-tmt/tmt").unwrap();
        assert_eq!(
            created_number(&repo, "https://github.com/pj-tmt/tmt/issues/77\n"),
            Some(77)
        );
        for bad in [
            "https://github.com/pj-tmt/other/issues/77",
            "https://github.com/pj-tmt/tmt/pull/77",
            "https://github.com/pj-tmt/tmt/issues/0",
            "https://github.com/pj-tmt/tmt/issues/7x",
            "created",
        ] {
            assert_eq!(created_number(&repo, bad), None, "{bad}");
        }
    }

    #[test]
    fn a_run_asks_in_batches_of_a_hundred_up_to_its_cap_and_leaves_the_rest() {
        let numbers: Vec<u64> = (1..=450).collect();
        let (calls, rest) = plan(&numbers);
        assert_eq!(
            calls.iter().map(|call| call.len()).collect::<Vec<_>>(),
            [100, 100]
        );
        assert_eq!((calls[1][99], rest.len(), rest[0]), (200, 250, 201));
        let (calls, rest) = plan(&numbers[..101]);
        assert_eq!(
            calls.iter().map(|call| call.len()).collect::<Vec<_>>(),
            [100, 1]
        );
        assert!(rest.is_empty());
        assert!(plan(&[]).0.is_empty());
    }

    #[test]
    fn the_query_asks_each_issue_under_its_number() {
        let repo = Repo::parse("pj-tmt/tmt").unwrap();
        let text = query(&repo, &[4053, 7]);
        assert!(text.starts_with("query{repository(owner:\"pj-tmt\",name:\"tmt\"){"));
        assert!(text.contains(
            "i4053:issue(number:4053){state closedByPullRequestsReferences(first:5){nodes{merged}}}"
        ));
        assert!(text.contains("i7:issue(number:7)"));
    }

    fn answer(success: bool, stdout: Value) -> Answer {
        Answer {
            success,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    #[test]
    fn an_issue_is_done_only_when_a_merged_pull_request_closed_it() {
        let reply = answer(
            true,
            json!({"data": {"repository": {
                "i1": {"state": "CLOSED", "closedByPullRequestsReferences": {"nodes": [{"merged": true}]}},
                "i2": {"state": "CLOSED", "closedByPullRequestsReferences": {"nodes": [{"merged": false}]}},
                "i3": {"state": "CLOSED", "closedByPullRequestsReferences": {"nodes": []}},
                "i4": {"state": "OPEN", "closedByPullRequestsReferences": {"nodes": [{"merged": true}]}},
                "i5": null
            }}}),
        );
        let found = standings(&reply, &[1, 2, 3, 4, 5]).unwrap();
        assert_eq!(
            found.values().copied().collect::<Vec<_>>(),
            [
                Standing::ClosedByMerge,
                Standing::ClosedOther,
                Standing::ClosedOther,
                Standing::Open,
                Standing::Missing
            ]
        );
    }

    #[test]
    fn a_failed_exit_with_data_still_answers_and_one_without_it_is_a_refusal() {
        // GitHub answers with errors for a missing issue and the data for the rest.
        let partial = answer(
            false,
            json!({"data": {"repository": {"i1": null}}, "errors": [{}]}),
        );
        assert_eq!(standings(&partial, &[1]).unwrap()[&1], Standing::Missing);
        let nothing = Answer {
            success: false,
            stdout: String::new(),
            stderr: "HTTP 502: Bad Gateway".into(),
        };
        let fault = standings(&nothing, &[1]).unwrap_err();
        assert_eq!(fault.code(), REFUSED);
        assert!(fault.message.contains("502"), "{}", fault.message);
        let unsigned = Answer {
            stderr: "To get started with GitHub CLI, please run:  gh auth login".into(),
            ..nothing
        };
        assert_eq!(standings(&unsigned, &[1]).unwrap_err().code(), UNAVAILABLE);
    }

    #[test]
    fn a_missing_program_is_unavailable_and_a_real_one_answers() {
        let missing = Process {
            program: PathBuf::from("/nonexistent/gh-for-the-test"),
        };
        assert_eq!(missing.run(&["--version"]).unwrap_err().code(), UNAVAILABLE);
        let echo = Process {
            program: PathBuf::from("/bin/echo"),
        };
        let answer = echo.run(&["hello"]).unwrap();
        assert!(answer.success && answer.stdout == "hello\n");
    }
}
