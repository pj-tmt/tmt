//! Office's durable identity-retirement consumer.
//!
//! Core owns the subscriptions: this consumer reads its pending deliveries,
//! records each attempt and acknowledges through `tmt api`, scoped to its own
//! consumer name. Each delivery is settled under the pairing scope lock by
//! `office_pairing::settle_scope` (mark, revoke, then acknowledge), so a
//! failure leaves the delivery pending with credentials intact.

use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tmt_adapters::{
    config::ConfigPaths,
    office_pairing::{OfficeInstallation, PairingCore, PairingIdentity, settle_scope},
};
use tmt_office_command::process_core_access::ProcessCoreAccess;
use tmt_office_model::office_protocol::{OFFICE_HOOK_BATCH_LIMIT, OfficeError, OfficeSyncReport};
use tmt_office_storage::retirement::OfficeRetirementFence;

/// This consumer's name in core's hook subscriptions.
pub const CONSUMER: &str = "tmt-office";

/// One pending delivery for this consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub identity_id: String,
    pub reference: String,
}

/// The consumer-scoped hook operations of `tmt api`.
pub trait HookApi {
    fn pending(&self, limit: usize) -> Result<Vec<Delivery>, OfficeError>;
    fn remaining(&self) -> Result<u64, OfficeError>;
    fn attempt(&self, delivery: &Delivery) -> Result<bool, OfficeError>;
    fn acknowledge(&self, delivery: &Delivery) -> Result<bool, OfficeError>;
}

/// Settles one batch. `settle` runs the scope's action and calls the given
/// acknowledgment only after it succeeded.
pub fn sync(
    api: &dyn HookApi,
    deadline: Instant,
    mut settle: impl FnMut(
        &Delivery,
        &dyn Fn() -> Result<bool, OfficeError>,
    ) -> Result<bool, OfficeError>,
) -> Result<OfficeSyncReport, OfficeError> {
    let mut report = OfficeSyncReport::default();
    for delivery in api.pending(OFFICE_HOOK_BATCH_LIMIT)? {
        if Instant::now() >= deadline {
            break;
        }
        // Record the attempt before external effects. Contended or failed work
        // rotates behind less-attempted items instead of starving the queue.
        if !api.attempt(&delivery)? {
            continue;
        }
        match settle(&delivery, &|| api.acknowledge(&delivery)) {
            Ok(true) => report.completed += 1,
            Ok(false) => {}
            Err(error) => {
                report.failed += 1;
                report.failure.get_or_insert(error);
            }
        }
    }
    report.pending = api.remaining()?;
    Ok(report)
}

/// The companion's `sync` operation: input `{}`.
pub fn execute(input: &[u8]) -> Vec<u8> {
    let result = (|| {
        if serde_json::from_slice::<Value>(input).ok() != Some(json!({})) {
            return Err(OfficeError::CredentialsInvalid);
        }
        let paths = ConfigPaths::discover().map_err(|_| OfficeError::CredentialsUnavailable)?;
        // No core database yet means no registered hooks; asking core would
        // create one. Only the file's presence is checked, never its contents.
        match std::fs::symlink_metadata(&paths.database) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(OfficeSyncReport::default());
            }
            Err(_) => return Err(OfficeError::CredentialsUnavailable),
            Ok(_) => {}
        }
        let core = CoreApi::discover()?;
        let fence = OfficeRetirementFence::discover();
        let deadline = Instant::now() + Duration::from_secs(25);
        let mut installation = None;
        sync(&core, deadline, |delivery, acknowledge| {
            if installation.is_none() {
                installation = Some(
                    OfficeInstallation::open(&paths, false)?
                        .ok_or(OfficeError::CredentialsUnavailable)?,
                );
            }
            settle_scope(
                installation.as_ref().expect("opened above"),
                &delivery.identity_id,
                &delivery.reference,
                &fence,
                deadline,
                acknowledge,
            )
        })
    })();
    let value = match result {
        Ok(report) => json!({
            "completed": report.completed,
            "failed": report.failed,
            "pending": report.pending,
            "failureCode": report.failure.map(|error| error.code()),
        }),
        Err(error) => json!({"error": error.code()}),
    };
    serde_json::to_vec(&value).expect("sync report is JSON")
}

