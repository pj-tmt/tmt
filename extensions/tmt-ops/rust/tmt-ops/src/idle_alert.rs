//! The squad idle alert: one announcement to the lead for each member idle too long.
//!
//! Detection is plain code over what Core already publishes. A member is over when Core's
//! admitted `session.activity` has been `idle` for the squad's `idle_alert.after`; the lead
//! hears about each idle episode once. Nothing is stored: the episode is Core's own idle start,
//! and the dispatch operation ID derived from it makes a repeat a no-op in Core.
//! The cron clock runs the scan (`Watch::advance`); `ls --json` marks the same members.

use crate::{
    board::activity::{ObservedActivity, Phase},
    config::{Config, IdleAlert},
    core::{Core, SquadError},
    reminder::one_line,
    squad::{Member, Squad},
};
use jiff::tz::TimeZone;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::Duration,
};

/// Scans are at least this far apart.
const SCAN_EVERY_MS: i64 = 60_000;
/// A longer silence than this (a sleeping laptop) makes the next look only a baseline.
const WAKE_GAP_MS: i64 = 300_000;
/// Each row value is cut at this many characters.
const VALUE_CHARS: usize = 120;
/// The row line (state, task, pending) fits this many bytes.
const ROW_BYTES: usize = 600;

/// Why an idle member is not announced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exception {
    /// The row's `pending` is set: the member waits on the owner.
    Pending,
    /// The row's `state` is `hold`: a resting seat.
    Hold,
}

/// A member idle past the threshold, from Core's idle start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Over {
    pub since_ms: u64,
    pub excepted: Option<Exception>,
}

/// Over means Core's `idle` phase for at least `after`. Working, ended, unknown and unseen rows
/// never count, and idle is never guessed from silence.
pub fn assess(member: &Member, setting: IdleAlert, now_ms: u64) -> Option<Over> {
    if !setting.enabled {
        return None;
    }
    let activity = ObservedActivity::of(&member.seen);
    let since_ms = activity
        .since_ms
        .filter(|_| activity.phase == Phase::Idle)?;
    let idle_for = now_ms.checked_sub(since_ms)?;
    if idle_for < u64::try_from(setting.after.as_millis()).unwrap_or(u64::MAX) {
        return None;
    }
    let field = |name: &str| member.fields.get(name).map(String::as_str);
    let excepted = if field("pending").is_some_and(|pending| !pending.trim().is_empty()) {
        Some(Exception::Pending)
    } else if field("state").is_some_and(|state| state.eq_ignore_ascii_case("hold")) {
        Some(Exception::Hold)
    } else {
        None
    };
    Some(Over { since_ms, excepted })
}

/// Every over member by identity, for the `ls` marks.
pub fn overs(members: &[Member], setting: IdleAlert, now_ms: u64) -> HashMap<String, Over> {
    members
        .iter()
        .filter_map(|member| Some((member.id.clone(), assess(member, setting, now_ms)?)))
        .collect()
}

/// `squad.idleAlert` on the squad, and `idleAlert` on each over row (the lead's included).
/// Absent, never null, when there is nothing to say about a member.
pub fn annotate(document: &mut Value, setting: IdleAlert, overs: &HashMap<String, Over>) {
    document["squad"]["idleAlert"] = json!({
        "enabled": setting.enabled,
        "afterMs": u64::try_from(setting.after.as_millis()).unwrap_or(u64::MAX),
    });
    if let Some(lead) = document["squad"].get_mut("lead") {
        mark(lead, overs);
    }
    for section in document["sections"].as_array_mut().into_iter().flatten() {
        for row in section["rows"].as_array_mut().into_iter().flatten() {
            mark(row, overs);
        }
    }
}

fn mark(row: &mut Value, overs: &HashMap<String, Over>) {
    let Some(over) = row["id"].as_str().and_then(|id| overs.get(id)) else {
        return;
    };
    row["idleAlert"] = match over.excepted {
        None => json!({"state": "over", "sinceMs": over.since_ms}),
        Some(reason) => json!({
            "state": "excepted",
            "reason": match reason { Exception::Pending => "pending", Exception::Hold => "hold" },
            "sinceMs": over.since_ms,
        }),
    };
}

/// One announcement the lead is owed.
#[derive(Debug, PartialEq, Eq)]
pub struct Announcement {
    pub operation_id: String,
    pub recipient_id: String,
    pub body: String,
}

