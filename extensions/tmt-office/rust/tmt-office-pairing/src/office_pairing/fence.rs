//! Office's retirement fence for pairing authority.
//!
//! Office storage owns the durable record; this owner only consults it. Every
//! write that grants or extends pairing authority runs under the scope lock,
//! which the retirement consumer also holds while it marks, revokes and
//! acknowledges, so a grant can never land after the consumer settled a scope.

use super::OfficeError;

pub trait RetirementFence {
    /// Whether this identity must not gain or extend pairing authority.
    fn is_retired(&self, identity_id: &str) -> Result<bool, OfficeError>;
    /// Durably records that Office observed this identity's retirement.
    /// Repeating it changes nothing.
    fn mark_retired(&self, identity_id: &str) -> Result<(), OfficeError>;
}

/// Writes pairing authority only while the identity is not fenced. Callers
/// hold the scope lock.
pub(super) fn write_authority(
    fence: &dyn RetirementFence,
    identity_id: &str,
    write: impl FnOnce() -> Result<(), OfficeError>,
) -> Result<(), OfficeError> {
    if fence.is_retired(identity_id)? {
        return Err(OfficeError::NotPaired);
    }
    write()
}

/// Settles one retirement delivery under the scope lock: mark, then revoke,
/// then acknowledge, each only after the previous step succeeded. An
/// interruption leaves the delivery pending, and every step is idempotent, so
/// a retry reaches the same state.
pub(super) fn settle_retirement<T>(
    fence: &dyn RetirementFence,
    identity_id: &str,
    revoke: impl FnOnce() -> Result<(), OfficeError>,
    acknowledge: impl FnOnce() -> Result<T, OfficeError>,
) -> Result<T, OfficeError> {
    fence.mark_retired(identity_id)?;
    revoke()?;
    acknowledge()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    struct Fence {
        retired: Cell<bool>,
        fail_mark: Cell<bool>,
        log: RefCell<Vec<&'static str>>,
    }

    impl RetirementFence for Fence {
        fn is_retired(&self, _: &str) -> Result<bool, OfficeError> {
            self.log.borrow_mut().push("check");
            Ok(self.retired.get())
        }
        fn mark_retired(&self, _: &str) -> Result<(), OfficeError> {
            if self.fail_mark.get() {
                return Err(OfficeError::CredentialsUnavailable);
            }
            self.log.borrow_mut().push("mark");
            self.retired.set(true);
            Ok(())
        }
    }

    const ID: &str = "11111111-1111-4111-8111-111111111111";

    #[test]
    fn authority_is_written_only_for_an_unfenced_identity() {
        let fence = Fence::default();
        let written = Cell::new(0);
        write_authority(&fence, ID, || {
            written.set(written.get() + 1);
            Ok(())
        })
        .unwrap();
        fence.retired.set(true);
        assert_eq!(
            write_authority(&fence, ID, || {
                written.set(written.get() + 1);
                Ok(())
            }),
            Err(OfficeError::NotPaired)
        );
        assert_eq!(written.get(), 1);
        // A fence that cannot answer blocks the write too.
        struct Unavailable;
        impl RetirementFence for Unavailable {
            fn is_retired(&self, _: &str) -> Result<bool, OfficeError> {
                Err(OfficeError::CredentialsUnavailable)
            }
            fn mark_retired(&self, _: &str) -> Result<(), OfficeError> {
                unreachable!()
            }
        }
        assert_eq!(
            write_authority(&Unavailable, ID, || unreachable!()),
            Err(OfficeError::CredentialsUnavailable)
        );
    }

    #[test]
    fn retirement_marks_then_revokes_then_acknowledges() {
        let fence = Fence::default();
        let log = &fence.log;
        let acknowledged = settle_retirement(
            &fence,
            ID,
            || {
                log.borrow_mut().push("revoke");
                Ok(())
            },
            || {
                log.borrow_mut().push("ack");
                Ok(true)
            },
        )
        .unwrap();
        assert!(acknowledged);
        assert_eq!(*log.borrow(), ["mark", "revoke", "ack"]);
    }

    #[test]
    fn an_interrupted_settlement_is_never_acknowledged_and_retries_cleanly() {
        let fence = Fence::default();
        let acknowledged = Cell::new(0);
        fence.fail_mark.set(true);
        let result = settle_retirement(
            &fence,
            ID,
            || unreachable!(),
            || unreachable!() as Result<(), _>,
        );
        assert_eq!(result, Err(OfficeError::CredentialsUnavailable));
        fence.fail_mark.set(false);
        // Marked, but revocation failed: still pending, and the scope is fenced.
        let result = settle_retirement(
            &fence,
            ID,
            || Err(OfficeError::RemoteUncertain),
            || {
                acknowledged.set(acknowledged.get() + 1);
                Ok(())
            },
        );
        assert_eq!(result, Err(OfficeError::RemoteUncertain));
        assert_eq!(acknowledged.get(), 0);
        assert!(fence.is_retired(ID).unwrap());
        // The retry repeats the idempotent mark and revoke, then acknowledges once.
        settle_retirement(
            &fence,
            ID,
            || Ok(()),
            || {
                acknowledged.set(acknowledged.get() + 1);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(acknowledged.get(), 1);
    }
}
