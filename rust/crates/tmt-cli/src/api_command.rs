//! One machine request per invocation; no shell, worker or persistent service.

use crate::output::Failure;
use std::{
    io::{self, Write},
    time::Duration,
};
use tmt_adapters::{api, config::ConfigPaths, response_input};

pub fn execute() -> io::Result<u8> {
    let result = response_input::read_stdin_bounded(Duration::from_secs(5), api::INPUT_LIMIT)
        .map_err(|error| api::Fault::new(
            if error.kind == response_input::ResponseInputFailure::Timeout { "API_INPUT_TIMEOUT" } else { "API_INPUT_INVALID" },
            "Expected one bounded JSON request on non-terminal stdin, closed within five seconds.",
        ))
        .and_then(|body| api::decode(&body))
        .map_err(|fault| (fault.status(), fault.encode()))
        .and_then(|request| {
            // Discovery must not even discover paths or open a storage handle.
            if matches!(request, api::Request::Capabilities) {
                return Ok(api::capabilities());
            }
            let paths = ConfigPaths::discover().map_err(|_| (1, api::Fault::unavailable().encode()))?;
            api::execute(&paths, request).map_err(|mut fault| {
                if let Some(error) = fault.take_storage_open() {
                    let failure = Failure::storage_access(error, &paths.global_dir, "No API operation was performed.", fault.code(), fault.message());
                    (failure.status, serde_json::to_vec(&failure.document()).expect("bounded storage failure"))
                } else { (fault.status(), fault.encode()) }
            })
        });
    let (status, body) = match result {
        Ok(body) => (0, body),
        Err((status, body)) => (status, body),
    };
    let mut stdout = io::stdout().lock();
    stdout.write_all(&body)?;
    writeln!(stdout)?;
    Ok(status)
}
