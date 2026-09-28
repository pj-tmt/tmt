//! One machine request per invocation; no shell, worker or persistent service.

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
        .and_then(|request| {
            // Discovery must not even discover paths or open a storage handle.
            if matches!(request, api::Request::Capabilities) {
                return Ok(api::capabilities());
            }
            let paths = ConfigPaths::discover().map_err(|_| api::Fault::unavailable())?;
            api::execute(&paths, request)
        });
    let (status, body) = match result {
        Ok(body) => (0, body),
        Err(error) => (1, error.encode()),
    };
    let mut stdout = io::stdout().lock();
    stdout.write_all(&body)?;
    writeln!(stdout)?;
    Ok(status)
}