/// The operation names the episode, not the scan: the room, the member, the lead and Core's idle
/// start. A new idle phase has a new start; a new lead is told once more.
pub fn operation_id(room_id: &str, member_id: &str, lead_id: &str, since_ms: u64) -> String {
    let mut hash = Sha256::new();
    hash.update(b"tmt-ops-idle-alert-v1\0");
    for value in [room_id, member_id, lead_id] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    hash.update(since_ms.to_be_bytes());
    tmt_ops::operation::uuid_v8(&hash.finalize())
}

/// The notices due now. Nothing is owed without exactly one lead, to the lead itself, for an
/// excepted member or while the alert is off.
pub fn notices(
    squad: &Squad,
    members: &[Member],
    setting: IdleAlert,
    now_ms: u64,
    zone: &TimeZone,
) -> Vec<Announcement> {
    let mut leads = members.iter().filter(|member| member.is_lead());
    let (Some(lead), None) = (leads.next(), leads.next()) else {
        return Vec::new();
    };
    members
        .iter()
        .filter(|member| !member.is_lead())
        .filter_map(|member| {
            let over = assess(member, setting, now_ms).filter(|over| over.excepted.is_none())?;
            Some(Announcement {
                operation_id: operation_id(&squad.room_id, &member.id, &lead.id, over.since_ms),
                recipient_id: lead.id.clone(),
                body: body(squad, member, over.since_ms, setting.after, zone),
            })
        })
        .collect()
}

/// A function of the episode and, for the row line, of the first attempt. It never holds the scan
/// time or an elapsed duration, so a retry builds the same text unless the row itself changed.
fn body(squad: &Squad, member: &Member, since_ms: u64, after: Duration, zone: &TimeZone) -> String {
    // Identity names are at most 64 characters and are the commands' operand: never cut shorter.
    let name = match one_line(&member.name, 64) {
        name if name.is_empty() => "unnamed member".to_owned(),
        name => name,
    };
    let since = i64::try_from(since_ms)
        .ok()
        .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
        .map_or_else(
            || format!("{since_ms} ms"),
            |time| {
                time.to_zoned(zone.clone())
                    .strftime("%Y-%m-%d %H:%M %:z")
                    .to_string()
            },
        );
    let value = |name: &str| {
        one_line(
            member.fields.get(name).map_or("", String::as_str),
            VALUE_CHARS,
        )
    };
    let mut values = [value("state"), value("task"), value("pending")];
    let row = loop {
        let row = format!(
            "state={} · task={} · pending={}",
            state_text(&values[0]),
            quoted(&values[1]),
            quoted(&values[2])
        );
        if row.len() <= ROW_BYTES {
            break row;
        }
        // Row text is the only part that can be long; trim the longest value first.
        match values.iter_mut().max_by_key(|value| value.len()) {
            Some(longest) if !longest.is_empty() => {
                longest.pop();
            }
            _ => break row,
        }
    };
    format!(
        "Idle alert · squad {squad}\n\
         {name} has been idle since {since} (alert after {after}).\n\
         Row (when sent): {row}\n\
         Look: tmt focus {name} · Talk: tmt talk {name} \"<message>\" --detach",
        squad = squad.name,
        after = threshold(after),
    )
}

fn threshold(after: Duration) -> String {
    match after.as_secs() {
        seconds if seconds % 3600 == 0 => format!("{}h", seconds / 3600),
        seconds if seconds % 60 == 0 => format!("{}m", seconds / 60),
        seconds => format!("{seconds}s"),
    }
}

/// A state word stays bare; any other text is quoted data.
fn state_text(state: &str) -> String {
    let word = !state.is_empty()
        && state
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if word {
        state.to_owned()
    } else {
        quoted(state)
    }
}

fn quoted(text: &str) -> String {
    if text.is_empty() {
        "none".to_owned()
    } else {
        Value::from(text).to_string()
    }
}

/// Decides when a scan runs and remembers which episodes this clock already got accepted.
/// The memory is only an optimisation: a new clock asks once more and Core's operation ID
/// makes that a no-op.
#[derive(Debug, Default)]
pub struct Watch {
    last_ms: Option<i64>,
    accepted: HashSet<String>,
}

impl Watch {
    pub fn new() -> Self {
        Self::default()
    }

    /// True at most once a minute. The first look, a look after a gap of over five minutes and
    /// a look after the clock moved back only record the time: members are judged a minute
    /// later, never at the moment of a wake.
    pub fn due(&mut self, now_ms: i64) -> bool {
        let scan = match self.last_ms {
            Some(last) if (0..=WAKE_GAP_MS).contains(&(now_ms - last)) => {
                if now_ms - last < SCAN_EVERY_MS {
                    return false;
                }
                true
            }
            _ => false,
        };
        self.last_ms = Some(now_ms);
        scan
    }

