//! A worker for the launcher tests: it adopts the private endpoint like a product's worker and
//! then behaves as its first argument says. Its second argument is a directory that receives
//! `pid` at start and `serving` once the handoff was accepted.
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tmt_extension_serve::{Handshake, StartupError, Timing, adopt};

fn main() {
    let mut arguments = std::env::args().skip(1);
    let mode = arguments.next().unwrap_or_default();
    let dir = PathBuf::from(arguments.next().expect("directory"));
    let handshake = Handshake {
        unconfirmed: StartupError::new("FIXTURE_UNCONFIRMED", "The fixture could not confirm."),
        cancelled: StartupError::new("FIXTURE_CANCELLED", "The fixture was cancelled."),
        timing: Timing {
            startup: Duration::from_secs(10),
            cleanup_wait: Duration::from_secs(10),
            record_bytes: 4096,
        },
    };
    let stop = Arc::new(AtomicBool::new(false));
    let mut handoff = adopt(&handshake, &stop).expect("adopt");
    fs::write(dir.join("pid"), std::process::id().to_string()).unwrap();
    match mode.as_str() {
        "ready" => {
            match handoff.ready(&json!({"state":"ready"})) {
                Ok(()) => fs::write(dir.join("serving"), "").unwrap(),
                // Cancelled before the accept: say so, with the cleanup confirmed.
                Err(error) => handoff.fail(&error, true),
            }
        }
        "fail" => handoff.fail(
            &StartupError::new("FIXTURE_FAILED", "The fixture failed.").with_hint("Try again."),
            true,
        ),
        // Never ready: the launcher's cancel or a lost launcher must unwind this worker.
        "silent" => {
            let until = Instant::now() + Duration::from_secs(20);
            while !stop.load(Ordering::SeqCst) && Instant::now() < until {
                thread::sleep(Duration::from_millis(10));
            }
            handoff.fail(&handshake.cancelled, true);
        }
        // Ignores every cancel: only the launcher's own-child kill can end it.
        "stubborn" => thread::sleep(Duration::from_secs(60)),
        other => panic!("unknown mode {other}"),
    }
}
