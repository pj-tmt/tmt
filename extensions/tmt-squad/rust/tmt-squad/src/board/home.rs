//! Board-only home data. Public aggregate documents keep their existing shape;
//! member selection uses the same acquisition and sections as user tabs.

use crate::{
    attention::Attention,
    config::{Config, Section, SortKey, UserTab},
    core::{Core, SquadError},
    filter::Filter,
    me::Me,
    squad::Squad,
    tab_view::{self, Acquired, Document},
    tabs::ALL,
};
use serde_json::{Value, json};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub members: usize,
    pub waiting: usize,
    pub blocked: usize,
    pub review: usize,
    pub working: usize,
    pub idle: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AgeSource {
    Request,
    Observed,
}

/// The painter can advance age without acquiring data or inventing a timestamp.
#[derive(Debug, PartialEq, Eq)]
pub struct Age {
    pub source: AgeSource,
    pub since_ms: u64,
}

#[derive(Debug)]
pub struct MemberRow {
    pub squad: String,
    /// Retains the public request projection for the existing answer composer.
    /// Home painting must show only member/squad/age, never pending or preview.
    pub member: Value,
    pub lead: Option<String>,
    pub age: Option<Age>,
}

#[derive(Debug)]
pub struct MemberSection {
    /// Stable section keys, independent of display text or cursor position.
    pub key: String,
    pub rows: Vec<MemberRow>,
}

#[derive(Debug)]
pub struct SquadLine {
    pub squad: String,
    pub lead: Option<Value>,
    pub counts: Counts,
    /// Exclusive non-lead membership counts, with one urgency state per known member.
    pub members: Counts,
}

#[derive(Debug)]
pub struct Home {
    pub summary: Counts,
    pub sections: Vec<MemberSection>,
    pub squads: Vec<SquadLine>,
    pub failures: Vec<Value>,
    pub incomplete: bool,
}

pub fn load(
    core: &Core,
    config: &Config,
    squads: &[Squad],
    order: &[String],
    me: Option<&Me>,
) -> Result<
    (
        Document,
        Home,
        std::collections::BTreeMap<String, super::app::RateView>,
    ),
    SquadError,
> {
    let settings = config.tabs()?;
    let squads = squads.iter().collect::<Vec<_>>();
    let acquired = tab_view::home_sources(core, config, &squads, me);
    let public = tab_view::document(config, &settings, order, ALL, &acquired)?;
    let inputs = squads
        .iter()
        .filter(|squad| acquired.documents.contains_key(&squad.name))
        .map(|squad| {
            Ok((
                squad.name.clone(),
                super::app::RateView {
                    settings: config.token_rate(&squad.name)?,
                    input: super::rate::Input {
                        room: squad.room_id.clone(),
                        names: Default::default(),
                        resumes: acquired
                            .member_ids(&squad.name)
                            .map(|id| (id.to_owned(), Value::Null))
                            .collect(),
                    },
                },
            ))
        })
        .collect::<Result<_, SquadError>>()?;
    Ok((
        public,
        model(order, &acquired, crate::status::now_ms()),
        inputs,
    ))
}

fn section(key: &str, filter: Option<&str>) -> Section {
    Section {
        title: key.into(),
        filter: filter.map(|text| Filter::parse(text).expect("the home section filter")),
        sort: ["squad", "name"]
            .map(|field| SortKey {
                field: field.into(),
                descending: false,
            })
            .into(),
        bind: Default::default(),
    }
}

fn counts(rows: &[Value]) -> Counts {
    let attention = Attention::of(&json!({"sections": [{"rows": rows}]}));
    Counts {
        members: rows.len(),
        waiting: attention.waiting,
        blocked: attention.blocked,
        review: rows.iter().filter(|row| row["state"] == "review").count(),
        working: rows.iter().filter(|row| row["state"] == "working").count(),
        idle: rows.iter().filter(|row| row["state"] == "idle").count(),
    }
}

