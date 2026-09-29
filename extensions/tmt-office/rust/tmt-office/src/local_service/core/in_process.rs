//! Test-only stand-in for the invoking `tmt`: the same `tmt api` code path and
//! the two JSON commands the service uses. It never observes panes: the
//! fixtures bind none, and bound presence is `tmt list`'s own contract, tested
//! in core.

use super::{CoreFault, LocalCore};
use serde_json::{Value, json};
use tmt_adapters::{api, config::ConfigPaths, room::encode_rooms, storage::Storage};
use tmt_core::{identity::IdentityReader, room::RoomRepository};
use tmt_office_storage::core_client::WriteOriginator;

pub(super) struct InProcessCore {
    pub paths: ConfigPaths,
}

impl LocalCore for InProcessCore {
    fn api(
        &self,
        operation: &str,
        input: Value,
        originator: Option<WriteOriginator<'_>>,
    ) -> Result<Value, CoreFault> {
        let mut envelope = json!({"version": 1, "operation": operation, "input": input});
        match originator {
            Some(WriteOriginator::Identity(identity)) => envelope["identity"] = json!(identity),
            Some(WriteOriginator::Anonymous) => envelope["originator"] = json!("anonymous"),
            None => {}
        }
        let outcome = api::decode(&envelope.to_string())
            .and_then(|request| api::execute(&self.paths, request));
        match outcome {
            Ok(body) => serde_json::from_slice(&body).map_err(|_| CoreFault::unavailable()),
            Err(error) => {
                let document: Value = serde_json::from_slice(&error.encode())
                    .map_err(|_| CoreFault::unavailable())?;
                Err(document["error"]["code"]
                    .as_str()
                    .map_or_else(CoreFault::unavailable, |code| CoreFault {
                        code: code.into(),
                    }))
            }
        }
    }

    fn command(&self, args: &[&str]) -> Result<Value, CoreFault> {
        let mut storage =
            Storage::open(&self.paths.database).map_err(|_| CoreFault::unavailable())?;
        let result = match args {
            // An unbound identity is offline in `tmt list --json`.
            ["list"] => storage
                .list_identities()
                .map(|identities| {
                    json!({"identities": identities.into_iter().map(|identity| json!({
                        "id": identity.id,
                        "lifetime": identity.lifetime.as_str(),
                        "presence": "offline",
                    })).collect::<Vec<_>>()})
                })
                .map_err(|_| CoreFault::unavailable()),
            ["room", "list"] => storage
                .list_meeting_rooms()
                .map(|rooms| json!({"rooms": serde_json::from_slice::<Value>(&encode_rooms(&rooms)).expect("rooms")}))
                .map_err(|_| CoreFault::unavailable()),
            _ => Err(CoreFault::unavailable()),
        };
        storage.close().map_err(|_| CoreFault::unavailable())?;
        result
    }
}
