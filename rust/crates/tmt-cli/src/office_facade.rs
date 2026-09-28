//! Reserved Office command registration and its in-process core lookup bridge.

pub(crate) use tmt_office_command::{grammar, invocation, parser};

pub(crate) fn execute(
    prefix: Option<String>,
    operation: invocation::OfficeOperation,
    mode: crate::invocation::OutputMode,
) -> std::io::Result<u8> {
    tmt_office_command::execute(prefix, operation, mode, &InProcessCoreAccess)
}

use crate::{
    identity_context,
    output::{Failure, after_cleanup},
    room_command,
};
use tmt_adapters::{config::ConfigPaths, storage::Storage};
use tmt_office_command::core_access::{CoreAccess, OfficeIdentity, RoomHistory};

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
    fn identity(&self, selector: Option<&str>) -> Result<OfficeIdentity, Failure> {
        // Verify an implicit caller before opening or migrating storage.
        let selector = identity_context::required(selector)?;
        let paths = ConfigPaths::discover().map_err(unavailable)?;
        let mut storage = Storage::open(paths.database).map_err(unavailable)?;
        let identity = identity_context::resolve(&mut storage, selector)?;
        storage.close().map_err(unavailable)?;
        Ok(OfficeIdentity {
            id: identity.id,
            name: identity.name,
        })
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
