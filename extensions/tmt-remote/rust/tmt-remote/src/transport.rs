//! Backend ingress port: carry exact envelopes and results to/from the common receiver.
//! Drivers neither interpret authority nor execute operations. The local driver is `routes::Routes`.
#[derive(Debug, PartialEq, Eq)]
pub struct Closed;

/// One explicit carrier call; failure never authorizes another send or implicit ack.
/// Origin is untrusted binding metadata, preserved for the common admission owner.
/// Subscribe returns a finite signed journal page; ack carries the caller's signed checkpoint.
pub trait Transport {
    fn append(&self, origin: Option<&str>, envelope: &[u8]) -> Result<Vec<u8>, Closed>;
    fn subscribe(&self, origin: Option<&str>, envelope: &[u8]) -> Result<Vec<u8>, Closed>;
    fn ack(&self, origin: Option<&str>, envelope: &[u8]) -> Result<Vec<u8>, Closed>;
}
