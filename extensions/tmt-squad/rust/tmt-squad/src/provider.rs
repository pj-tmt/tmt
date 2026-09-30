//! Field providers: `[squad.<name>.fields.<field>]` runs a program of the
//! user's for each member and shows its output as that field, for data TMT
//! does not have, such as a pull request's review state from `gh`.
//!
//! Providers come only from the user's own squad.toml. A program runs
//! directly, never through a shell: each `{field}` fills exactly one argument
//! (the run-binding policy, `Template::fill_argument`), output and time are
//! bounded, and a failure shows as `?`, never as an error screen. Values are
//! cached per squad in Squad's cache directory with the argv that produced
//! them, so a changed `{pr_link}` never shows the old link's value. Readers
//! (the board, `ls`) only read the cache; runs happen off the board's paint.

use crate::{cache, core::SquadError, squad::Member, template::Template};
use serde_json::{Map, Value, json};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use toml_edit::{Item, TableLike};

const MAX_PROVIDERS: usize = 8;
const OUTPUT_LIMIT: usize = 4 * 1024;
/// A shown value is one short line.
const VALUE_LIMIT: usize = 200;
/// Programs running at once, across all members and providers.
const PARALLEL: usize = 4;
const DEFAULT_EVERY: Duration = Duration::from_secs(60);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    pub name: String,
    run: Vec<Template>,
    every: Duration,
    timeout: Duration,
    output: Output,
}

/// How a provider's output becomes its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Output {
    /// The first line, or `{"value", "color"}`.
    Line,
    /// `gh pr view --json number,state,isDraft,reviewDecision`.
    GithubPr,
}

/// The `github-pr` preset: a member's pull request from its `pr_link`.
const GITHUB_PR: &[&str] = &[
    "gh",
    "pr",
    "view",
    "{pr_link}",
    "--json",
    "number,state,isDraft,reviewDecision",
];

impl Provider {
    /// How often a member's value is run again.
    pub fn every(&self) -> Duration {
        self.every
    }
}

fn invalid(message: impl Into<String>) -> SquadError {
    SquadError::new("SQUAD_CONFIG_INVALID", message)
}

/// `"<n>s"`, `"<n>m"` or `"<n>h"` within `range` seconds.
fn duration(
    item: &Item,
    place: &str,
    range: std::ops::RangeInclusive<u64>,
) -> Result<Duration, SquadError> {
    let wrong = || {
        invalid(format!(
            "`{place}` must be {}s-{}s, such as \"30s\", \"5m\" or \"1h\".",
            range.start(),
            range.end()
        ))
    };
    let text = item.as_str().ok_or_else(wrong)?;
    let (number, unit) = text.split_at(text.len().saturating_sub(1));
    let seconds = match (number.parse::<u64>(), unit) {
        (Ok(number), "s") => number,
        (Ok(number), "m") => number.saturating_mul(60),
        (Ok(number), "h") => number.saturating_mul(3_600),
        _ => return Err(wrong()),
    };
    range
        .contains(&seconds)
        .then(|| Duration::from_secs(seconds))
        .ok_or_else(wrong)
}

