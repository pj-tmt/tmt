//! Terminal and signed reads share projections; no network or mutation is performed.
use super::service::{self, Caller, Machines, Page};
use crate::{admission::MessagePermit, devices::Devices, error::RemoteError, pairing::now_ms};
use serde::Deserialize;
use serde_json::Value;

type Result<T> = std::result::Result<T, RemoteError>;
enum MemberRead {
    Roster(Page),
    Machines { member: String, page: Page },
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MachinePage {
    member_id: String,
    cursor: Option<String>,
    limit: usize,
}
fn parse(operation: &str, input: &Value) -> Result<MemberRead> {
    let keys = match operation {
        "remote.members.list" => vec!["cursor", "limit"],
        "remote.members.machines" => vec!["memberId", "cursor", "limit"],
        _ => return Err(service::invalid()),
    };
    let object = input.as_object().ok_or_else(service::invalid)?;
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err(service::invalid());
    }
    if operation == "remote.members.list" {
        return serde_json::from_value(input.clone())
            .map(MemberRead::Roster)
            .map_err(|_| service::invalid());
    }
    let input: MachinePage =
        serde_json::from_value(input.clone()).map_err(|_| service::invalid())?;
    if input.member_id.is_empty() {
        return Err(service::invalid());
    }
    Ok(MemberRead::Machines {
        member: input.member_id,
        page: Page {
            cursor: input.cursor,
            limit: input.limit,
        },
    })
}
fn activity(view: &mut Machines, observed: Option<crate::session::DeviceActivities>) -> Result<()> {
    if let Some(observed) = observed {
        if observed.len() != view.machines.len() {
            return Err(service::invalid());
        }
        for (row, (count, time)) in view.machines.iter_mut().zip(observed) {
            row.live_session_count = Some(count);
            row.last_activity_at_ms = time;
        }
    }
    Ok(())
}
pub(crate) fn signed(
    permit: &MessagePermit,
    source: &dyn super::observations::MemberObservationSource,
) -> Result<Vec<u8>> {
    let read = parse(permit.message.envelope().operation, &permit.message.input)?;
    let value = match read {
        MemberRead::Roster(page) => {
            let invitation = source.invitation();
            let view = permit.with_store(|store| {
                let mut view =
                    service::roster(store, Caller::Browser(&permit.grant), page, now_ms()?)?;
                let scope =
                    service::scope(&store.connection, Caller::Browser(&permit.grant), now_ms()?)?;
                if view.administrator {
                    view.invitation = Some(invitation);
                }
                view.routes = source.routes(store, scope.member.as_deref());
                Ok(view)
            })?;
            permit.with_store(|store| {
                service::validate_roster(store, Caller::Browser(&permit.grant), &view, now_ms()?)
            })?;
            serde_json::to_value(view).map_err(crate::store::database)?
        }
        MemberRead::Machines { member, page } => {
            let (mut view, grants) = permit.with_store(|store| {
                let view = service::machines(
                    store,
                    Caller::Browser(&permit.grant),
                    &member,
                    page,
                    now_ms()?,
                )?;
                let grants = service::machine_grants(
                    store,
                    Caller::Browser(&permit.grant),
                    &view,
                    now_ms()?,
                )?;
                Ok((view, grants))
            })?;
            // Session locks never nest under the Store guard used above.
            activity(
                &mut view,
                Some(permit.sessions.management_activity(&grants, now_ms()?)?),
            )?;
            permit.with_store(|store| {
                service::machine_grants(store, Caller::Browser(&permit.grant), &view, now_ms()?)
            })?;
            serde_json::to_value(view).map_err(crate::store::database)?
        }
    };
    permit.response(&value)
}
/// Only the existing private local-owner control socket calls this adapter.
pub fn local(
    devices: &Devices,
    source: &dyn super::observations::MemberObservationSource,
    operation: &str,
    input: &Value,
) -> Result<Value> {
    match parse(operation, input)? {
        MemberRead::Roster(page) => {
            let invitation = source.invitation();
            let view = {
                let store = devices.store()?;
                let mut view = service::roster(&store, Caller::LocalOwner, page, now_ms()?)?;
                view.invitation = Some(invitation);
                view.routes = source.routes(&store, None);
                view
            };
            serde_json::to_value(view).map_err(crate::store::database)
        }
        MemberRead::Machines { member, page } => {
            let (mut view, grants) = {
                let store = devices.store()?;
                let view = service::machines(&store, Caller::LocalOwner, &member, page, now_ms()?)?;
                let grants = service::machine_grants(&store, Caller::LocalOwner, &view, now_ms()?)?;
                (view, grants)
            };
            activity(&mut view, devices.member_activity(&grants, now_ms()?)?)?;
            {
                let store = devices.store()?;
                service::machine_grants(&store, Caller::LocalOwner, &view, now_ms()?)?;
            }
            serde_json::to_value(view).map_err(crate::store::database)
        }
    }
}
