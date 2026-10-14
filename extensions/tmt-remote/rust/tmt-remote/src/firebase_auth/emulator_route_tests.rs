use super::{
    AuthError, AuthErrorKind, Cancellation, Http, Operation, Request, Response,
    emulator_route::EmulatorRoute,
};
use std::sync::{Mutex, atomic::AtomicBool};

#[test]
fn emulator_route_accepts_only_demo_loopback_and_closed_official_paths() {
    let route = EmulatorRoute::new("demo-proof", "127.0.0.1:9099", "[::1]:8085").unwrap();
    for (url, expected) in [
        (
            "https://identitytoolkit.googleapis.com/v1/accounts:signUp?key=public",
            "http://127.0.0.1:9099/identitytoolkit.googleapis.com/v1/accounts:signUp?key=public",
        ),
        (
            "https://identitytoolkit.googleapis.com/v1/accounts:delete?key=public",
            "http://127.0.0.1:9099/identitytoolkit.googleapis.com/v1/accounts:delete?key=public",
        ),
        (
            "https://securetoken.googleapis.com/v1/token?key=public",
            "http://127.0.0.1:9099/securetoken.googleapis.com/v1/token?key=public",
        ),
        (
            "https://firestore.googleapis.com/v1/projects/demo-proof/databases/(default)/documents:commit",
            "http://[::1]:8085/v1/projects/demo-proof/databases/(default)/documents:commit",
        ),
        (
            "https://firestore.googleapis.com/v1/projects/demo-proof/databases/(default)/documents/log/one",
            "http://[::1]:8085/v1/projects/demo-proof/databases/(default)/documents/log/one",
        ),
        (
            "https://firestore.googleapis.com/v1/projects/demo-proof/databases/(default)/documents/x/log:runAggregationQuery",
            "http://[::1]:8085/v1/projects/demo-proof/databases/(default)/documents/x/log:runAggregationQuery",
        ),
    ] {
        assert_eq!(route.url(url).unwrap(), expected);
    }
    for url in [
        "http://identitytoolkit.googleapis.com/v1/accounts:signUp?key=public",
        "https://identitytoolkit.googleapis.com.evil/v1/accounts:signUp?key=public",
        "https://identitytoolkit.googleapis.com/v1/accounts:lookup?key=public",
        "https://identitytoolkit.googleapis.com/v1/accounts:signUp?key=public&other=1",
        "https://firestore.googleapis.com/v1/projects/real-proof/databases/(default)/documents:commit",
        "https://firestore.googleapis.com/v1/projects/demo-other/databases/(default)/documents:commit",
        "https://firestore.googleapis.com/v1/projects/demo-proof/databases/other/documents:commit",
        "https://firestore.googleapis.com/v1/projects/demo-proof/databases/(default)/documents:unknown",
        "https://firestore.googleapis.com/v1/projects/demo-proof/databases/(default)/documents/a/../b",
        "https://firestore.googleapis.com/v1/projects/demo-proof/databases/(default)/documents/a/%2e",
        "https://firestore.googleapis.com/v1/projects/demo-proof/databases/(default)/documents/log/one?alt=media",
        "https://example.com/redirect",
    ] {
        assert!(
            matches!(route.url(url), Err(e) if e.kind == AuthErrorKind::InputInvalid),
            "{url}"
        );
    }
}

#[test]
fn emulator_route_refuses_non_demo_or_non_loopback_authorities() {
    for (project, auth, firestore) in [
        ("real-proof", "127.0.0.1:9099", "127.0.0.1:8085"),
        ("demo-", "127.0.0.1:9099", "127.0.0.1:8085"),
        ("demo-proof", "example.com:9099", "127.0.0.1:8085"),
        ("demo-proof", "127.0.0.1:9099", "192.0.2.1:8085"),
        ("demo-proof", "127.0.0.1:0", "127.0.0.1:8085"),
        ("demo-proof", "http://127.0.0.1:9099", "127.0.0.1:8085"),
        ("demo-proof", "user@127.0.0.1:9099", "127.0.0.1:8085"),
        ("demo-proof", "127.0.0.1:9099/path", "127.0.0.1:8085"),
    ] {
        assert!(
            matches!(EmulatorRoute::new(project, auth, firestore), Err(e) if e.kind == AuthErrorKind::InputInvalid)
        );
    }
}

#[test]
fn emulator_substitution_preserves_fields_and_budget_and_never_falls_back() {
    struct Observed(Mutex<usize>, u16);
    impl Http for Observed {
        fn send(
            &self,
            request: &Request,
            deadline: u64,
            cancel: &dyn Cancellation,
        ) -> Result<Response, AuthError> {
            *self.0.lock().unwrap() += 1;
            assert_eq!(
                request.url(),
                "http://127.0.0.1:8085/v1/projects/demo-proof/databases/(default)/documents:commit"
            );
            assert_eq!(request.operation(), Operation::Commit);
            assert_eq!(request.method(), "POST");
            assert_eq!(request.body(), b"sealed-original");
            assert_eq!(request.content_type(), "application/json");
            assert_eq!(request.bearer(), Some("secret-canary"));
            assert_eq!(request.response_bytes(), 37);
            assert_eq!(deadline, 91);
            assert!(
                cancel.cancelled(),
                "cancellation is passed through unchanged"
            );
            assert!(!format!("{request:?}").contains("secret-canary"));
            Ok(Response {
                status: self.1,
                body: b"bounded".to_vec(),
                retry_after_ms: Some(7),
            })
        }
    }
    let route = EmulatorRoute::new("demo-proof", "localhost:9099", "127.0.0.1:8085").unwrap();
    let request = Request::firestore(Operation::Commit,
        "https://firestore.googleapis.com/v1/projects/demo-proof/databases/(default)/documents:commit".into(),
        b"sealed-original".to_vec(), "secret-canary".into(), 37);
    let real = Observed(Mutex::new(0), 429);
    let reply = route
        .send(&real, &request, 91, &AtomicBool::new(true))
        .unwrap();
    assert_eq!(
        (reply.status, reply.body, reply.retry_after_ms),
        (429, b"bounded".to_vec(), Some(7))
    );
    assert_eq!(*real.0.lock().unwrap(), 1);
    let unknown = request.with_emulator_url("https://example.com/unknown".into());
    assert_eq!(
        route
            .send(&real, &unknown, 91, &AtomicBool::new(true))
            .unwrap_err()
            .kind,
        AuthErrorKind::InputInvalid
    );
    assert_eq!(
        *real.0.lock().unwrap(),
        1,
        "unknown URL must not reach any HTTP adapter"
    );
    let redirect = Observed(Mutex::new(0), 307);
    assert_eq!(
        route
            .send(&redirect, &request, 91, &AtomicBool::new(true))
            .unwrap_err()
            .kind,
        AuthErrorKind::InvalidReply
    );
    assert_eq!(
        *redirect.0.lock().unwrap(),
        1,
        "redirect is never followed or retried"
    );
}
