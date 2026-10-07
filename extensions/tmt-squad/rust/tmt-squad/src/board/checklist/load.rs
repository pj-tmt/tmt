//! Typed jobs on the existing board worker. Writes are never collapsed with reads.
use crate::{
    checklist::model::{Id, Request},
    checklist::{Applied, Code, Error, Preview, Service},
    config::Config,
    core::Core,
    squad::Squad,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    pub controller: u64,
    pub serial: u64,
    pub room: Option<Id>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Job {
    Rooms,
    Read,
    Ids,
    Apply { actor: Id, request: Box<Request> },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Task {
    pub key: Key,
    pub job: Job,
}
#[derive(Debug)]
pub(crate) enum Outcome {
    Rooms(Result<Vec<(Id, String)>, Error>),
    Read(Result<Preview, Error>),
    Ids(Result<(Id, Id), Error>),
    Apply(Result<Applied, Error>),
}
pub(crate) struct Completed {
    pub key: Key,
    pub outcome: Outcome,
}
#[derive(Default)]
pub(crate) struct Lane {
    session: Option<(u64, Id, Service)>,
}
fn failure(code: Code, message: impl ToString) -> Error {
    Error {
        code,
        message: message.to_string(),
        current: None,
    }
}
fn uuid() -> Result<Id, Error> {
    Id::parse(&crate::id::new_v4().map_err(|error| failure(Code::StorageError, error))?)
}
impl Lane {
    #[cfg(test)]
    pub(super) fn seeded(owner: u64, room: Id, service: Service) -> Self {
        Self {
            session: Some((owner, room, service)),
        }
    }
    fn service(&mut self, core: &Core, key: &Key) -> Result<&Service, Error> {
        let room = key
            .room
            .as_ref()
            .ok_or_else(|| failure(Code::InputInvalid, "No exact room selected."))?;
        if self
            .session
            .as_ref()
            .is_none_or(|(owner, id, _)| *owner != key.controller || id != room)
        {
            let config = Config::load(core).map_err(|error| failure(Code::StorageError, error))?;
            self.session = Some((
                key.controller,
                room.clone(),
                Service::new(core, &config, room.clone())?,
            ));
        }
        Ok(&self.session.as_ref().unwrap().2)
    }
    pub fn complete(&mut self, core: &Core, task: Task) -> Completed {
        let outcome = match task.job {
            Job::Rooms => Outcome::Rooms(
                Squad::list(core)
                    .map_err(|error| failure(Code::RoomUnavailable, error))
                    .and_then(|rooms| {
                        rooms
                            .into_iter()
                            .map(|room| Ok((Id::parse(&room.room_id)?, room.name)))
                            .collect()
                    }),
            ),
            Job::Read => Outcome::Read(self.service(core, &task.key).and_then(Service::preview)),
            Job::Ids => Outcome::Ids(uuid().and_then(|checklist| Ok((checklist, uuid()?)))),
            Job::Apply { actor, request } => Outcome::Apply((|| {
                let service = self.service(core, &task.key)?;
                // Revalidate admitted identity/authority, but never recapture a different caller.
                let preview = service.preview()?;
                if preview.actor_id != actor {
                    return Err(failure(Code::Forbidden, "The preview actor changed."));
                }
                service.apply(&request)
            })()),
        };
        Completed {
            key: task.key,
            outcome,
        }
    }
}
