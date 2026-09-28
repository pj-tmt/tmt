//! Office storage operations shared by the one-shot companion protocol and
//! the local HTTP service. Each opens, uses and closes the Office store.

pub mod avatar;
pub mod board;
pub mod profile;
pub mod prop;
pub mod storage;
pub mod whiteboard;
pub mod world;

/// Reconciles Office's stored references before a one-shot write command.
/// A failure never blocks the command and cannot be reported on the
/// protocol's stderr; point-of-use and in-transaction checks still apply.
/// The local service reconciles once at start instead of per request.
pub(crate) fn reconcile_before_write(layout: &crate::StorageLayout) {
    if let Ok(mut store) = crate::OfficeStore::open_configured(layout) {
        let _ = store.reconcile(tmt_adapters::request_runtime::wall_time_ms() as i64);
        let _ = store.close();
    }
}
