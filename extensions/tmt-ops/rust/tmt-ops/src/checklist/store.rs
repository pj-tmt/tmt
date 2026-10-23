//! One document per canonical room UUID, and one per (room, member) UUID pair, in the Ops state
//! database.
use super::{
    Code, Error,
    model::{Document, Id},
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tmt_ops::state::{BUSY_RETRY, Cause, Database, Refusal, Slot, StateFailure};

/// Implementation resource limits, independent of individual admitted fields.
const LIMIT: u64 = 64 * 1024 * 1024;

pub(super) struct Store {
    extension_root: PathBuf,
    room_id: Id,
    /// Set for a conversation list; the room's own list has none.
    member_id: Option<Id>,
}
impl Store {
    pub(super) fn new(
        root: &Path,
        room_id: Id,
        member_id: Option<Id>,
        subtree: &'static str,
    ) -> Result<Self, Error> {
        if !root.is_absolute() {
            return Err(Error::storage(
                "storage.root must return an absolute dataRoot.",
            ));
        }
        // Aliases are allowed only in the trusted public root. The state database and the
        // legacy checklist files below it are checked by their owners.
        let root = match fs::canonicalize(root) {
            Ok(root) => root,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => root.into(),
            Err(e) => return Err(Error::storage(e)),
        };
        Ok(Self {
            extension_root: root.join(subtree),
            room_id,
            member_id,
        })
    }
    pub(super) fn room_id(&self) -> &Id {
        &self.room_id
    }
    pub(super) fn member_id(&self) -> Option<&Id> {
        self.member_id.as_ref()
    }
    fn slot(&self) -> Slot {
        let room = self
            .extension_root
            .join("checklist")
            .join(self.room_id.as_str());
        match &self.member_id {
            None => Slot {
                kind: "checklist",
                key: self.room_id.as_str().to_owned(),
                legacy: room.join("items.json"),
                limit: LIMIT,
            },
            // Both halves are canonical UUIDs, so this key cannot equal a room's. No earlier
            // release stored a conversation list, so its legacy file is a name nothing creates.
            Some(member) => Slot {
                kind: "checklist",
                key: format!("{}/{}", self.room_id.as_str(), member.as_str()),
                legacy: room
                    .join("members")
                    .join(member.as_str())
                    .join("items.json"),
                limit: LIMIT,
            },
        }
    }
    fn decode(&self, bytes: Option<&[u8]>) -> Result<Option<Document>, Error> {
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let value = serde_json::from_slice(bytes).map_err(Error::storage)?;
        let document = Document::decode(&value)?;
        if document.room_id != self.room_id {
            return Err(Error::storage(
                "Checklist document belongs to another room UUID.",
            ));
        }
        if document.member_id != self.member_id {
            return Err(Error::storage(
                "Checklist document belongs to another conversation.",
            ));
        }
        Ok(Some(document))
    }
    pub(super) fn read(&self) -> Result<Option<Document>, Error> {
        let slot = self.slot();
        let Some(mut database) =
            Database::open_existing(&self.extension_root, &slot.legacy).map_err(refused)?
        else {
            return Ok(None);
        };
        database
            .read(&slot, |bytes| self.decode(bytes))
            .map_err(failed)
    }
    /// `before_publish` runs after the candidate is staged and before it commits; its
    /// refusal discards the candidate, as does any failure of `change`.
    pub(super) fn update<T>(
        &self,
        change: impl FnOnce(&mut Option<Document>) -> Result<T, Error>,
        before_publish: impl FnOnce() -> Result<(), Error>,
    ) -> Result<T, Error> {
        let slot = self.slot();
        let mut database = Database::open(&self.extension_root).map_err(refused)?;
        let value = database
            .update(
                &slot,
                |bytes| {
                    let mut document = self.decode(bytes)?;
                    let before = document.clone();
                    let result = change(&mut document)?;
                    if document == before {
                        return Ok((None, result));
                    }
                    let document = document
                        .ok_or_else(|| Error::storage("Checklist documents cannot be removed."))?;
                    document.validate()?;
                    if document.room_id != self.room_id || document.member_id != self.member_id {
                        return Err(Error::storage(
                            "Candidate room or conversation UUID changed.",
                        ));
                    }
                    let bytes = serde_json::to_vec(&document.encode()).map_err(Error::storage)?;
                    if bytes.len() as u64 > LIMIT {
                        return Err(Error::storage(
                            "Checklist document exceeds the 64 MiB implementation limit.",
                        ));
                    }
                    Ok((Some(bytes), result))
                },
                |_| {
                    #[cfg(test)]
                    test_stage(Stage::BeforePublish).map_err(Error::storage)?;
                    before_publish()
                },
            )
            .map_err(failed)?;
        #[cfg(test)]
        test_stage(Stage::AfterCommit).map_err(|e| {
            refused(Refusal {
                cause: Cause::Busy,
                message: e.to_string(),
                may_not_be_saved: true,
            })
        })?;
        Ok(value)
    }
}

fn refused(refusal: Refusal) -> Error {
    if refusal.may_not_be_saved {
        return Error::new(
            Code::OutcomeUnknown,
            format!(
                "Checklist replacement may have committed; original operation outcome remains unknown: {}",
                refusal.message
            ),
        );
    }
    match refusal.cause {
        Cause::Busy => Error::storage(format!("{} {BUSY_RETRY}", refusal.message)),
        Cause::Newer | Cause::Failed => Error::storage(refusal.message),
    }
}
fn failed(failure: StateFailure<Error>) -> Error {
    match failure {
        StateFailure::Refused(refusal) => refused(refusal),
        StateFailure::Change(error) => error,
    }
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
    /// Inside the transaction, after the candidate is written and before it commits.
    BeforePublish,
    /// After the commit succeeded, standing in for an acknowledgement that never arrived.
    AfterCommit,
}
#[cfg(test)]
type Fault = Box<dyn FnMut(Stage) -> std::io::Result<()>>;
#[cfg(test)]
thread_local! { static FAULT: std::cell::RefCell<Option<Fault>> = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
fn test_stage(stage: Stage) -> std::io::Result<()> {
    FAULT.with_borrow_mut(|fault| match fault {
        Some(fault) => fault(stage),
        None => Ok(()),
    })
}
#[cfg(test)]
pub(super) fn with_fault<T>(fault: Fault, run: impl FnOnce() -> T) -> T {
    struct Reset(Option<Fault>);
    impl Drop for Reset {
        fn drop(&mut self) {
            FAULT.with_borrow_mut(|f| *f = self.0.take());
        }
    }
    let _reset = Reset(FAULT.replace(Some(fault)));
    run()
}

#[cfg(test)]
mod tests;
