//! Office command access to core-owned identity and room history.

use crate::{
    identity_context,
    output::{Failure, after_cleanup},
    room_command,
};
use tmt_adapters::{config::ConfigPaths, storage::Storage};
use tmt_core::identity::Identity;

pub(crate) struct RoomHistory {
    pub id: String,
}

/// The Office handlers need only these core lookups. A process-backed
/// implementation can replace this adapter without changing the handlers.
pub(crate) trait CoreAccess {
    fn identity(&self, selector: Option<&str>) -> Result<Identity, Failure>;
    fn room_history(&self, selector: &str) -> Result<RoomHistory, Failure>;
}

pub(crate) struct InProcessCoreAccess;

fn unavailable(error: impl std::error::Error + 'static) -> Failure {
    Failure::new(
        "OFFICE_IO_ERROR",
        "Could not access local Office state or output.",
        1,
    )
    .caused_by(error)
}

impl CoreAccess for InProcessCoreAccess {
    fn identity(&self, selector: Option<&str>) -> Result<Identity, Failure> {
        // Verify an implicit caller before opening or migrating storage.
        let selector = identity_context::required(selector)?;
        let paths = ConfigPaths::discover().map_err(unavailable)?;
        let mut storage = Storage::open(paths.database).map_err(unavailable)?;
        let identity = identity_context::resolve(&mut storage, selector)?;
        storage.close().map_err(unavailable)?;
        Ok(identity)
    }

    fn room_history(&self, selector: &str) -> Result<RoomHistory, Failure> {
        let paths = ConfigPaths::discover().map_err(|error| {
            Failure::new("CONFIG_ERROR", "Could not resolve configuration paths.", 1)
                .caused_by(error)
        })?;
        let mut storage =
            Storage::open(paths.database).map_err(|error| room_command::failure(error.into()))?;
        let room = room_command::resolve_history(&mut storage, selector);
        after_cleanup(room, || storage.close()).map(|room| RoomHistory { id: room.id })
    }
}