/// Core reached through the invoking `tmt`: public identity reads and the
/// consumer-scoped hook operations.
pub struct CoreApi(ProcessCoreAccess);

impl CoreApi {
    pub fn discover() -> Result<Self, OfficeError> {
        ProcessCoreAccess::discover()
            .map(Self)
            .map_err(|_| OfficeError::CredentialsUnavailable)
    }

    fn call(&self, operation: &str, input: Value) -> Result<Value, OfficeError> {
        self.0
            .api(operation, input)
            .map_err(|_| OfficeError::CredentialsUnavailable)
    }

    fn hook(&self, operation: &str, delivery: &Delivery, field: &str) -> Result<bool, OfficeError> {
        self.call(
            operation,
            json!({"consumer": CONSUMER, "identityId": delivery.identity_id, "reference": delivery.reference}),
        )?[field]
            .as_bool()
            .ok_or(OfficeError::CredentialsUnavailable)
    }
}

impl HookApi for CoreApi {
    fn pending(&self, limit: usize) -> Result<Vec<Delivery>, OfficeError> {
        let page = self.call(
            "identityHooks.pending",
            json!({"consumer": CONSUMER, "limit": limit}),
        )?;
        page["hooks"]
            .as_array()
            .ok_or(OfficeError::CredentialsUnavailable)?
            .iter()
            .map(|hook| {
                Ok(Delivery {
                    identity_id: hook["identityId"]
                        .as_str()
                        .ok_or(OfficeError::CredentialsUnavailable)?
                        .to_owned(),
                    reference: hook["reference"]
                        .as_str()
                        .ok_or(OfficeError::CredentialsUnavailable)?
                        .to_owned(),
                })
            })
            .collect()
    }

    fn remaining(&self) -> Result<u64, OfficeError> {
        self.call(
            "identityHooks.pending",
            json!({"consumer": CONSUMER, "limit": 1}),
        )?["pending"]
            .as_u64()
            .ok_or(OfficeError::CredentialsUnavailable)
    }

    fn attempt(&self, delivery: &Delivery) -> Result<bool, OfficeError> {
        self.hook("identityHooks.attempt", delivery, "recorded")
    }

    fn acknowledge(&self, delivery: &Delivery) -> Result<bool, OfficeError> {
        self.hook("identityHooks.ack", delivery, "acknowledged")
    }
}

impl PairingCore for CoreApi {
    fn active_identity(&self, identity_id: &str) -> Result<Option<PairingIdentity>, OfficeError> {
        self.0
            .active_identity_by_id(identity_id)
            .map(|found| {
                found.map(|identity| PairingIdentity {
                    id: identity.id,
                    name: identity.name,
                })
            })
            .map_err(|_| OfficeError::CredentialsUnavailable)
    }

