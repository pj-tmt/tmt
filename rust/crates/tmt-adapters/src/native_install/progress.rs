//! Read-only observations of the existing installer. An observer has no vote
//! in verification, cancellation or publication and cannot return an error.

#[derive(Debug, Clone, Copy)]
pub enum ProgressEvent<'a> {
    Selected {
        version: &'a str,
        archive_bytes: u64,
    },
    Download {
        received_bytes: u64,
        total_bytes: Option<u64>,
    },
    Downloaded,
    Verifying,
    Verified,
    Activating,
    Activated,
}

pub type ProgressObserver<'a> = &'a dyn Fn(ProgressEvent<'_>);

#[derive(Debug, Clone)]
pub struct ReleasePreview {
    pub version: String,
    pub archive_bytes: u64,
}
