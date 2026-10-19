//! Final UX copy and secret-free JSON projections; no provider or filesystem.
use serde_json::json;
use tmt_remote::firebase_auth::{AuthState, Status, presentation};
fn status(state: AuthState, expires_at_ms: Option<u64>, notice: bool) -> Status {
    Status {
        state,
        uid: Some("fixture-identity-long".into()),
        expires_at_ms,
        expiry_notice: notice,
        permissions_repaired: false,
    }
}
#[test]
fn expiring_copy_waits_until_after_the_fixed_date_and_json_never_uses_zero() {
    let end = 1_794_441_600_000;
    let value = status(AuthState::Active, Some(end), true);
    assert_eq!(
        presentation::status_line(&value, end - 12 * 86_400_000).unwrap(),
        "Firestore sign-in ends on 2026-11-12 (in 12 days). After that, run `tmt remote sync sign-in` to create a new identity. Page access for this machine is not available yet."
    );
    assert_eq!(
        serde_json::to_value(presentation::project(&value)).unwrap(),
        json!({"state":"expiring","expiresAtMs":end})
    );
    assert!(
        presentation::status_line(
            &status(AuthState::Active, Some(end), false),
            end - 31 * 86_400_000
        )
        .is_none()
    );
    assert_eq!(
        serde_json::to_value(presentation::project(&status(
            AuthState::Unenrolled,
            None,
            false
        )))
        .unwrap(),
        json!({"state":"none","expiresAtMs":null})
    );
}
#[test]
fn ended_and_unknown_lines_name_the_actual_next_step_without_renewal_copy() {
    let ended = status(AuthState::ReSignInNeeded, None, false);
    assert_eq!(
        presentation::status_line(&ended, 0).unwrap(),
        "Firestore sign-in has ended. Run `tmt remote sync sign-in` to create a new identity. Page access for this machine is not available yet."
    );
    assert_eq!(
        serde_json::to_value(presentation::project(&ended)).unwrap(),
        json!({"state":"needs-renewal","expiresAtMs":null})
    );
    assert_eq!(
        presentation::SIGN_IN_UNCONFIRMED,
        "Firestore did not confirm the sign-in. Nothing was retried. Signing in again may create a second sign-in identity. Page access for this machine is not available yet."
    );
    assert_eq!(
        serde_json::to_value(presentation::unconfirmed(None)).unwrap(),
        json!({"state":"unconfirmed","expiresAtMs":null})
    );
}
#[test]
fn human_command_outputs_abbreviate_uid_and_active_sign_in_says_nothing_changed() {
    assert_eq!(
        presentation::signed_in("fixture-identity-long", false),
        "Signed in to Firestore as fixture-. Page access for this machine is not available yet."
    );
    assert_eq!(
        presentation::signed_in("fixture-identity-long", true),
        "Already signed in as fixture-. Nothing changed."
    );
    assert_eq!(
        presentation::revoked(Some("fixture-identity-long")),
        "Revoked the Firestore sign-in fixture-. Run `tmt remote sync sign-in` to create a new identity. Page access for this machine is not available yet."
    );
    assert_eq!(
        presentation::revoked(None),
        "Already revoked. Run `tmt remote sync sign-in` to create a new identity. Page access for this machine is not available yet."
    );
    assert_eq!(
        presentation::REVOKE_UNCONFIRMED,
        "Revoke not confirmed. Run `tmt remote sync revoke` again; nothing else changed."
    );
}
