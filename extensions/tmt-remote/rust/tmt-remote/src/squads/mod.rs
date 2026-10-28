//! One Remote-owned list/send/delivery interface; backends never grant agent authority.
mod summary;
pub use summary::{Member, MemberPresence, Squad, SquadObservation, State, Summary};

#[cfg(test)]
mod tests;

use crate::{
    cloud_command, cloud_inbox::DeliveryEvidence, cloud_pairing::service::Service,
    error::RemoteError,
};
use serde::Serialize;

/// Transport identity and selector remain local routing metadata, outside the synchronized summary.
pub struct Record {
    pub machine_id: String,
    pub remote: String,
    pub summary: Vec<u8>,
}
/// Supplies verified retained Firestore summaries; this boundary does not own caching or storage.
pub trait SummaryReader {
    fn read(&self) -> Result<Vec<Record>, RemoteError>;
}
#[derive(Serialize)]
pub struct Machine {
    pub remote: String,
    pub status: MachineSummaryStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<Summary>,
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MachineSummaryStatus {
    Available,
    Unavailable,
    Incompatible,
}
#[derive(Serialize)]
pub struct List {
    pub version: u64,
    pub machines: Vec<Machine>,
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryPhase {
    Queued,
    Delivered,
    Answered,
    Failed,
    Held,
    Unconfirmed,
    Unknown,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub version: u64,
    pub operation_id: String,
    pub state: DeliveryPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
/// One consumer boundary. Neither a list row nor a delivery observation grants permission to send.
pub trait Interface {
    fn list(&self) -> Result<List, RemoteError>;
    /// Preserve the existing prepared-message protocol; its `sent` is not a delivery receipt.
    fn send(
        &self,
        raw: &[u8],
        selected: Option<&str>,
    ) -> Result<cloud_command::Answer, RemoteError>;
    fn delivery(&self, remote: &str, operation_id: &str) -> Result<Delivery, RemoteError>;
}
pub struct Firestore<'a> {
    service: &'a Service,
    summaries: &'a dyn SummaryReader,
}
impl<'a> Firestore<'a> {
    pub fn new(service: &'a Service, summaries: &'a dyn SummaryReader) -> Self {
        Self { service, summaries }
    }
}
impl Interface for Firestore<'_> {
    fn list(&self) -> Result<List, RemoteError> {
        project_records(self.service.machine_id(), self.summaries.read()?)
    }
    fn send(
        &self,
        raw: &[u8],
        selected: Option<&str>,
    ) -> Result<cloud_command::Answer, RemoteError> {
        serde_json::from_value(self.service.send(raw, selected)?).map_err(|_| unavailable())
    }
    fn delivery(&self, remote: &str, operation_id: &str) -> Result<Delivery, RemoteError> {
        Ok(project_delivery(
            operation_id,
            self.service.delivery(remote, operation_id)?,
        ))
    }
}
fn unavailable() -> RemoteError {
    RemoteError::new(
        "REMOTE_SUMMARY_UNAVAILABLE",
        "The machine summary list is unavailable.",
    )
}
fn project_records(local: &str, records: Vec<Record>) -> Result<List, RemoteError> {
    if records.len() > crate::limits::SQUAD_SUMMARY_MACHINES {
        return Err(unavailable());
    }
    let mut ids = std::collections::HashSet::new();
    let mut remotes = std::collections::HashSet::new();
    let mut machines = Vec::new();
    for record in records {
        if !crate::canonical::is_core_id(&record.machine_id)
            || !ids.insert(record.machine_id.clone())
            || record.remote == "local"
            || !crate::remote_catalog::name_ok(&record.remote)
            || !remotes.insert(record.remote.clone())
        {
            return Err(unavailable());
        }
        if record.machine_id != local {
            let (status, summary) = classify_summary(&record.summary);
            machines.push(Machine {
                remote: record.remote,
                status,
                summary,
            });
        }
    }
    Ok(List {
        version: 1,
        machines,
    })
}
fn classify_summary(bytes: &[u8]) -> (MachineSummaryStatus, Option<Summary>) {
    use MachineSummaryStatus::*;
    if bytes.len() > crate::limits::SQUAD_SUMMARY_BYTES {
        return (Unavailable, None);
    }
    let Some(value) = crate::wire::strict_json(bytes) else {
        return (Unavailable, None);
    };
    if value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|version| version != 1)
    {
        return (Incompatible, None);
    }
    match Summary::read(bytes) {
        Ok(summary) => (
            if summary.squads().is_some() {
                Available
            } else {
                Unavailable
            },
            Some(summary),
        ),
        Err(_) => (Unavailable, None),
    }
}
fn project_delivery(operation_id: &str, evidence: DeliveryEvidence) -> Delivery {
    use DeliveryPhase::*;
    let (state, reason) = match evidence {
        DeliveryEvidence::Unknown => (Unknown, None),
        DeliveryEvidence::CarrierStored => (Queued, None),
        DeliveryEvidence::Journal(value) => match value["state"].as_str() {
            Some("held") => (Held, None),
            Some("uncertain") => (Unconfirmed, None),
            Some("refused" | "cancelled") => (
                Failed,
                value["reason"]
                    .as_str()
                    .filter(|s| cloud_command::fixed_code(s))
                    .map(str::to_owned),
            ),
            _ => (Unknown, None),
        },
        DeliveryEvidence::Request(value) => match value["messageState"]["state"].as_str() {
            Some("answered") => (Answered, None),
            Some("delivered" | "working") => (Delivered, None),
            Some("sent") => (Queued, None),
            Some("batched") => (Held, None),
            Some("unconfirmed") => (Unconfirmed, None),
            Some("not_sent") => (
                Failed,
                value["messageState"]["reason"]["code"]
                    .as_str()
                    .filter(|s| cloud_command::fixed_code(s))
                    .map(str::to_owned),
            ),
            _ => (Unknown, None),
        },
    };
    Delivery {
        version: 1,
        operation_id: operation_id.into(),
        state,
        reason,
    }
}
