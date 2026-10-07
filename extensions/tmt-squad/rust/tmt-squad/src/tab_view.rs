//! Cross-squad tab documents shared by the board and `ls --tab`.
//! Acquisition stays behind public core commands; renderers consume one result.

use crate::{
    attention::Attention,
    config::{Config, Rank, SortKey, UserTab},
    core::{Core, SquadError},
    filter::Row,
    requests,
    rows::Rows,
    squad::Squad,
    status,
    tabs::{self, ALL, LEADS},
};
use serde_json::{Value, json};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

pub struct Document {
    pub document: Value,
    pub rows: Rows,
    pub attention: BTreeMap<String, Attention>,
}

/// A row retains numeric values and source state ranks outside its public JSON.
pub(crate) struct ProjectedRow {
    pub value: Value,
    numbers: BTreeMap<String, f64>,
    rank: Rank,
}
impl Row for ProjectedRow {
    fn value(&self, field: &str) -> Option<&str> {
        self.value.value(field)
    }
}
impl ProjectedRow {
    fn compare(&self, key: &SortKey, other: &Self) -> Ordering {
        status::compare_values(
            key,
            self,
            other,
            (self.numbers.get(&key.field), other.numbers.get(&key.field)),
            (self.rank, other.rank),
        )
    }
}
pub(crate) struct Acquired {
    pub documents: BTreeMap<String, Value>,
    rows: BTreeMap<String, Vec<ProjectedRow>>,
    pub failures: Vec<Value>,
}

/// Fixed aggregate grids, shared by document loading and settings inspection.
pub fn rows(key: &str) -> Rows {
    if key == ALL {
        return Rows::overview();
    }
    let mut rows = Rows::leads();
    if tabs::user_name(key).is_some() {
        rows.columns[1].title = "MEMBER".into();
    }
    rows
}

pub fn key(config: &Config, name: &str) -> Result<String, SquadError> {
    match name {
        "leads" => Ok(LEADS.into()),
        "all" => Ok(ALL.into()),
        _ => {
            let name = name.strip_prefix("tab:").unwrap_or(name);
            config
                .tabs()?
                .user
                .iter()
                .any(|tab| tab.name == name)
                .then(|| tabs::user_key(name))
                .ok_or_else(|| {
                    SquadError::new(
                        "SQUAD_TAB_NOT_FOUND",
                        format!("Tab '{name}' does not exist."),
                    )
                })
        }
    }
}

pub(crate) fn user_document(tab: &UserTab, order: &[String], acquired: &Acquired) -> Value {
    let mut rows = in_tab_order(order, &acquired.documents)
        .into_iter()
        .flat_map(|name| acquired.rows.get(name).into_iter().flatten())
        .filter(|row| tab.selection.includes(*row))
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        tab.selection
            .sort
            .iter()
            .map(|key| a.compare(key, b))
            .find(|order| order.is_ne())
            .unwrap_or(Ordering::Equal)
    });
    // References retain the one public projection and its sorting metadata.
    let sections = status::sections(
        &rows,
        &tab.sections,
        |key, a, b| a.compare(key, b),
        |row| row.value.clone(),
    );
    json!({"squad": {"name": tab.name, "lead": null}, "sections": sections})
}

pub fn load(
    core: &Core,
    config: &Config,
    squads: &[Squad],
    tabs: &[String],
    me: Option<&crate::me::Me>,
    key: &str,
) -> Result<Document, SquadError> {
    let settings = config.tabs()?;
    let user = settings
        .user
        .iter()
        .find(|tab| Some(tab.name.as_str()) == tabs::user_name(key));
    if !tabs::builtin(key) && user.is_none() {
        return Err(SquadError::new("SQUAD_TAB_NOT_FOUND", "Unknown tab key."));
    }
    let listed = if key != ALL {
        Some(core.json(&["ls"])?)
    } else {
        None
    };
    let all = squads.iter().collect::<Vec<_>>();
    let mut acquired = roster_documents(core, config, &all, me, listed.as_ref());
    acquired.focus(core);
    document(config, &settings, tabs, key, &acquired)
}

