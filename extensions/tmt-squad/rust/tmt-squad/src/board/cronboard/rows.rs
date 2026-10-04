//! Job rows: the projection of one cron read into list rows, and the row markup
//! shared by the squad tab's jobs half and the `c` list. Pure.

use super::line::{first_line, next_time, time, zone};
use crate::cron_service::{JobKey, JobView, schedule_text};
use serde_json::{Value, json};
use tmt_cli_style::Role;

/// Which columns a surface has room for; the rest step aside by priority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Columns {
    pub squad: bool,
    pub what: bool,
}

impl Columns {
    /// The message preview is the first column to go, below 100 cells.
    pub fn for_width(squad: bool, width: u16) -> Self {
        Self {
            squad,
            what: width >= 100,
        }
    }

    /// The `<tmt-row>` markup for these columns, bound to `row`.
    pub fn markup(self) -> String {
        let squad = if self.squad {
            r#"<tmt-text id="squad" bind="row.squad" token="muted" class="w-12 shrink-0 truncate"/>"#
        } else {
            ""
        };
        let what = if self.what {
            r#"<tmt-text id="what" bind="row.what" token="text" class="grow min-w-0 truncate"/>"#
        } else {
            ""
        };
        format!(
            r#"<tmt-row class="flex-col"><tmt-row class="flex-row gap-1 shrink-0"><tmt-text id="mark" bind="row.mark" token-bind="row.mark_role" class="w-1 shrink-0"/><tmt-text id="id" bind="row.cid" token="link" class="w-4 shrink-0"/>{squad}<tmt-text id="owner" bind="row.owner" token-bind="row.owner_role" class="w-16 shrink-0 truncate"/>{what}<tmt-text id="schedule" bind="row.schedule" token="dim" class="w-22 shrink-0 truncate"/><tmt-text id="next" bind="row.next" token-bind="row.next_role" class="w-15 shrink-0 truncate"/></tmt-row><tmt-repeat each="row.detail" as="line"><tmt-row id-bind="line.id" class="flex-row gap-1 shrink-0"><tmt-text bind="line.label" token="dim" class="w-8 shrink-0"/><tmt-text bind="line.text" token="text" wrap="true" class="grow min-w-0"/></tmt-row></tmt-repeat></tmt-row>"#
        )
    }
}

/// `<room uuid>/<c-id>`: stable across refresh, reorder and squad renames.
pub(super) fn row_id(key: &JobKey) -> String {
    format!("{}/{}", key.room_id, key.id)
}

pub(super) fn key_of(view: &JobView) -> JobKey {
    JobKey::of(&view.job)
}

/// The schema fields the row markup binds, for `picker_surface::schema`.
pub(super) fn schema_rows() -> tmt_tui::binding::Schema {
    use tmt_tui::binding::Schema;
    let scalar = |fields: &[&str]| {
        Schema::Collection(Box::new(Schema::Object(
            fields
                .iter()
                .map(|name| {
                    (
                        (*name).to_owned(),
                        if *name == "id" {
                            Schema::StableId
                        } else {
                            Schema::Scalar
                        },
                    )
                })
                .collect(),
        )))
    };
    let mut fields = vec![
        ("id", Schema::StableId),
        ("disabled", Schema::Boolean),
        ("detail", scalar(&["id", "label", "text"])),
    ];
    let names = [
        "mark",
        "mark_role",
        "cid",
        "squad",
        "owner",
        "owner_role",
        "what",
        "schedule",
        "next",
        "next_role",
    ];
    fields.extend(names.map(|name| (name, Schema::Scalar)));
    Schema::Collection(Box::new(Schema::Object(
        fields
            .into_iter()
            .map(|(name, schema)| (name.to_owned(), schema))
            .collect(),
    )))
}

fn mark(view: &JobView) -> (&'static str, Role) {
    match view.job.state() {
        "on" => ("●", Role::Working),
        "paused" => ("○", Role::Dim),
        _ => ("✗", Role::Blocked),
    }
}

/// Up to three future slots; `Schedule` owns the math, zones and DST rules.
fn upcoming(view: &JobView, now_ms: i64) -> Vec<String> {
    let zone = zone(view);
    let mut after = now_ms;
    let mut slots = Vec::new();
    while slots.len() < 3 {
        let Ok(Some(slot)) = view.job.schedule.next_after(after) else {
            break;
        };
        slots.extend(time(slot, now_ms, &zone));
        after = slot;
    }
    slots
}

fn detail(view: &JobView, id: &str, now_ms: i64) -> Vec<Value> {
    // The message is sanitized for display; stored bytes never change here.
    let message = crate::board::notes::sanitize(&view.job.message).replace('\n', " ↵ ");
    let when = match view.job.state() {
        "on" => format!("{}   tz {}", upcoming(view, now_ms).join(" · "), zone(view)),
        "paused" => format!("paused · p to resume   tz {}", zone(view)),
        _ => format!("no owner · o to reassign   tz {}", zone(view)),
    };
    vec![
        json!({"id": format!("{id}#message"), "label": "message", "text": message}),
        json!({"id": format!("{id}#next"), "label": "next", "text": when}),
    ]
}

/// One row per job; `expanded` names the job shown in place.
pub(super) fn project(jobs: &[&JobView], expanded: Option<&str>, now_ms: i64) -> Vec<Value> {
    jobs.iter()
        .map(|view| {
            let id = row_id(&key_of(view));
            let (symbol, mark_role) = mark(view);
            let state = view.job.state();
            let (next, next_role) = match state {
                "on" => (
                    next_time(view, now_ms).unwrap_or_else(|| "–".into()),
                    Role::Text,
                ),
                "paused" => ("paused".into(), Role::Dim),
                _ => ("no owner".into(), Role::Blocked),
            };
            let owner = view.owner_name.as_deref().unwrap_or("–");
            json!({
                "id": id,
                "disabled": false,
                "mark": symbol,
                "mark_role": mark_role.name(),
                "cid": view.job.id(),
                "squad": view.job.squad,
                "owner": first_line(owner),
                "owner_role": if state == "no owner" { Role::Blocked } else { Role::Text }.name(),
                "what": first_line(&view.job.message),
                "schedule": schedule_text(&view.job.schedule),
                "next": next,
                "next_role": next_role.name(),
                "detail": if expanded == Some(id.as_str()) { detail(view, &id, now_ms) } else { vec![] },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