    /// One duty of the clock tick. Reads the settings fresh (they change while a clock runs),
    /// then one roster and one `ls` per enabled squad, and announces what is owed. `keep` renews
    /// the clock's lease and says false when it was lost or a stop was requested.
    pub fn advance(
        &mut self,
        core: &Core,
        config: &Path,
        now_ms: i64,
        zone: &TimeZone,
        keep: &mut dyn FnMut() -> bool,
    ) -> Vec<SquadError> {
        if !self.due(now_ms) {
            return Vec::new();
        }
        let config = match Config::read(config.to_path_buf()) {
            Ok(config) => config,
            Err(error) => return vec![error],
        };
        let enabled = config.idle_alert_squads();
        if enabled.is_empty() {
            self.accepted.clear();
            return Vec::new();
        }
        let squads = match Squad::list(core) {
            Ok(squads) => squads,
            Err(error) => return vec![error],
        };
        let now = u64::try_from(now_ms).unwrap_or_default();
        let mut warnings = Vec::new();
        let mut still_over = HashSet::new();
        let mut complete = true;
        for squad in squads.iter().filter(|squad| enabled.contains(&squad.name)) {
            if !keep() {
                complete = false;
                break;
            }
            let setting = match config.idle_alert(&squad.name) {
                Ok(setting) if setting.enabled => setting,
                Ok(_) => continue,
                Err(error) => {
                    complete = false;
                    warnings.push(error);
                    continue;
                }
            };
            let members = match squad.members(core, false) {
                Ok(members) => members,
                Err(error) => {
                    complete = false;
                    warnings.push(error);
                    continue;
                }
            };
            for notice in notices(squad, &members, setting, now, zone) {
                if self.accepted.contains(&notice.operation_id) {
                    still_over.insert(notice.operation_id);
                    continue;
                }
                if !keep() {
                    complete = false;
                    break;
                }
                match send(core, squad, &notice) {
                    Ok(()) => {
                        still_over.insert(notice.operation_id);
                    }
                    Err(error) => {
                        complete = false;
                        warnings.push(error);
                    }
                }
            }
        }
        if complete {
            self.accepted = still_over;
        } else {
            self.accepted.extend(still_over);
        }
        warnings
    }
}

/// Core accepts an operation once. A refusal as a changed intent means an earlier attempt of this
/// episode (its row line differed) is already accepted, so it counts as done. When the outcome is
/// uncertain, `dispatch.show` says whether the operation exists; nothing is created twice.
fn send(core: &Core, squad: &Squad, notice: &Announcement) -> Result<(), SquadError> {
    let input = json!({
        "operationId": notice.operation_id,
        "recipientIds": [notice.recipient_id],
        "message": notice.body,
        "kind": "announcement",
        "room": {"kind": "direct", "roomId": squad.room_id},
    });
    let receipt = match core.dispatch_automated(input, None) {
        Ok(receipt) => receipt,
        Err(error) if error.code == "DISPATCH_IDEMPOTENCY_CONFLICT" => return Ok(()),
        Err(error)
            if matches!(
                error.code.as_str(),
                "SQUAD_CORE_UNAVAILABLE" | "STORAGE_UNAVAILABLE"
            ) =>
        {
            core.api("dispatch.show", json!({"operationId": notice.operation_id}))
                .map_err(|_| {
                    SquadError::new(
                        &error.code,
                        format!(
                            "{} Operation {}; recover acceptance with dispatch.show before retrying.",
                            error.message, notice.operation_id
                        ),
                    )
                })?
        }
        Err(error) => {
            return Err(SquadError::new(
                &error.code,
                format!("{} Operation {}.", error.message, notice.operation_id),
            ));
        }
    };
    let queued = receipt["operationId"] == notice.operation_id
        && receipt["items"].as_array().is_some_and(|items| {
            items.len() == 1
                && items[0]["recipientId"] == notice.recipient_id
                && items[0]["acceptance"] == "queued"
                && items[0]["requestId"].as_str().is_some()
        });
    if queued {
        Ok(())
    } else {
        Err(SquadError::new(
            "SQUAD_IDLE_ALERT_NOT_QUEUED",
            format!(
                "Operation {} was not accepted for the lead; inspect its receipt before retrying.",
                notice.operation_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests;
