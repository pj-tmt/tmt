//! Native switch fixture: a real executable vnode, argv/tty and optional old
//! clock lease. It is not a historical Squad release or a board renderer.

use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

fn main() {
    let stop = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, stop.clone()).unwrap();
    let arguments: Vec<_> = std::env::args().collect();
    let proof =
        PathBuf::from(std::env::var_os("TMT_FORMER_PROOF").expect("test owns the proof path"));
    let clock = std::env::var_os("TMT_FORMER_CLOCK_ROOT")
        .map(|root| tmt_ops::cron::Clock::new(&PathBuf::from(root)).unwrap());
    let now = || jiff::Timestamp::now().as_millisecond();
    let mut lease = clock.as_ref().map(|clock| {
        clock
            .acquire(now(), std::process::id(), std::env::var("TMUX_PANE").ok())
            .unwrap()
            .expect("fixture owns old clock")
    });
    fs::write(&proof,json!({"pid":std::process::id(),"args":arguments,"cwd":std::env::current_dir().unwrap(),"pane":std::env::var("TMUX_PANE").ok()}).to_string()).unwrap();
    println!("Former board fixture ready");
    while !stop.load(Ordering::Acquire) {
        if let Some(lease) = lease.as_mut() {
            match lease.renew(now()) {
                Ok(_) => {}
                // The Ops path migration holds the clock lock while it examines the
                // lease. Like the production clock, wait for the next tick.
                Err(error) if error.code == "SQUAD_CRON_CLOCK_BUSY" => {}
                Err(error) => panic!("old clock renew failed: {error}"),
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    if let Some(lease) = lease {
        lease.release().unwrap();
    }
    fs::write(
        proof.with_extension("stopped"),
        b"TERM observed; lease released",
    )
    .unwrap();
}