fn member_counts(rows: &[Value], lead: &Value) -> Counts {
    let mut counts = Counts::default();
    for row in rows {
        if lead["id"].is_string() && row["id"] == lead["id"] {
            continue;
        }
        counts.members += 1;
        if row["pending"].is_string()
            || row["waitingOnYou"]
                .as_array()
                .is_some_and(|requests| !requests.is_empty())
        {
            counts.waiting += 1;
        } else {
            match row["state"].as_str() {
                Some("blocked") => counts.blocked += 1,
                Some("review") => counts.review += 1,
                Some("working") => counts.working += 1,
                Some("idle") => counts.idle += 1,
                // Custom or unreported states still count as members.
                _ => (),
            }
        }
    }
    counts
}

fn model(order: &[String], acquired: &Acquired, now: u64) -> Home {
    let mut tab = UserTab {
        name: "home".into(),
        selection: section("home", None),
        sections: Vec::new(),
    };
    let all = tab_view::user_document(&tab, order, acquired);
    let members = all["sections"][0]["rows"]
        .as_array()
        .expect("projected members");
    tab.sections = vec![
        section("needs-you", Some("pending or waiting_on_you")),
        section("blocked", Some("state = blocked")),
    ];
    let selected = tab_view::user_document(&tab, order, acquired);
    // The shared pipeline retains unmatched rows; they belong in squad lines,
    // not in a third attention section.
    let sections = selected["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .take(2)
        .map(|section| {
            let key = section["title"].as_str().expect("named home section");
            let rows = section["rows"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|member| {
                    let squad = member["squad"].as_str().expect("projected squad");
                    let since = if key == "needs-you" {
                        member["waitingOnYou"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|request| request["preparedAtMs"].as_u64())
                            .filter(|since| *since <= now)
                            .min()
                    } else {
                        member["staleness"]["unchangedSinceMs"]
                            .as_u64()
                            .filter(|since| *since <= now)
                    };
                    MemberRow {
                        squad: squad.into(),
                        member: member.clone(),
                        lead: acquired.documents[squad]["squad"]["lead"]["name"]
                            .as_str()
                            .map(str::to_owned),
                        age: since.map(|since_ms| Age {
                            source: if key == "needs-you" {
                                AgeSource::Request
                            } else {
                                AgeSource::Observed
                            },
                            since_ms,
                        }),
                    }
                })
                .collect();
            MemberSection {
                key: key.into(),
                rows,
            }
        })
        .collect::<Vec<_>>();
    // Public all order already includes hidden squads after the tab line.
    let overview = tab_view::all_document(
        order,
        &acquired.documents,
        &tab_view::tab_attention(&acquired.documents),
    );
    let mut summary = Counts::default();
    let squads = overview["sections"][0]["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|squad| {
            let name = squad["name"].as_str().expect("squad overview name");
            let rows = members
                .iter()
                .filter(|row| row["squad"] == name)
                .cloned()
                .collect::<Vec<_>>();
            let one = counts(&rows);
            summary.members += one.members;
            summary.waiting += one.waiting;
            summary.blocked += one.blocked;
            summary.review += one.review;
            summary.working += one.working;
            summary.idle += one.idle;
            let lead = &acquired.documents[name]["squad"]["lead"];
            SquadLine {
                squad: name.into(),
                lead: lead.is_object().then(|| lead.clone()),
                counts: one,
                members: member_counts(&rows, lead),
            }
        })
        .collect();
    Home {
        summary,
        sections,
        squads,
        failures: acquired.failures.clone(),
        incomplete: acquired
            .documents
            .values()
            .any(|document| document["olderRequestsNotShown"] == true),
    }
}

#[cfg(test)]
mod tests;

mod controller;
mod paint;
pub(super) use controller::{Send, Target};
pub(super) use paint::{age_label, hints, render, summary};

mod tiles;