/// Shapes acquired rows without reading core again. Board-only compositions
/// keep the public aggregate document on this same path.
pub(crate) fn document(
    config: &Config,
    settings: &crate::config::Tabs,
    tabs: &[String],
    key: &str,
    acquired: &Acquired,
) -> Result<Document, SquadError> {
    let user = settings
        .user
        .iter()
        .find(|tab| Some(tab.name.as_str()) == tabs::user_name(key));
    let documents = &acquired.documents;
    let attention = acquired.attention(config, tabs);
    let (mut document, rows) = match key {
        LEADS => (leads_document(tabs, documents), rows(key)),
        ALL => (all_document(tabs, documents, &attention), rows(key)),
        _ => {
            let rows = rows(key);
            (
                user_document(user.expect("validated user tab"), tabs, acquired),
                rows,
            )
        }
    };
    document["tab"] = json!(tabs::label(key));
    if !acquired.failures.is_empty() {
        document["partial"] = json!(true);
        document["failures"] = json!(&acquired.failures);
    }
    let grid = rows.value();
    document["columns"] = grid["columns"].clone();
    document["lines"] = grid["lines"].clone();
    document["squad"]["attention"] = attention.get(key).copied().unwrap_or_default().document();
    Ok(Document {
        document,
        rows,
        attention,
    })
}