    fn register_retirement_hook(
        &self,
        identity_id: &str,
        reference: &str,
    ) -> Result<(), OfficeError> {
        self.call(
            "identityHooks.register",
            json!({"consumer": CONSUMER, "identityId": identity_id, "reference": reference}),
        )
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const ID: &str = "11111111-1111-4111-8111-111111111111";

    /// Core's hook state machine for one consumer: pending until acknowledged.
    #[derive(Default)]
    struct Api {
        pending: RefCell<Vec<(Delivery, bool)>>,
        log: RefCell<Vec<String>>,
    }

    impl Api {
        fn with(references: &[&str]) -> Self {
            let api = Self::default();
            for reference in references {
                api.pending.borrow_mut().push((
                    Delivery {
                        identity_id: ID.into(),
                        reference: (*reference).into(),
                    },
                    false,
                ));
            }
            api
        }
    }

    impl HookApi for Api {
        fn pending(&self, limit: usize) -> Result<Vec<Delivery>, OfficeError> {
            Ok(self
                .pending
                .borrow()
                .iter()
                .filter(|(_, delivered)| !delivered)
                .take(limit)
                .map(|(delivery, _)| delivery.clone())
                .collect())
        }
        fn remaining(&self) -> Result<u64, OfficeError> {
            Ok(self
                .pending
                .borrow()
                .iter()
                .filter(|(_, delivered)| !delivered)
                .count() as u64)
        }
        fn attempt(&self, delivery: &Delivery) -> Result<bool, OfficeError> {
            self.log
                .borrow_mut()
                .push(format!("attempt {}", delivery.reference));
            Ok(self
                .pending
                .borrow()
                .iter()
                .any(|(pending, delivered)| pending == delivery && !delivered))
        }
        fn acknowledge(&self, delivery: &Delivery) -> Result<bool, OfficeError> {
            self.log
                .borrow_mut()
                .push(format!("ack {}", delivery.reference));
            let mut pending = self.pending.borrow_mut();
            let entry = pending
                .iter_mut()
                .find(|(pending, _)| pending == delivery)
                .unwrap();
            let changed = !entry.1;
            entry.1 = true;
            Ok(changed)
        }
    }

    fn far() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    #[test]
    fn each_delivery_records_an_attempt_then_acknowledges_only_after_its_action() {
        let api = Api::with(&["a", "b"]);
        let report = sync(&api, far(), |delivery, acknowledge| {
            api.log
                .borrow_mut()
                .push(format!("revoke {}", delivery.reference));
            acknowledge()
        })
        .unwrap();
        assert_eq!((report.completed, report.failed, report.pending), (2, 0, 0));
        assert_eq!(
            *api.log.borrow(),
            [
                "attempt a",
                "revoke a",
                "ack a",
                "attempt b",
                "revoke b",
                "ack b"
            ]
        );
    }

    #[test]
    fn a_failed_action_keeps_the_delivery_pending_and_is_reported() {
        let api = Api::with(&["a", "b"]);
        let report = sync(&api, far(), |delivery, acknowledge| {
            if delivery.reference == "a" {
                Err(OfficeError::RemoteUncertain)
            } else {
                acknowledge()
            }
        })
        .unwrap();
        assert_eq!((report.completed, report.failed, report.pending), (1, 1, 1));
        assert_eq!(report.failure, Some(OfficeError::RemoteUncertain));
        assert!(!api.log.borrow().contains(&"ack a".to_owned()));
        // The retry settles it; the idempotent action runs again, the ack once.
        let report = sync(&api, far(), |_, acknowledge| acknowledge()).unwrap();
        assert_eq!((report.completed, report.pending), (1, 0));
    }

    #[test]
    fn a_crash_before_acknowledgment_retries_and_duplicates_settle_once() {
        let api = Api::with(&["a"]);
        // The action succeeded, then the process died before acknowledging.
        let report = sync(&api, far(), |_, _| Err(OfficeError::CredentialsUnavailable)).unwrap();
        assert_eq!(report.pending, 1);
        let acknowledged = RefCell::new(0);
        for _ in 0..2 {
            sync(&api, far(), |_, acknowledge| {
                let changed = acknowledge()?;
                *acknowledged.borrow_mut() += usize::from(changed);
                Ok(changed)
            })
            .unwrap();
        }
        assert_eq!(*acknowledged.borrow(), 1);
        assert_eq!(api.remaining().unwrap(), 0);
    }

    #[test]
    fn an_unrecorded_attempt_is_skipped_and_the_deadline_stops_the_batch() {
        struct Contended(Api);
        impl HookApi for Contended {
            fn pending(&self, limit: usize) -> Result<Vec<Delivery>, OfficeError> {
                self.0.pending(limit)
            }
            fn remaining(&self) -> Result<u64, OfficeError> {
                self.0.remaining()
            }
            fn attempt(&self, _: &Delivery) -> Result<bool, OfficeError> {
                Ok(false)
            }
            fn acknowledge(&self, delivery: &Delivery) -> Result<bool, OfficeError> {
                self.0.acknowledge(delivery)
            }
        }
        let report = sync(&Contended(Api::with(&["a"])), far(), |_, _| unreachable!()).unwrap();
        assert_eq!((report.completed, report.pending), (0, 1));
        let report = sync(&Api::with(&["a"]), Instant::now(), |_, _| unreachable!()).unwrap();
        assert_eq!((report.completed, report.pending), (0, 1));
    }

    #[test]
    fn sync_input_is_exactly_an_empty_object() {
        for input in [&b""[..], b"[]", br#"{"x":1}"#] {
            assert_eq!(
                execute(input),
                br#"{"error":"OFFICE_CREDENTIALS_INVALID"}"#.to_vec()
            );
        }
    }
}
