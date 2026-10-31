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
pub(super) fn write(
    storage: &mut Storage,
    query: crate::orchestration::OrchWriteQuery,
) -> Result<Vec<u8>, Fault> {
    crate::orchestration::orchestration_write(storage, query, wall_time_ms).map_err(write_error)
}
pub(super) fn write_error(e: crate::orchestration::OrchReadFailure) -> Fault {
    match e {
        crate::orchestration::OrchReadFailure::Job(crate::storage::JobStoreError::Storage(error))=>Fault::new("ORCH_STORAGE_UNAVAILABLE","Job write could not be confirmed; inspect its definition and canonical receipt before retrying.").with_storage_access(error,"The job write could not be confirmed. Inspect its current revision before retrying."),
        error @ crate::orchestration::OrchReadFailure::Job(_)=>Fault::new(error.code(),error.message()),
        crate::orchestration::OrchReadFailure::Policy(e)=>Fault::new(crate::orchestration::orch_error_code(e),crate::orchestration::orch_error_message(e)),
        crate::orchestration::OrchReadFailure::Storage(e)=>Fault::new("ORCH_STORAGE_UNAVAILABLE","Orchestration write could not be confirmed; inspect its configuration revision before retrying.").with_storage_access(e, "The write could not be confirmed. Inspect its current configuration revision before retrying."),
    }
}
