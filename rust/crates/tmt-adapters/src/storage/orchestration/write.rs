//! One locked snapshot and publication owner. Callers see typed state, never a connection.
use super::*;

impl Storage {
    /// The callback is invoked once after writer admission, against the current
    /// snapshot. It must do no external I/O or transport. A returned error rolls
    /// back; a commit error is not permission to replay the user's mutation.
    pub(crate) fn update_orchestration<T, E>(
        &mut self,
        change: impl FnOnce(&OrchStoredState) -> Result<(OrchSnapshot, T), E>,
    ) -> Result<T, E>
    where
        E: From<StorageError>,
    {
        let tx = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| classify(e, "Begin orchestration configuration write"))?;
        let before = read_snapshot(&tx, None, None)?;
        let (after, result) = change(&before)?;
        if after != before.snapshot {
            if after.revision
                != before
                    .snapshot
                    .revision
                    .checked_add(1)
                    .ok_or_else(corrupt)?
            {
                return Err(corrupt().into());
            }
            publish(&tx, &before.snapshot, &after, before.time_zone.as_deref())?;
        }
        tx.commit()
            .map_err(|e| classify(e, "Commit orchestration configuration write"))?;
        Ok(result)
    }
}

fn execute(c: &Connection, sql: &str, values: impl rusqlite::Params) -> Result<(), StorageError> {
    c.execute(sql, values)
        .map(|_| ())
        .map_err(|e| classify(e, "Write orchestration configuration"))
}
fn encoded(value: &Value) -> Result<String, StorageError> {
    serde_json::to_string(value).map_err(|_| corrupt())
}
fn publish(
    c: &Connection,
    before: &OrchSnapshot,
    after: &OrchSnapshot,
    zone: Option<&str>,
) -> Result<(), StorageError> {
    for old in &before.plans {
        if !after.plans.iter().any(|p| p.id == old.id) {
            execute(c, "DELETE FROM orchestration_plans WHERE id=?", [&old.id])?;
        }
    }
    for plan in &after.plans {
        let old = before.plans.iter().find(|p| p.id == plan.id);
        if old == Some(plan) {
            continue;
        }
        let value = wire::plan_value(plan).map_err(|_| corrupt())?;
        execute(
            c,
            "INSERT INTO orchestration_plans(id,name,revision,priority,enabled,schedule,fields) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET name=excluded.name,revision=excluded.revision,priority=excluded.priority,enabled=excluded.enabled,schedule=excluded.schedule,fields=excluded.fields",
            params![
                plan.id,
                plan.name,
                plan.revision as i64,
                plan.priority,
                plan.enabled,
                encoded(&value["schedule"])?,
                encoded(&value["fields"])?
            ],
        )?;
        if old.is_none_or(|p| p.targets != plan.targets) {
            execute(
                c,
                "DELETE FROM orchestration_plan_targets WHERE plan_id=?",
                [&plan.id],
            )?;
            for (position, target) in value["targets"]
                .as_array()
                .ok_or_else(corrupt)?
                .iter()
                .enumerate()
            {
                execute(
                    c,
                    "INSERT INTO orchestration_plan_targets(plan_id,position,target) VALUES(?1,?2,?3)",
                    params![plan.id, position as i64, encoded(target)?],
                )?;
            }
        }
        if old.is_none_or(|p| p.members != plan.members) {
            execute(
                c,
                "DELETE FROM orchestration_member_overrides WHERE plan_id=?",
                [&plan.id],
            )?;
            for member in value["members"].as_array().ok_or_else(corrupt)? {
                execute(
                    c,
                    "INSERT INTO orchestration_member_overrides(plan_id,identity_id,fields) VALUES(?1,?2,?3)",
                    params![
                        plan.id,
                        member["identityId"].as_str().ok_or_else(corrupt)?,
                        encoded(&member["fields"])?
                    ],
                )?;
            }
        }
    }
    if before.leads != after.leads {
        execute(c, "DELETE FROM orchestration_leads", [])?;
        for id in &after.leads {
            execute(
                c,
                "INSERT INTO orchestration_leads(identity_id) VALUES(?1)",
                [id],
            )?;
        }
    }
    if before.route_grants != after.route_grants {
        execute(c, "DELETE FROM orchestration_route_grants", [])?;
        for grant in &after.route_grants {
            execute(
                c,
                "INSERT INTO orchestration_route_grants(from_id,to_id,until_ms,granted_by) VALUES(?1,?2,?3,?4)",
                params![
                    grant.from_id,
                    grant.to_id,
                    grant.until_ms as i64,
                    grant.granted_by
                ],
            )?;
        }
    }
    for old in &before.focus {
        if !after.focus.iter().any(|f| f.identity_id == old.identity_id) {
            execute(
                c,
                "DELETE FROM orchestration_focus WHERE identity_id=?",
                [&old.identity_id],
            )?;
        }
    }
    for focus in &after.focus {
        if before.focus.iter().any(|f| f == focus) {
            continue;
        }
        execute(
            c,
            "INSERT INTO orchestration_focus(identity_id,revision,start_ms,end_ms) VALUES(?1,?2,?3,?4) ON CONFLICT(identity_id) DO UPDATE SET revision=excluded.revision,start_ms=excluded.start_ms,end_ms=excluded.end_ms",
            params![
                focus.identity_id,
                focus.revision as i64,
                focus.start_ms as i64,
                focus.end_ms as i64
            ],
        )?;
    }
    execute(
        c,
        "INSERT INTO orchestration_settings(id,revision,time_zone) VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision",
        params![after.revision as i64, zone],
    )
}
