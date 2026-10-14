use super::{AuthErrorKind, ProductionHttp, Project, SystemClock};
use std::{process::Command, sync::Arc};

#[test]
fn production_constructor_is_selected_without_process_wide_env_changes() {
    let root =
        std::env::temp_dir().join(format!("remote-http-{}", crate::store::uuid_v4().unwrap()));
    std::fs::create_dir(&root).unwrap();
    for (case, auth, firestore) in [
        ("absent", None, None),
        ("active", Some("127.0.0.1:9099"), Some("127.0.0.1:8085")),
        ("real", Some("127.0.0.1:9099"), Some("127.0.0.1:8085")),
        ("partial", Some("127.0.0.1:9099"), None),
        ("unsafe", Some("example.com:9099"), Some("127.0.0.1:8085")),
    ] {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "firebase_auth::production_http_tests::constructor_child",
                "--nocapture",
            ])
            .env_clear()
            .env("HOME", &root)
            .env("XDG_CONFIG_HOME", &root)
            .env("REMOTE_HTTP_TEST_CASE", case);
        if let Some(value) = auth {
            child.env("FIREBASE_AUTH_EMULATOR_HOST", value);
        }
        if let Some(value) = firestore {
            child.env("FIRESTORE_EMULATOR_HOST", value);
        }
        let result = child.output().unwrap();
        assert!(
            result.status.success(),
            "case {case}: {} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn constructor_child() {
    let Ok(case) = std::env::var("REMOTE_HTTP_TEST_CASE") else {
        return;
    };
    let project = Project::new(
        if case == "real" {
            "real-proof"
        } else {
            "demo-proof"
        },
        "12345",
        "public",
    )
    .unwrap();
    let result = ProductionHttp::new(&project, Arc::new(SystemClock::default()));
    if cfg!(feature = "test-emulator") && matches!(case.as_str(), "real" | "partial" | "unsafe") {
        assert!(matches!(result, Err(e) if e.kind == AuthErrorKind::InputInvalid));
    } else {
        assert_eq!(
            result.unwrap().emulator_active(),
            cfg!(feature = "test-emulator") && case == "active"
        );
    }
}
