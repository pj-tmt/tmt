//! `tmt-driver-herdr __tmt-driver <protocol> <op>`: one request on stdin,
//! one answer on stdout. The first-party Herdr driver ships beside `tmt` in
//! the CLI archive; its behavior is the `tmt_driver_herdr` library.

use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(tmt_driver_herdr::serve_call())
}
