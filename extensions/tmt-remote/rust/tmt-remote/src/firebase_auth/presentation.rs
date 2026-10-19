//! One owner for the accepted command/status copy and secret-free projection.
use super::{AuthError, AuthErrorKind, AuthState, Status};
use serde::Serialize;
pub const SIGN_IN_ENDED: &str = "Firestore sign-in has ended. Run `tmt remote sync sign-in` to create a new identity. Page access for this machine is not available yet.";
pub const SIGN_IN_UNCONFIRMED: &str = "Firestore did not confirm the sign-in. Nothing was retried. Signing in again may create a second sign-in identity. Page access for this machine is not available yet.";
pub const REVOKE_UNCONFIRMED: &str =
    "Revoke not confirmed. Run `tmt remote sync revoke` again; nothing else changed.";
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
enum SignInState {
    None,
    Active,
    Expiring,
    NeedsRenewal,
    Unconfirmed,
    Unreadable,
    Busy,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInView {
    state: SignInState,
    expires_at_ms: Option<u64>,
}
pub fn project(status: &Status) -> SignInView {
    SignInView {
        state: match status.state {
            AuthState::Unenrolled => SignInState::None,
            AuthState::Active if status.expiry_notice => SignInState::Expiring,
            AuthState::Active => SignInState::Active,
            AuthState::ReSignInNeeded => SignInState::NeedsRenewal,
            AuthState::Revoking => SignInState::Unconfirmed,
        },
        expires_at_ms: status.expires_at_ms,
    }
}
pub fn unconfirmed(expires_at_ms: Option<u64>) -> SignInView {
    SignInView {
        state: SignInState::Unconfirmed,
        expires_at_ms,
    }
}
/// Credential diagnostics never hide the selected door's ordinary status.
pub fn diagnostic(error: &AuthError) -> SignInView {
    SignInView {
        state: match error.kind {
            AuthErrorKind::Unavailable | AuthErrorKind::StateChanged => SignInState::Busy,
            _ => SignInState::Unreadable,
        },
        expires_at_ms: None,
    }
}
pub fn status_line(status: &Status, now_ms: u64) -> Option<String> {
    match status.state {
        AuthState::Active if status.expiry_notice => status.expires_at_ms.map(|end| {
            let date = crate::calendar::utc_date(end);
            let days = end.saturating_sub(now_ms).div_ceil(86_400_000);
            format!("Firestore sign-in ends on {date} (in {days} days). After that, run `tmt remote sync sign-in` to create a new identity. Page access for this machine is not available yet.")
        }),
        AuthState::ReSignInNeeded => Some(SIGN_IN_ENDED.into()),
        AuthState::Revoking => Some(format!("Firestore sign-in: {REVOKE_UNCONFIRMED}")),
        _ => None,
    }
}
pub fn signed_in(uid: &str, reused: bool) -> String {
    let uid: String = uid.chars().take(8).collect();
    if reused {
        format!("Already signed in as {uid}. Nothing changed.")
    } else {
        format!(
            "Signed in to Firestore as {uid}. Page access for this machine is not available yet."
        )
    }
}
pub fn revoked(uid: Option<&str>) -> String {
    let outcome = match uid {
        Some(uid) => format!(
            "Revoked the Firestore sign-in {}.",
            uid.chars().take(8).collect::<String>()
        ),
        None => "Already revoked.".into(),
    };
    format!(
        "{outcome} Run `tmt remote sync sign-in` to create a new identity. Page access for this machine is not available yet."
    )
}
