//! What `objects.config` projects: the backend the service was delivered, its supported
//! interface and its effective bounds. It is a snapshot taken when the service opens, so
//! answering a request reads no usage, allocation or ledger row, creates no namespace and
//! file and mutates nothing, and it carries no path, secret or inventory.
use crate::objects::{BackendCaps, Quotas};
use tmt_extension_objects::{Config, Limits, Projection, TransferBounds};

#[derive(Clone, Copy, Debug)]
pub(super) struct ConfigSource {
    pub(super) backend_id: &'static str,
    pub(super) caps: BackendCaps,
    pub(super) quotas: Quotas,
}
impl ConfigSource {
    /// The configuration one origin may see: the owner's view adds the effective quotas.
    pub(super) fn project(&self, projection: Projection) -> Config {
        let bounds = TransferBounds {
            payload_bytes: self.caps.max_payload_bytes,
            chunk_bytes: self.caps.chunk_bytes,
        };
        Config {
            backend_id: self.backend_id.to_owned(),
            // What the delivered backend guarantees: create-only publication, chunked
            // reads, and recovery of an interrupted original by its original ID.
            immutable_create: true,
            chunked_read: true,
            recover_by_original_id: true,
            limits: match projection {
                Projection::Browser => Limits::Browser(bounds),
                Projection::Local => Limits::Local {
                    bounds,
                    namespace_bytes: self.quotas.namespace_bytes,
                    extension_bytes: self.quotas.extension_bytes,
                    installation_bytes: self.quotas.installation_bytes,
                },
            },
        }
    }
}
