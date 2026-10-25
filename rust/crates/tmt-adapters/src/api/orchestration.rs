use super::Fault;
use crate::{
    orchestration::{OrchReadQuery, orchestration_read},
    request_runtime::wall_time_ms,
    storage::Storage,
};

pub(super) fn read(storage: &Storage, query: OrchReadQuery) -> Result<Vec<u8>, Fault> {
    orchestration_read(storage, query, wall_time_ms())
        .map_err(|e| Fault::new(e.code(), e.message()))
}
