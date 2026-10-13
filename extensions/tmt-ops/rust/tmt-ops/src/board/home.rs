//! Board-only home data. Public aggregate documents keep their existing shape;
//! workspace selection uses the acquired UUID roster, independently of summaries.

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

pub(super) use tmt_cli_style::breakpoint::{MD, SM};
/// HOME-only compact and two-meter design checkpoints; not shared token roles.
pub(super) const COMPACT: u16 = 60;
pub(super) const WIDE: u16 = 160;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub members: usize,
    pub waiting: usize,
    pub blocked: usize,
    pub review: usize,
    pub working: usize,
    pub idle: usize,
}

#[derive(Debug)]
pub struct SquadLine {
    pub squad: String,
    pub lead: Option<Value>,
    pub counts: Counts,
}

#[derive(Debug)]
pub struct Home {
    pub(crate) workspace: super::workspace::observe::OpsWorkspaceSnapshot,
    pub summary: Counts,
    /// Acquired global display choice; never inferred from a squad override.
    pub window: crate::config::TokenWindow,
    /// Overview uses global windows; per-squad samplers retain their overrides.
    pub windows: [crate::config::TokenWindow; 3],
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
    listed: &Result<Value, SquadError>,
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
    // Models are public session observations, independent of token sampling.
    let mut acquired = tab_view::home_sources(core, config, &squads, me);
    acquired.digest(core);
    let public = tab_view::document(config, &settings, order, ALL, &acquired)?;
    let resumes = match listed {
        Ok(listed) => super::rate::Input::resumes(listed),
        Err(error) => {
            let mut failure = error.to_json();
            failure["source"] = json!("models");
            acquired.failures.push(failure);
            Default::default()
        }
    };
    let inputs = squads
        .iter()
        .filter(|squad| acquired.documents.contains_key(&squad.name))
        .map(|squad| {
            Ok((
                squad.name.clone(),
                super::app::RateView {
                    history: None,
                    settings: config.token_rate(&squad.name)?,
                    input: super::rate::Input {
                        room: squad.room_id.clone(),
                        names: acquired.member_names(&squad.name),
                        resumes: acquired
                            .member_ids(&squad.name)
                            .map(|id| {
                                (
                                    id.to_owned(),
                                    resumes.get(id).cloned().unwrap_or(Value::Null),
                                )
                            })
                            .collect(),
                    },
                },
            ))
        })
        .collect::<Result<_, SquadError>>()?;
    let mut home = model(order, &acquired);
    home.workspace =
        super::workspace::observe::OpsWorkspaceSnapshot::from_acquired(&squads, &acquired, listed);
    home.workspace.configure(config)?;
    let settings = config.token_rate("")?;
    home.windows = settings.windows;
    home.window = settings.window.available(home.windows);
    Ok((public, home, inputs))
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

fn model(order: &[String], acquired: &Acquired) -> Home {
    let tab = UserTab {
        name: "home".into(),
        selection: section("home", None),
        sections: Vec::new(),
    };
    let all = tab_view::user_document(&tab, order, acquired);
    let members = all["sections"][0]["rows"]
        .as_array()
        .expect("projected members");
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
            }
        })
        .collect();
    Home {
        workspace: Default::default(),
        summary,
        window: crate::config::TokenWindow::MINUTE,
        windows: crate::config::TokenWindow::DEFAULTS,
        squads,
        failures: acquired.failures.clone(),
        incomplete: acquired
            .documents
            .values()
            .any(|document| document["olderRequestsNotShown"] == true),
    }
}

#[cfg(test)]
pub(super) mod tests;

pub(crate) mod counters;
mod paint;
pub(super) use bar::{hidden_needs, hidden_usage};
pub(super) use limits::clock_marks as limit_clock_marks;
pub(super) use limits::hidden_limits;
pub(super) use paint::{limits_of, summary_of, usage_of};

/// Held header strips of one immutable view, keyed by data, width and look.
/// Workspace owns body geometry; a new snapshot starts with empty derivations.
#[derive(Default)]
pub(super) struct Scenes {
    /// The strips outside the body.
    summary: scene::Kept<ratatui::text::Line<'static>>,
    usage: scene::Kept<Option<ratatui::text::Line<'static>>>,
    limits: scene::Kept<Vec<ratatui::text::Line<'static>>>,
}

impl Scenes {
    /// How many scenes were solved and painted since the view began.
    #[cfg(test)]
    pub(super) fn builds(&self) -> usize {
        self.summary.builds + self.usage.builds + self.limits.builds
    }
}

mod bar;
mod limits;
use super::view::scene;