/// The `all` tab's document: a row per squad, `name` the squad's, with its
/// lead, member count and attention counts as fields.
pub(crate) fn all_document(
    tabs: &[String],
    documents: &BTreeMap<String, Value>,
    attention: &BTreeMap<String, Attention>,
) -> Value {
    let rows: Vec<Value> = in_tab_order(tabs, documents)
        .into_iter()
        .map(|name| {
            let document = &documents[name];
            let lead = document["squad"]["lead"]["name"].as_str();
            let members = document["sections"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|section| section["rows"].as_array().into_iter().flatten())
                .filter_map(|row| row["name"].as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            let attention = attention.get(name).copied().unwrap_or_default();
            json!({
                "name": name,
                "squad": name,
                "state": attention.state(),
                "fields": {
                    "squad": name,
                    "lead": lead,
                    "members": members.to_string(),
                    "waiting": attention.waiting.to_string(),
                    "blocked": attention.blocked.to_string(),
                },
            })
        })
        .collect();
    json!({
        "squad": {"name": "all", "lead": null},
        "sections": status::sections(&rows, &[], |_,_,_| Ordering::Equal, Clone::clone),
    })
}

/// Squads in tab order, then squads off the tab line.
fn in_tab_order<'a>(tabs: &'a [String], documents: &'a BTreeMap<String, Value>) -> Vec<&'a String> {
    let mut order: Vec<&String> = tabs
        .iter()
        .filter(|key| documents.contains_key(*key))
        .collect();
    order.extend(documents.keys().filter(|key| !tabs.contains(key)));
    order
}

/// Each squad's roster-only status document, with what waits on the user
/// from one inbox read shared by all, and presence when an `ls` document is
/// given. A squad that cannot be read is left out.
pub(crate) fn roster_documents(
    core: &Core,
    config: &Config,
    squads: &[&Squad],
    me: Option<&crate::me::Me>,
    listed: Option<&Value>,
) -> Acquired {
    roster_documents_with(core, config, squads, me, listed, false)
}

/// Home observes raw member content through the existing staleness owner,
/// without adding notes, room history, presence or provider commands.
pub(crate) fn home_sources(
    core: &Core,
    config: &Config,
    squads: &[&Squad],
    me: Option<&crate::me::Me>,
) -> Acquired {
    roster_documents_with(core, config, squads, me, None, true)
}

fn roster_documents_with(
    core: &Core,
    config: &Config,
    squads: &[&Squad],
    me: Option<&crate::me::Me>,
    listed: Option<&Value>,
    observe: bool,
) -> Acquired {
    let mut acquired = Acquired {
        documents: BTreeMap::new(),
        rows: BTreeMap::new(),
        failures: Vec::new(),
    };
    if squads.is_empty() {
        return acquired;
    }
    let mut failure = |source: &str, squad: Option<&str>, error: SquadError| {
        let mut item = error.to_json();
        item["source"] = json!(source);
        if let Some(name) = squad {
            item["squad"] = json!(name);
        }
        acquired.failures.push(item);
    };
    let waiting = match me {
        Some(me) => match requests::inbox(core, &me.id) {
            Ok(inbox) => Some((me.id.as_str(), inbox)),
            Err(error) => {
                failure("inbox", None, error);
                None
            }
        },
        None => None,
    };
    for squad in squads {
        let result = (|| {
            let layout = config.layout(&squad.name)?;
            let states = config.states(&squad.name, layout)?;
            let sections = config.sections(&squad.name)?;
            let rows = config.rows(&squad.name)?;
            // Lock before the same roster read; concurrent observers cannot
            // publish an older member snapshot over a newer one.
            let observer = observe
                .then(|| {
                    Ok::<_, SquadError>(crate::staleness::Observer::begin(
                        config.path(),
                        squad,
                        config.reminders(&squad.name)?,
                    ))
                })
                .transpose()?;
            let mut members = squad.roster_with(core, rows.reads_metadata())?;
            if let Some(listed) = listed {
                crate::squad::join_presence(&mut members, listed);
            }
            let providers = config.providers(&squad.name)?;
            let cached = crate::provider::Cache::load(&squad.name);
            // Record raw task/state before provider and column projection.
            // Missing optional evidence does not invent notes or activity age.
            let ages = observer.map(|observer| {
                observer.record(&members, &providers, &cached, None, None, status::now_ms())
            });
            crate::provider::apply(&providers, &mut members, &cached);
            status::prepare(&rows, &states, &mut members);
            let mut document =
                status::prepared_document(squad, layout, &states, &sections, members.clone());
            status::sort(&mut members, layout, &states);
            members.sort_by_key(|member| !member.is_lead());
            // Collect once before source sections or lead selection can omit or
            // repeat identities; both documents use the public member projection.
            let mut flat = json!({"squad": {"lead": null}, "sections":
                status::sections(&members, &[], |_, _, _| Ordering::Equal, status::row)});
            if let Some((me, inbox)) = &waiting {
                requests::apply_waiting(&mut document, &squad.name, me, inbox);
                requests::apply_waiting(&mut flat, &squad.name, me, inbox);
            }
            // Ages decorate only the board's retained member projection;
            // source documents (and public all JSON/text) remain unchanged.
            if let Some(ages) = ages {
                ages.apply(&mut flat);
            }
            let projected = project_rows(&squad.name, &flat, &members, &states);
            Ok::<_, SquadError>((document, projected))
        })();
        match result {
            Ok((document, rows)) => {
                acquired.documents.insert(squad.name.clone(), document);
                acquired.rows.insert(squad.name.clone(), rows);
            }
            Err(error) => failure("squad", Some(&squad.name), error),
        }
    }
    acquired
}

fn project_rows(
    squad: &str,
    document: &Value,
    members: &[crate::squad::Member],
    states: &crate::config::States,
) -> Vec<ProjectedRow> {
    let mut seen = BTreeSet::new();
    std::iter::once(&document["squad"]["lead"])
        .chain(
            document["sections"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|section| section["rows"].as_array().into_iter().flatten()),
        )
        .filter_map(|row| {
            let id = row["id"].as_str()?;
            if !seen.insert(id) {
                return None;
            }
            let member = members.iter().find(|member| member.id == id);
            let mut value = row.clone();
            value["squad"] = json!(squad);
            value["fields"]["squad"] = json!(squad);
            let rank = states.rank(value.value("state"));
            Some(ProjectedRow {
                value,
                numbers: member
                    .map(|member| member.numbers.clone())
                    .unwrap_or_default(),
                rank,
            })
        })
        .collect()
}

impl Acquired {
    /// One optional batched policy read shared by source documents and flat rows.
    pub fn focus(&mut self, core: &Core) {
        let mut values = self
            .documents
            .values_mut()
            .chain(
                self.rows
                    .values_mut()
                    .flat_map(|rows| rows.iter_mut().map(|row| &mut row.value)),
            )
            .collect::<Vec<_>>();
        crate::focus::enrich(core, &mut values);
    }
    /// Display names from the same full roster as member_ids, before row filters.
    pub fn member_names(&self, squad: &str) -> BTreeMap<String, String> {
        self.rows
            .get(squad)
            .into_iter()
            .flatten()
            .filter_map(|row| {
                Some((
                    row.value["id"].as_str()?.to_owned(),
                    row.value["name"].as_str()?.to_owned(),
                ))
            })
            .collect()
    }

    /// Full roster UUIDs retained before sections can repeat or omit rows.
    pub fn member_ids(&self, squad: &str) -> impl Iterator<Item = &str> {
        self.rows
            .get(squad)
            .into_iter()
            .flatten()
            .filter_map(|row| row.value["id"].as_str())
    }

    pub fn attention(&self, config: &Config, order: &[String]) -> BTreeMap<String, Attention> {
        let mut attention = tab_attention(&self.documents);
        if let Ok(settings) = config.tabs() {
            for tab in settings.user {
                attention.insert(
                    tabs::user_key(&tab.name),
                    Attention::of(&user_document(&tab, order, self)),
                );
            }
        }
        attention
    }
    pub fn include(&mut self, config: &Config, squad: &str, document: Value) {
        // Deferred attention only filters public text; no sort metadata is needed.
        let states = config
            .layout(squad)
            .and_then(|layout| config.states(squad, layout));
        if let Ok(states) = states {
            let rows = project_rows(squad, &document, &[], &states);
            self.rows.insert(squad.into(), rows);
        }
        self.documents.insert(squad.into(), document);
    }
}

/// One squad's document from its roster: the same document `status`
/// builds, so a tab and `ls --json` never disagree.
#[cfg(test)]
pub(crate) fn roster_document(
    config: &Config,
    squad: &Squad,
    roster: Vec<crate::squad::Member>,
    waiting: Option<(&str, &requests::Window)>,
) -> Result<Value, crate::core::SquadError> {
    let layout = config.layout(&squad.name)?;
    let states = config.states(&squad.name, layout)?;
    let sections = config.sections(&squad.name)?;
    let rows = config.rows(&squad.name)?;
    let mut document = status::document(squad, layout, &states, &sections, &rows, roster);
    if let Some((me, inbox)) = waiting {
        requests::apply_waiting(&mut document, &squad.name, me, inbox);
    }
    Ok(document)
}

/// The leads tab's document: one row per squad that has a lead, in tab
/// order (hidden squads last), each carrying its squad as `squad` and as
/// the `squad` field.
pub(crate) fn leads_document(tabs: &[String], documents: &BTreeMap<String, Value>) -> Value {
    let rows: Vec<Value> = in_tab_order(tabs, documents)
        .into_iter()
        .filter_map(|name| {
            let mut lead = documents[name]["squad"]["lead"].clone();
            if !lead.is_object() {
                return None;
            }
            lead["squad"] = json!(name);
            lead["fields"]["squad"] = json!(name);
            Some(lead)
        })
        .collect();
    json!({
        "squad": {"name": "leads", "lead": null},
        "sections": status::sections(&rows, &[], |_,_,_| Ordering::Equal, Clone::clone),
    })
}

/// Every squad tab's attention, and the leads tab's from its leads.
pub(crate) fn tab_attention(documents: &BTreeMap<String, Value>) -> BTreeMap<String, Attention> {
    let mut attention: BTreeMap<String, Attention> = documents
        .iter()
        .map(|(name, document)| (name.clone(), Attention::of(document)))
        .collect();
    attention.insert(
        LEADS.to_owned(),
        Attention::of(&leads_document(&[], documents)),
    );
    // Every squad's members, each counted once per squad.
    let all = attention
        .iter()
        .filter(|(key, _)| !tabs::builtin(key))
        .fold(Attention::default(), |sum, (_, one)| Attention {
            waiting: sum.waiting + one.waiting,
            blocked: sum.blocked + one.blocked,
        });
    attention.insert(ALL.to_owned(), all);
    attention
}
