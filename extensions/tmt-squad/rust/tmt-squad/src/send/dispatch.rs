//! Frozen dispatch intent and acceptance recovery shared by board send effects.
use super::{LeadRecipient, new_operation, refused};
use crate::{
    config::Config,
    core::{Core, SquadError},
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, io::Write, os::unix::fs::OpenOptionsExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Intent {
    pub operation: String,
    pub sender: String,
    pub recipients: Vec<LeadRecipient>,
    pub input: Value,
    /// Uncertain acceptance is recovered before any explicit retry.
    pub recover: bool,
    pub accepted: bool,
    pub queued: bool,
}
impl Intent {
    pub fn new(
        sender: &str,
        recipients: Vec<LeadRecipient>,
        kind: &str,
        room: Option<&str>,
        text: &str,
    ) -> Result<Self, SquadError> {
        let operation = new_operation()?;
        let ids: BTreeSet<&str> = recipients
            .iter()
            .map(|recipient| recipient.id.as_str())
            .collect();
        let mut input =
            json!({"operationId":operation,"recipientIds":ids,"message":text,"kind":kind});
        if let Some(room) = room {
            input["room"] = json!({"kind":"direct", "roomId":room});
        }
        Ok(Self {
            operation,
            sender: sender.into(),
            recipients,
            input,
            recover: false,
            accepted: false,
            queued: false,
        })
    }

    fn journal(&self, config: &Config) -> Result<std::path::PathBuf, SquadError> {
        let directory = config
            .path()
            .parent()
            .ok_or_else(|| refused("No Squad state directory."))?
            .join("board-dispatches");
        fs::create_dir_all(&directory).map_err(io_error)?;
        let path = directory.join(format!("{}.json", self.operation));
        let document = json!({"version":1,"senderId":self.sender,"recipients":self.recipients.iter().map(|recipient| json!({"squad":recipient.squad,"id":recipient.id,"name":recipient.name})).collect::<Vec<_>>(),"input":self.input});
        let bytes = serde_json::to_vec(&document).expect("serializable dispatch intent");
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(&bytes).map_err(io_error)?;
                file.sync_all().map_err(io_error)?;
                fs::File::open(&directory)
                    .and_then(|directory| directory.sync_all())
                    .map_err(io_error)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if fs::read(&path).map_err(io_error)? != bytes {
                    return Err(refused("Saved dispatch intent changed; refuse retry."));
                }
            }
            Err(error) => return Err(io_error(error)),
        }
        Ok(path)
    }

    pub fn attempt(&mut self, core: &Core, config: &Config) -> Result<String, SquadError> {
        if self.accepted {
            return Ok("Notification already accepted; nothing sent again.".into());
        }
        let path = self.journal(config)?;
        let receipt = if self.recover {
            match core.api("dispatch.show", json!({"operationId":self.operation})) {
                Ok(receipt) => receipt,
                // The owned invoking child has returned and its process group is
                // closed by Core's runner. A fresh submit may retry the same intent.
                Err(error) if error.code == "DISPATCH_NOT_FOUND" => {
                    self.recover = false;
                    return self.attempt(core, config);
                }
                Err(error) => return Err(self.uncertain(error, &path)),
            }
        } else {
            match core.api_write("dispatch.create", self.input.clone(), Some(&self.sender)) {
                Ok(receipt) => receipt,
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "SQUAD_CORE_UNAVAILABLE" | "STORAGE_UNAVAILABLE" | "API_UNAVAILABLE"
                    ) =>
                {
                    self.recover = true;
                    core.api("dispatch.show", json!({"operationId":self.operation}))
                        .map_err(|_| self.uncertain(error, &path))?
                }
                Err(error) => {
                    fs::remove_file(&path).map_err(io_error)?;
                    return Err(error);
                }
            }
        };
        let outcome = acceptance(&self.operation, &self.recipients, &receipt).map_err(|error| {
            self.recover = true;
            SquadError::new(
                &error.code,
                format!("{} Saved intent: {}", error.message, path.display()),
            )
        })?;
        self.accepted = true;
        self.queued = receipt["items"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item["acceptance"] == "queued"));
        if let Err(error) = fs::remove_file(&path) {
            return Ok(format!(
                "{outcome} Accepted operation {}; could not remove saved intent: {error}",
                self.operation
            ));
        }
        Ok(outcome)
    }
    fn uncertain(&self, error: SquadError, path: &std::path::Path) -> SquadError {
        SquadError::new(
            &error.code,
            format!(
                "Acceptance uncertain for operation {}; inspect dispatch.show before sending again. Saved intent: {}",
                self.operation,
                path.display()
            ),
        )
    }
}

pub(super) fn send(
    core: &Core,
    config: &Config,
    sender: &str,
    recipients: &[LeadRecipient],
    kind: &str,
    room: Option<&str>,
    text: &str,
) -> Result<String, SquadError> {
    Intent::new(sender, recipients.to_vec(), kind, room, text)?.attempt(core, config)
}

fn io_error(error: std::io::Error) -> SquadError {
    SquadError::new("SQUAD_DISPATCH_IO", error.to_string())
}

pub(super) fn acceptance(
    operation: &str,
    recipients: &[LeadRecipient],
    receipt: &Value,
) -> Result<String, SquadError> {
    let ids: BTreeSet<&str> = recipients
        .iter()
        .map(|recipient| recipient.id.as_str())
        .collect();
    let items = receipt["items"].as_array().filter(|items| receipt["operationId"] == operation && items.len() == ids.len())
        .ok_or_else(|| refused(format!("Acceptance unavailable for operation {operation}; inspect dispatch.show before sending again.")))?;
    let mut seen = BTreeSet::new();
    let mut outcomes = Vec::new();
    for item in items {
        let id = item["recipientId"].as_str().filter(|id| ids.contains(id) && seen.insert(*id))
            .ok_or_else(|| refused(format!("Acceptance does not match operation {operation}; inspect dispatch.show before sending again.")))?;
        let accepted = match item["acceptance"].as_str() {
            Some("queued") if item["requestId"].as_str().is_some() => "queued",
            Some("recipientUnavailable") => "unavailable",
            _ => {
                return Err(refused(format!(
                    "Acceptance unavailable for operation {operation}; inspect dispatch.show before sending again."
                )));
            }
        };
        let names: BTreeSet<_> = recipients
            .iter()
            .filter(|recipient| recipient.id == id)
            .map(|recipient| tmt_cli_style::table::escape(&recipient.name))
            .collect();
        outcomes.push(format!(
            "{}: {accepted}",
            names.into_iter().collect::<Vec<_>>().join("/")
        ));
    }
    Ok(outcomes.join("; "))
}