fn read_one(name: &str, table: &dyn TableLike, place: &str) -> Result<Provider, SquadError> {
    let mut run = None;
    let mut preset = None;
    let mut every = DEFAULT_EVERY;
    let mut timeout = DEFAULT_TIMEOUT;
    for (key, value) in table.iter() {
        let here = format!("{place}.{key}");
        match key {
            "run" => {
                let malformed = || {
                    invalid(format!(
                        "`{here}` must be a program and its arguments, such as [\"gh\", \"pr\", \"view\", \"{{pr_link}}\"]."
                    ))
                };
                let argv = value
                    .as_array()
                    .filter(|argv| (1..=32).contains(&argv.len()))
                    .ok_or_else(malformed)?
                    .iter()
                    .map(|arg| {
                        let text = arg.as_str().ok_or_else(malformed)?;
                        Template::parse(text)
                            .map_err(|error| invalid(format!("`{here}`: {error}.")))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                match argv[0].literal() {
                    Some(program) if program.starts_with('/') || !program.contains('/') => {}
                    _ => {
                        return Err(invalid(format!(
                            "`{here}` must start with a literal program name on PATH or an absolute path."
                        )));
                    }
                }
                run = Some(argv);
            }
            "preset" => match value.as_str() {
                Some("github-pr") => preset = Some(Output::GithubPr),
                _ => return Err(invalid(format!("`{here}` must be \"github-pr\"."))),
            },
            "every" => every = duration(value, &here, 10..=86_400)?,
            "timeout" => timeout = duration(value, &here, 1..=30)?,
            other => {
                return Err(invalid(format!(
                    "`{place}.{other}` is not a field provider setting; use run or preset, every and timeout."
                )));
            }
        }
    }
    let (run, output) = match (run, preset) {
        (Some(_), Some(_)) => {
            return Err(invalid(format!(
                "`{place}` sets both `run` and `preset`; keep one."
            )));
        }
        (Some(run), None) => (run, Output::Line),
        (None, Some(preset)) => (
            GITHUB_PR
                .iter()
                .map(|arg| Template::parse(arg).expect("the preset's arguments parse"))
                .collect(),
            preset,
        ),
        (None, None) => {
            return Err(invalid(format!(
                "`{place}` needs `run` or `preset = \"github-pr\"`."
            )));
        }
    };
    Ok(Provider {
        name: name.to_owned(),
        run,
        every,
        timeout,
        output,
    })
}

/// `[squad.<name>.fields]`, one table per provided field, in name order.
/// `own` refuses names Squad reads itself.
pub fn read(
    squad: Option<&dyn TableLike>,
    name: &str,
    field_name: impl Fn(&str) -> bool,
    own: impl Fn(&str) -> bool,
) -> Result<Vec<Provider>, SquadError> {
    let place = format!("squad.{name}.fields");
    let Some(item) = squad.and_then(|table| table.get("fields")) else {
        return Ok(Vec::new());
    };
    let table = item
        .as_table_like()
        .ok_or_else(|| invalid(format!("`{place}` must be a table of fields.")))?;
    if table.len() > MAX_PROVIDERS {
        return Err(invalid(format!(
            "`{place}` may define at most {MAX_PROVIDERS} fields."
        )));
    }
    table
        .iter()
        .map(|(field, settings)| {
            let here = format!("{place}.{field}");
            if !field_name(field) {
                return Err(invalid(format!("`{here}` must be named like a field.")));
            }
            if own(field) {
                return Err(invalid(format!(
                    "`{here}`: `{field}` is a field Squad reads itself; name the provider another way."
                )));
            }
            let settings = settings
                .as_table_like()
                .ok_or_else(|| invalid(format!("`{here}` must be a table.")))?;
            read_one(field, settings, &here)
        })
        .collect()
}

/// The argv for one member, or None when a placeholder is missing or refused
/// (a value that would start an argument with `-`): the provider does not
/// run for that member, and its field shows as missing.
fn argv(provider: &Provider, row: &Value) -> Option<Vec<String>> {
    provider
        .run
        .iter()
        .map(|arg| arg.fill_argument(row).ok())
        .collect()
}

/// One program run for one member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    field: String,
    member: String,
    argv: Vec<String>,
    timeout: Duration,
    output: Output,
}

/// What a run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Value {
        value: String,
        color: Option<String>,
    },
    Empty,
    Failed,
}

/// Cached values of one squad's providers:
/// `{<field>: {<member id>: {argv, value, color, atMs, failed}}}`.
pub struct Cache {
    path: Option<PathBuf>,
    document: Map<String, Value>,
}

impl Cache {
    /// Where every squad's cache lives; its modification time moves when any
    /// squad's values are saved.
    pub fn directory() -> Option<PathBuf> {
        cache::directory("fields")
    }

    /// Unreadable or corrupt content reads as empty: providers run again.
    pub fn load(squad: &str) -> Self {
        let path = Self::directory().map(|directory| directory.join(format!("{squad}.json")));
        Self::at(path)
    }

    pub(crate) fn at(path: Option<PathBuf>) -> Self {
        let document = path
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        Self { path, document }
    }

    fn entry(&self, field: &str, member: &str) -> Option<&Value> {
        self.document.get(field)?.get(member)
    }

