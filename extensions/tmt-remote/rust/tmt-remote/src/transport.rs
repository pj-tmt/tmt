//! Bindings move envelopes to one message owner; they never invoke CoreClient.
#[derive(Debug, PartialEq, Eq)]
pub struct Closed;
pub trait Transport {
    fn append(&self, envelope: &[u8]) -> Result<Vec<u8>, Closed>;
    fn subscribe(&self, envelope: &[u8]) -> Result<Vec<u8>, Closed>;
    fn ack(&self, envelope: &[u8]) -> Result<Vec<u8>, Closed>;
}
// This interim message owner cannot authenticate or adopt any request.
struct MessageService;
impl MessageService {
    fn admit(&self, _envelope: &[u8]) -> Result<Vec<u8>, Closed> {
        Err(Closed)
    }
}
pub struct LoopbackTransport {
    message: MessageService,
}
impl Default for LoopbackTransport {
    fn default() -> Self {
        Self {
            message: MessageService,
        }
    }
}
impl Transport for LoopbackTransport {
    fn append(&self, bytes: &[u8]) -> Result<Vec<u8>, Closed> {
        self.message.admit(bytes)
    }
    fn subscribe(&self, bytes: &[u8]) -> Result<Vec<u8>, Closed> {
        self.message.admit(bytes)
    }
    fn ack(&self, bytes: &[u8]) -> Result<Vec<u8>, Closed> {
        self.message.admit(bytes)
    }
}
