//! Signed owner-browser adapter. The serving coordinator remains the decision owner.
use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserInvitationInput {
    pub operation_id: String,
    pub offer_id: String,
    pub snapshot: String,
    pub matched: bool,
    #[serde(default)]
    pub sending: bool,
}
/// Injected by the existing serving composition, never selected by a wire field.
pub trait OwnerInvitationPort: Send + Sync {
    fn pending(&self, permit: &MessagePermit) -> Result<Value>;
    fn decide(&self, permit: &MessagePermit, input: &BrowserInvitationInput) -> Result<Value>;
}
pub struct NoOwnerInvitations;
impl OwnerInvitationPort for NoOwnerInvitations {
    fn pending(&self, _: &MessagePermit) -> Result<Value> {
        Err(unavailable())
    }
    fn decide(&self, _: &MessagePermit, _: &BrowserInvitationInput) -> Result<Value> {
        Err(unavailable())
    }
}
pub(crate) fn unavailable() -> RemoteError {
    RemoteError::new(
        "REMOTE_INVITATION_UNAVAILABLE",
        "Current invitation is unavailable.",
    )
}
pub(crate) fn authorize(permit: &MessagePermit) -> Result<()> {
    if !permit.owner_browser() {
        return Err(invalid());
    }
    permit.with_store(|store| require_writable(&store.connection, &permit.grant))
}
pub(crate) fn append(permit: &MessagePermit, owner: &dyn OwnerInvitationPort) -> Result<Vec<u8>> {
    authorize(permit)?;
    let envelope = permit.message.envelope();
    let result = if envelope.operation == "remote.invites.pending" {
        if permit.message.input != json!({}) {
            return Err(invalid());
        }
        owner.pending(permit)?
    } else {
        let input: BrowserInvitationInput = decode(&permit.message.input)?;
        if input.operation_id != envelope.id
            || !canonical::is_core_id(&input.operation_id)
            || canonical::uuid(&input.offer_id).is_err()
            || canonical::base64url_bytes(&input.snapshot, 32).is_err()
            || (!input.matched && input.sending)
        {
            return Err(invalid());
        }
        owner.decide(permit, &input)?
    };
    permit.response(&result)
}
