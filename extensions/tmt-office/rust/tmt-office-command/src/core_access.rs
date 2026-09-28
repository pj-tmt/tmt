//! The only core-owned lookups used by public Office handlers.

use tmt_command_output::Failure;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficeIdentity {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomHistory {
    pub id: String,
}

pub trait CoreAccess {
    fn identity(&self, selector: Option<&str>) -> Result<OfficeIdentity, Failure>;
    fn room_history(&self, selector: &str) -> Result<RoomHistory, Failure>;
}
