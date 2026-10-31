//! Cache-only directory and one original-ID observation over the existing owner socket.
use super::*;
use crate::squads::{
    Delivery, DeliveryPhase, Firestore, Interface, List, Machine, MachineSummaryStatus, Summary,
};

pub fn squad_list(directory: &Path) -> Result<Value, RemoteError> {
    let answer = request(directory, &json!({"op":"squads.list"}))?.ok_or_else(not_running)?;
    let invalid = crate::operations::invalid;
    let row = answer.as_object().ok_or_else(invalid)?;
    let rows = row
        .get("machines")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if row.len() != 2
        || row.get("version") != Some(&json!(1))
        || rows.len() > crate::limits::SQUAD_SUMMARY_MACHINES
    {
        return Err(invalid());
    }
    let mut names = std::collections::HashSet::new();
    let mut machines = Vec::new();
    for value in rows {
        let fields = value.as_object().ok_or_else(invalid)?;
        let remote = value["remote"]
            .as_str()
            .filter(|name| *name != "local" && crate::remote_catalog::name_ok(name))
            .ok_or_else(invalid)?;
        if !names.insert(remote) {
            return Err(invalid());
        }
        let (status, summary) = match value["status"].as_str() {
            Some("available") if fields.len() == 3 && fields.contains_key("summary") => {
                let summary = Summary::read(
                    &serde_json::to_vec(&value["summary"]).map_err(crate::store::database)?,
                )?;
                (MachineSummaryStatus::Available, Some(summary))
            }
            Some("unavailable") if fields.len() == 2 => (MachineSummaryStatus::Unavailable, None),
            Some("incompatible") if fields.len() == 2 => (MachineSummaryStatus::Incompatible, None),
            _ => return Err(invalid()),
        };
        machines.push(Machine {
            remote: remote.into(),
            status,
            summary,
        });
    }
    serde_json::to_value(List {
        version: 1,
        machines,
    })
    .map_err(crate::store::database)
}
pub fn squad_delivery(
    directory: &Path,
    remote: &str,
    operation: &str,
) -> Result<Value, RemoteError> {
    if remote == "local"
        || !crate::remote_catalog::name_ok(remote)
        || !canonical::is_core_id(operation)
    {
        return Err(crate::operations::invalid());
    }
    let answer = request_with_deadline(
        directory,
        &json!({"op":"squads.delivery","remote":remote,"operationId":operation}),
        Some(Instant::now() + crate::limits::CLOUD_MESSAGE_CONTROL_WAIT),
    )?
    .ok_or_else(not_running)?;
    let fields = answer.as_object().ok_or_else(crate::operations::invalid)?;
    if fields.len() != 3 + usize::from(fields.contains_key("reason"))
        || answer["version"] != 1
        || answer["operationId"] != operation
        || answer.get("reason").is_some_and(|r| {
            r.as_str()
                .is_none_or(|r| !crate::cloud_command::fixed_code(r))
        })
    {
        return Err(crate::operations::invalid());
    }
    let state = match answer["state"].as_str() {
        Some("queued") => DeliveryPhase::Queued,
        Some("delivered") => DeliveryPhase::Delivered,
        Some("answered") => DeliveryPhase::Answered,
        Some("failed") => DeliveryPhase::Failed,
        Some("held") => DeliveryPhase::Held,
        Some("unconfirmed") => DeliveryPhase::Unconfirmed,
        Some("unknown") => DeliveryPhase::Unknown,
        _ => return Err(crate::operations::invalid()),
    };
    serde_json::to_value(Delivery {
        version: 1,
        operation_id: operation.into(),
        state,
        reason: answer["reason"].as_str().map(str::to_owned),
    })
    .map_err(crate::store::database)
}
pub(super) fn serve(
    frame: &Value,
    cloud: Option<&crate::cloud_pairing::service::Service>,
) -> Value {
    let result = (|| {
        let row = frame.as_object().ok_or_else(crate::operations::invalid)?;
        let cloud = cloud.ok_or_else(crate::cloud_command::upgrade)?;
        let interface = Firestore::new(cloud, cloud);
        match frame["op"].as_str() {
            Some("squads.list") if row.len() == 1 => {
                serde_json::to_value(interface.list()?).map_err(crate::store::database)
            }
            Some("squads.delivery") if row.len() == 3 => {
                let remote = frame["remote"]
                    .as_str()
                    .filter(|r| *r != "local" && crate::remote_catalog::name_ok(r))
                    .ok_or_else(crate::operations::invalid)?;
                let operation = frame["operationId"]
                    .as_str()
                    .filter(|id| canonical::is_core_id(id))
                    .ok_or_else(crate::operations::invalid)?;
                serde_json::to_value(interface.delivery(remote, operation)?)
                    .map_err(crate::store::database)
            }
            _ => Err(crate::operations::invalid()),
        }
    })();
    result.unwrap_or_else(|error| error.json_value())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aggregate_reply_bound_is_not_an_ordinary_frame_or_a_message_request_allowance() {
        let frame = json!({"version":1,"machines":[],"padding":"x".repeat(LINE_BYTES)});
        let mut bytes = serde_json::to_vec(&frame).unwrap();
        bytes.push(b'\n');
        for policy in [
            LinePolicy::Ordinary,
            LinePolicy::MessageRequest,
            LinePolicy::SquadDirectory,
        ] {
            let (mut stream, _peer) = UnixStream::pair().unwrap();
            let accepted = poll_line_policy(&mut stream, &mut bytes.clone(), policy).is_ok();
            assert_eq!(accepted, matches!(policy, LinePolicy::SquadDirectory));
        }
        let mut over = vec![b'x'; crate::limits::SQUAD_DIRECTORY_CONTROL_BYTES + 1];
        let (mut stream, _peer) = UnixStream::pair().unwrap();
        assert!(poll_line_policy(&mut stream, &mut over, LinePolicy::SquadDirectory).is_err());
    }
}
