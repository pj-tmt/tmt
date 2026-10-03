//! Fixtures shared by Squad's unit tests.

use std::path::Path;
use tmt_test_support::write_executable;

/// Prewarm a fixture without executing its payload. macOS first-exec assessment
/// queues fresh files under parallel load; keep it outside production deadlines.
/// Never rewrite the executable after this one bounded readiness probe.
pub(crate) fn write_ready_executable(path: &Path, script: &str) {
    let payload = script
        .strip_prefix("#!/bin/sh\n")
        .expect("shell fixture shebang");
    write_executable(
        path,
        format!("#!/bin/sh\nif [ \"$1\" = __tmt_fixture_ready ]; then exit 0; fi\n{payload}")
            .as_bytes(),
        0o755,
    )
    .expect("publish shell fixture");
    let ready = probe(path, std::time::Duration::from_secs(30))
        .expect("fixture readiness completes within 30 seconds");
    assert!(
        ready.success,
        "fixture readiness failed: {}",
        path.display()
    );
    assert!(
        ready.stdout.is_empty(),
        "fixture readiness has no payload output"
    );
}

fn probe(
    path: &Path,
    ceiling: std::time::Duration,
) -> Result<crate::runner::Finished, crate::runner::RunError> {
    crate::runner::run(path, &["__tmt_fixture_ready".into()], b"", ceiling, 1024)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn readiness_has_no_payload_effects_and_a_stalled_probe_is_bounded() {
        let dir = std::env::temp_dir().join(format!(
            "tmt-squad-ready-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("fixture");
        let marker = dir.join("called");
        write_ready_executable(
            &file,
            &format!(
                "#!/bin/sh\nprintf called > '{}'\nprintf payload\n",
                marker.display()
            ),
        );
        assert!(!marker.exists());
        let output = crate::runner::run(&file, &[], b"", Duration::from_secs(30), 1024).unwrap();
        assert!(output.success);
        assert_eq!(output.stdout, b"payload");
        assert_eq!(std::fs::read(&marker).unwrap(), b"called");
        let stalled = dir.join("stalled");
        write_executable(&stalled, b"#!/bin/sh\nexec /bin/sleep 30\n", 0o755).unwrap();
        assert!(matches!(
            probe(&stalled, Duration::from_millis(100)),
            Err(crate::runner::RunError::Timeout)
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
