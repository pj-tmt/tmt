//! One-shot fallback policy shared by requests and advisory hints.

use super::{ActionResult, DeliveryAcceptance, SendFailure};

pub fn send_preferred<E>(
    preferred: impl FnOnce() -> ActionResult<DeliveryAcceptance, SendFailure<E>>,
    fallback: impl FnOnce() -> ActionResult<DeliveryAcceptance, SendFailure<E>>,
) -> ActionResult<DeliveryAcceptance, SendFailure<E>> {
    match preferred() {
        ActionResult::Unsupported | ActionResult::Failed(SendFailure::NotSent(_)) => fallback(),
        terminal => terminal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_unsupported_or_proven_non_delivery_can_fall_back() {
        for result in [
            ActionResult::Unsupported,
            ActionResult::Failed(SendFailure::NotSent(())),
        ] {
            assert_eq!(
                send_preferred(
                    || result,
                    || ActionResult::Completed(DeliveryAcceptance::Submitted)
                ),
                ActionResult::Completed(DeliveryAcceptance::Submitted)
            );
        }
        for result in [
            ActionResult::Completed(DeliveryAcceptance::Queued),
            ActionResult::Completed(DeliveryAcceptance::Submitted),
            ActionResult::Failed(SendFailure::Uncertain(())),
            ActionResult::Failed(SendFailure::Denied(())),
            ActionResult::Failed(SendFailure::AwaitingApproval(())),
        ] {
            let expected = result.clone();
            assert_eq!(
                send_preferred(
                    || result,
                    || panic!("accepted, uncertain or policy-blocked send must not fall back")
                ),
                expected
            );
        }
    }
}