    pub fn record(&mut self, job: &Job, outcome: &Outcome, now_ms: u64) {
        let (value, color, failed) = match outcome {
            Outcome::Value { value, color } => (json!(value), json!(color), false),
            Outcome::Empty => (Value::Null, Value::Null, false),
            Outcome::Failed => (Value::Null, Value::Null, true),
        };
        let pr_state = if job.output == Output::GithubPr {
            value
                .as_str()
                .and_then(|value| value.split_whitespace().nth(1))
                .filter(|state| matches!(*state, "draft" | "open" | "closed" | "merged"))
                .map(str::to_owned)
        } else {
            None
        };
        let field = self
            .document
            .entry(job.field.clone())
            .or_insert_with(|| json!({}));
        field[&job.member] = json!({
            "argv": job.argv, "value": value, "color": color, "atMs": now_ms, "failed": failed, "prState": pr_state,
        });
    }

    /// Successful, current github-pr preset states only. Arbitrary text
    /// providers never become activity evidence, and this executes nothing.
    pub fn github_pr_states(
        &self,
        providers: &[Provider],
        member: &Member,
        now_ms: u64,
    ) -> std::collections::BTreeMap<String, String> {
        providers
            .iter()
            .filter(|provider| provider.output == Output::GithubPr)
            .filter_map(|provider| {
                let argv = argv(provider, &crate::status::row(member))?;
                let entry = current(self, provider, member, &argv)?;
                let at = entry["atMs"].as_u64()?;
                if entry["failed"] != false
                    || now_ms.checked_sub(at)? > provider.every.as_millis() as u64
                {
                    return None;
                }
                let state = entry["prState"].as_str()?;
                matches!(state, "draft" | "open" | "closed" | "merged")
                    .then(|| (provider.name.clone(), state.to_owned()))
            })
            .collect()
    }

    pub fn save(&self) -> std::io::Result<()> {
        match &self.path {
            Some(path) => cache::replace(
                path,
                Value::Object(self.document.clone()).to_string().as_bytes(),
            ),
            None => Ok(()),
        }
    }
}

/// The cached result for this member's current argv, when there is one.
fn current<'a>(
    cache: &'a Cache,
    provider: &Provider,
    member: &Member,
    argv: &[String],
) -> Option<&'a Value> {
    cache
        .entry(&provider.name, &member.id)
        .filter(|entry| entry["argv"] == json!(argv))
}

/// Every run that is due: no result yet for the member's current argv, or
/// one older than the provider's `every`.
pub fn due(providers: &[Provider], members: &[Member], cache: &Cache, now_ms: u64) -> Vec<Job> {
    let mut jobs = Vec::new();
    for provider in providers {
        let every = u64::try_from(provider.every.as_millis()).unwrap_or(u64::MAX);
        for member in members {
            let Some(argv) = argv(provider, &crate::status::row(member)) else {
                continue;
            };
            let fresh = current(cache, provider, member, &argv)
                .and_then(|entry| entry["atMs"].as_u64())
                .is_some_and(|at| now_ms.saturating_sub(at) < every);
            if !fresh {
                jobs.push(Job {
                    field: provider.name.clone(),
                    member: member.id.clone(),
                    argv,
                    timeout: provider.timeout,
                    output: provider.output,
                });
            }
        }
    }
    jobs
}

/// Each member's cached value as its field of the provider's name, `?` (and
/// marked failed) after a failed run; nothing when there is no current
/// value. Replaces a same-named field an agent wrote: the user's provider
/// decides.
pub fn apply(providers: &[Provider], members: &mut [Member], cache: &Cache) {
    for provider in providers {
        for member in members.iter_mut() {
            let entry = argv(provider, &crate::status::row(member))
                .and_then(|argv| current(cache, provider, member, &argv).cloned());
            member.fields.remove(&provider.name);
            member.failed.remove(&provider.name);
            let Some(entry) = entry else {
                continue;
            };
            if entry["failed"] == true {
                member.fields.insert(provider.name.clone(), "?".into());
                member.failed.insert(provider.name.clone());
            } else if let Some(value) = entry["value"].as_str() {
                member
                    .fields
                    .insert(provider.name.clone(), value.to_owned());
            }
        }
    }
}

