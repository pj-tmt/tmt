//! Standalone entrypoint for the independently released Herdr driver.

use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(tmt_driver_herdr::serve_call())
}
