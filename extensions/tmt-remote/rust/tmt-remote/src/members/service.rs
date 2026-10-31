//! Shared bounded member reads. Adapters provide current authority, never wire roles.
use super::{Member, Role, store as sql};
use crate::{
    error::RemoteError,
    store::{Grant, Store, database, grant_in, machine_in},
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub(super) type Result<T> = std::result::Result<T, RemoteError>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub cursor: Option<String>,
    pub limit: usize,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub remote_id: String,
    pub member_id: String,
    pub role: Role,
    pub display_name: String,
    pub sending_enabled: bool,
    pub revision: u64,
    pub created_at_ms: u64,
    pub revoked_at_ms: Option<u64>,
}
impl From<Member> for Person {
    fn from(m: Member) -> Self {
        Self {
            remote_id: m.remote_id,
            member_id: m.member_id,
            role: m.role,
            display_name: m.display_name,
            sending_enabled: m.sending_enabled,
            revision: m.revision,
            created_at_ms: m.created_at_ms,
            revoked_at_ms: m.revoked_at_ms,
        }
    }
}
/// Read-only historical use, separate from immutable mutation receipt people.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonObservation {
    #[serde(flatten)]
    pub person: Person,
    pub last_used_at_ms: Option<u64>,
}
/// Durable row facts only; neither timestamp nor count is a presence observation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub remote_id: String,
    pub member_count: Option<u64>,
    pub last_active_at_ms: Option<u64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Roster {
    pub remote_id: String,
    pub administrator: bool,
    pub display_name: Option<String>,
    pub invitation: Option<super::observations::MemberInvitationView>,
    pub routes: super::observations::MemberRouteView,
    pub members: Vec<PersonObservation>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    pub client_id: String,
    pub name: String,
    pub kind: String,
    pub revision: u64,
    pub revoked: bool,
    pub expires_at_ms: Option<u64>,
    pub sending_scope: bool,
    pub delivery: String,
    pub recipient_access: &'static str,
    pub grant_live: bool,
    pub last_used_at_ms: Option<u64>,
    pub this_browser: bool,
    pub live_session_count: Option<usize>,
    pub last_activity_at_ms: Option<u64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Machines {
    pub remote_id: String,
    pub member_id: String,
    pub machines: Vec<Machine>,
    pub next_cursor: Option<String>,
}

/// A local owner is admitted by the private control socket. Signed callers must
/// already pass the exact-door browser Session boundary and revalidate on return.
pub enum Caller<'a> {
    LocalOwner,
    Browser(&'a Grant),
}
pub(crate) struct MemberViewAccess {
    pub remote: String,
    pub caller: String,
    pub member: Option<String>,
}
impl MemberViewAccess {
    pub fn administrator(&self) -> bool {
        self.member.is_none()
    }
    pub fn target(&self, member: &str) -> Result<()> {
        if self.member.as_ref().is_some_and(|own| own != member) {
            return Err(denied());
        }
        Ok(())
    }
}
pub(crate) fn denied() -> RemoteError {
    RemoteError::new(
        "REMOTE_MEMBER_OWNER_REQUIRED",
        "This member operation requires the current administrator or its permitted self-service view.",
    )
}
pub(crate) fn invalid() -> RemoteError {
    RemoteError::new("REMOTE_INPUT_INVALID", "Invalid member management input.")
}
pub(crate) fn scope(c: &Connection, caller: Caller<'_>, now: u64) -> Result<MemberViewAccess> {
    let remote = machine_in(c)?.id;
    let Caller::Browser(grant) = caller else {
        return Ok(MemberViewAccess {
            remote,
            caller: "local-owner".into(),
            member: None,
        });
    };
    let current = grant_in(c, &grant.client_id)?
        .filter(|g| {
            g.live_at(now)
                && g.revision == grant.revision
                && g.public_key == grant.public_key
                && g.kind == "browser"
                && g.origin == grant.origin
        })
        .ok_or_else(denied)?;
    let member = sql::for_client(c, &current.client_id)?
        .filter(|m| m.remote_id == remote && m.revoked_at_ms.is_none())
        .ok_or_else(denied)?;
    let admin = member.role == Role::Owner
        && sql::require_owner(
            c,
            &super::Owner::DesignatedBrowser {
                client_id: &current.client_id,
                revision: current.revision,
            },
            now,
        )
        .is_ok();
    Ok(MemberViewAccess {
        remote,
        caller: current.client_id,
        member: (!admin).then_some(member.member_id),
    })
}

pub fn roster(store: &Store, caller: Caller<'_>, page: Page, now: u64) -> Result<Roster> {
    let scope = scope(&store.connection, caller, now)?;
    let mut view = super::projection::roster(&store.connection, &scope, page)?;
    view.display_name = crate::settings::read_or_default(&store.data_root)
        .remote_name()
        .map(str::to_owned);
    Ok(view)
}
pub(crate) fn summary(store: &Store, caller: Caller<'_>, now: u64) -> Result<Summary> {
    let scope = scope(&store.connection, caller, now)?;
    super::projection::summary(&store.connection, &scope)
}
/// A designation can change while the admitted grant remains live. Facts are disclosed only
/// after rechecking the current administrator boundary, just like the full roster.
pub(crate) fn validate_summary(
    store: &Store,
    caller: Caller<'_>,
    view: &Summary,
    now: u64,
) -> Result<()> {
    let current = scope(&store.connection, caller, now)?;
    if current.remote != view.remote_id
        || (!current.administrator()
            && (view.member_count.is_some() || view.last_active_at_ms.is_some()))
    {
        return Err(denied());
    }
    Ok(())
}
pub fn machines(
    store: &Store,
    caller: Caller<'_>,
    member: &str,
    page: Page,
    now: u64,
) -> Result<Machines> {
    let scope = scope(&store.connection, caller, now)?;
    scope.target(member)?;
    let target = sql::member_in(&store.connection, &scope.remote, member)?.ok_or_else(invalid)?;
    if !scope.administrator() && target.revoked_at_ms.is_some() {
        return Err(denied());
    }
    super::projection::machines(store, &scope, member, page, now)
}

pub(super) fn page_size(page: &Page) -> Result<i64> {
    if !(1..=crate::limits::MANAGEMENT_PAGE).contains(&page.limit) {
        return Err(invalid());
    }
    i64::try_from(page.limit + 1).map_err(database)
}

/// A designation can change without ending the caller Session. Recheck the view's
/// disclosure boundary after observations and immediately before the adapter signs it.
pub(crate) fn validate_roster(
    store: &Store,
    caller: Caller<'_>,
    view: &Roster,
    now: u64,
) -> Result<()> {
    let current = scope(&store.connection, caller, now)?;
    if current.remote != view.remote_id
        || current.administrator() != view.administrator
        || current.member.as_ref().is_some_and(|member| {
            view.members
                .iter()
                .any(|row| &row.person.member_id != member)
        })
    {
        return Err(denied());
    }
    Ok(())
}
pub(crate) fn machine_grants(
    store: &Store,
    caller: Caller<'_>,
    view: &Machines,
    now: u64,
) -> Result<Vec<Grant>> {
    let current = scope(&store.connection, caller, now)?;
    current.target(&view.member_id)?;
    if current.remote != view.remote_id {
        return Err(denied());
    }
    view.machines
        .iter()
        .map(|row| {
            let member = sql::for_client(&store.connection, &row.client_id)?.ok_or_else(denied)?;
            if member.remote_id != current.remote || member.member_id != view.member_id {
                return Err(denied());
            }
            let grant = grant_in(&store.connection, &row.client_id)?.ok_or_else(denied)?;
            if grant.revision != row.revision {
                return Err(denied());
            }
            Ok(grant)
        })
        .collect()
}