/// One line of printable text, at most [`VALUE_LIMIT`] characters.
fn clean(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(VALUE_LIMIT)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Plain text is the value; `{"value": …, "color": "<token>"}` also names a
/// color token, kept for the board's theme colors (#514).
fn outcome(stdout: &[u8]) -> Outcome {
    let text = String::from_utf8_lossy(stdout);
    let text = text.trim();
    let (value, color) = match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(object)) => {
            let value = match object.get("value") {
                Some(Value::String(value)) => value.clone(),
                Some(Value::Number(number)) => number.to_string(),
                _ => String::new(),
            };
            let color = object
                .get("color")
                .and_then(Value::as_str)
                .filter(|token| crate::rows::field_name(token))
                .map(str::to_owned);
            (value, color)
        }
        _ => (text.to_owned(), None),
    };
    match clean(&value) {
        value if value.is_empty() => Outcome::Empty,
        value => Outcome::Value { value, color },
    }
}

/// `#412 open`, `#412 draft`, `#412 merged` or `#412 closed`; an open pull
/// request adds its review: `#412 open · approved`, `· changes requested` or
/// `· review required`. Anything else from `gh` is a failed run.
fn github_pr(stdout: &[u8]) -> Outcome {
    let Ok(pr) = serde_json::from_slice::<Value>(stdout) else {
        return Outcome::Failed;
    };
    let (Some(number), Some(state)) = (pr["number"].as_u64(), pr["state"].as_str()) else {
        return Outcome::Failed;
    };
    let state = match (state, pr["isDraft"] == true) {
        ("OPEN", true) => "draft",
        ("OPEN", false) => "open",
        ("MERGED", _) => "merged",
        ("CLOSED", _) => "closed",
        _ => return Outcome::Failed,
    };
    let review = match pr["reviewDecision"].as_str() {
        Some("APPROVED") => Some("approved"),
        Some("CHANGES_REQUESTED") => Some("changes requested"),
        Some("REVIEW_REQUIRED") => Some("review required"),
        _ => None,
    };
    let value = match review.filter(|_| state == "open") {
        Some(review) => format!("#{number} {state} · {review}"),
        None => format!("#{number} {state}"),
    };
    Outcome::Value { value, color: None }
}

/// Runs one job: a failure to start, a non-zero exit, a timeout or too much
/// output is a failed run.
fn execute(job: &Job) -> Outcome {
    let (program, args) = job.argv.split_first().expect("a provider has a program");
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    match crate::runner::run(Path::new(program), &args, b"", job.timeout, OUTPUT_LIMIT) {
        Ok(finished) if finished.success => match job.output {
            Output::Line => outcome(&finished.stdout),
            Output::GithubPr => github_pr(&finished.stdout),
        },
        _ => Outcome::Failed,
    }
}

/// Runs every job, [`PARALLEL`] at a time, and returns each outcome in the
/// jobs' order.
pub fn run(jobs: &[Job]) -> Vec<Outcome> {
    let next = Mutex::new(0..jobs.len());
    let outcomes = Mutex::new(vec![Outcome::Failed; jobs.len()]);
    std::thread::scope(|scope| {
        for _ in 0..PARALLEL.min(jobs.len()) {
            scope.spawn(|| {
                loop {
                    let Some(index) = next.lock().expect("job queue").next() else {
                        break;
                    };
                    let outcome = execute(&jobs[index]);
                    outcomes.lock().expect("outcomes")[index] = outcome;
                }
            });
        }
    });
    outcomes.into_inner().expect("outcomes")
}

/// Runs what is due for `members` against the squad's saved cache, records
/// the outcomes and saves it. Returns how many programs ran.
pub fn refresh(squad: &str, providers: &[Provider], members: &[Member], now_ms: u64) -> usize {
    refresh_in(Cache::load(squad), providers, members, now_ms)
}

fn refresh_in(mut cache: Cache, providers: &[Provider], members: &[Member], now_ms: u64) -> usize {
    let jobs = due(providers, members, &cache, now_ms);
    if jobs.is_empty() {
        return 0;
    }
    for (job, outcome) in jobs.iter().zip(run(&jobs)) {
        cache.record(job, &outcome, now_ms);
    }
    // A cache that cannot be written only means the values run again.
    let _ = cache.save();
    jobs.len()
}

#[cfg(test)]
mod tests;
