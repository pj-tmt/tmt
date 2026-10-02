//! `tmt-driver-herdr __tmt-driver <protocol> <op>`: one request on stdin,
//! one answer on stdout.

use std::process::ExitCode;
use tmt_driver_herdr::{HerdrDriver, run::Processes};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = tmt_driver_protocol::serve(
        &args,
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        &mut HerdrDriver::new(Processes),
    );
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
