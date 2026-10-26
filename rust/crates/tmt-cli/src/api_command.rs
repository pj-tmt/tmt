//! Invocation-owned API requests, with an explicit ordered stdio session mode.
use crate::output::Failure;
use std::{
    io::{self, Write},
    os::fd::AsFd,
    time::Duration,
};
use tmt_adapters::{api, config::ConfigPaths, framed_stdio, response_input};

struct Response {
    status: u8,
    body: Vec<u8>,
    stale: bool,
}
impl Response {
    fn fault(mut fault: api::Fault, paths: Option<&ConfigPaths>) -> Self {
        let stale = fault.code() == api::SESSION_STALE;
        let (status, body) = if let Some((error, effect)) = fault.take_storage_access() {
            let paths = paths.expect("storage faults have selected paths");
            let failure = Failure::storage_access(
                error,
                &paths.global_dir,
                effect,
                fault.code(),
                fault.message(),
            );
            (
                failure.status,
                serde_json::to_vec(&failure.document()).expect("bounded storage failure"),
            )
        } else {
            (fault.status(), fault.encode())
        };
        Self {
            status,
            body,
            stale,
        }
    }
}
fn call(body: &[u8], session: &mut Option<api::Session>) -> Response {
    let decoded = std::str::from_utf8(body)
        .map_err(|_| api::Fault::new("API_INPUT_INVALID", "Expected a UTF-8 JSON request."))
        .and_then(api::decode);
    let request = match decoded {
        Ok(request) => request,
        Err(fault) => {
            // Even invalid envelopes cannot keep an old-schema session alive.
            if let Some(active) = session
                && let Err(stale) = active.validate_schema()
            {
                return Response::fault(stale, Some(active.paths()));
            }
            return Response::fault(fault, None);
        }
    };
    // Discovery must not even select paths before the first non-constant request.
    if session.is_none() && matches!(request, api::Request::Capabilities) {
        return Response {
            status: 0,
            body: api::capabilities(),
            stale: false,
        };
    }
    if session.is_none() {
        match ConfigPaths::discover() {
            Ok(paths) => *session = Some(api::Session::new(paths)),
            Err(_) => return Response::fault(api::Fault::unavailable(), None),
        }
    }
    let session = session.as_mut().expect("selected paths");
    match session.execute(request) {
        Ok(body) => Response {
            status: 0,
            body,
            stale: false,
        },
        Err(fault) => Response::fault(fault, Some(session.paths())),
    }
}
fn close(session: Option<api::Session>) -> Result<(), api::Fault> {
    session.map_or(Ok(()), api::Session::close)
}
pub fn execute() -> io::Result<u8> {
    let mut session = None;
    let response = match response_input::read_stdin_bounded(
        Duration::from_secs(5),
        api::INPUT_LIMIT,
    ) {
        Ok(body) => call(body.as_bytes(), &mut session),
        Err(error) => Response::fault(
            api::Fault::new(
                if error.kind == response_input::ResponseInputFailure::Timeout {
                    "API_INPUT_TIMEOUT"
                } else {
                    "API_INPUT_INVALID"
                },
                "Expected one bounded JSON request on non-terminal stdin, closed within five seconds.",
            ),
            None,
        ),
    };
    let closed = close(session);
    let response = if response.status == 0 {
        match closed {
            Ok(()) => response,
            Err(fault) => Response::fault(fault, None),
        }
    } else {
        response
    };
    let mut stdout = io::stdout().lock();
    stdout.write_all(&response.body)?;
    writeln!(stdout)?;
    Ok(response.status)
}

fn serve_session(
    input: &impl AsFd,
    output: &impl AsFd,
    mut receive: impl FnMut(&[u8]) -> Response,
) -> io::Result<u8> {
    framed_stdio::require_redirected(input, output)?;
    let mut reader = framed_stdio::Reader::new(input, api::INPUT_LIMIT);
    while let Some(line) = reader.read_frame()? {
        let payload = line.strip_suffix(b"\n").expect("complete frame");
        let response = receive(payload);
        framed_stdio::publish(output, &response.body, api::OUTPUT_LIMIT)?;
        tmt_adapters::extension_hooks::deliver_pending();
        if response.stale {
            return Ok(1);
        }
    }
    Ok(0)
}
pub fn execute_session() -> io::Result<u8> {
    let mut session = None;
    let stdout = tmt_cli_style::stream::stdout(true).into_inner();
    let pending = serve_session(&io::stdin(), &stdout, |body| call(body, &mut session));
    let closed = close(session);
    let status = pending?;
    if status == 0 {
        closed.map_err(|fault| io::Error::other(fault.message().to_owned()))?;
    }
    Ok(status)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        os::unix::net::UnixStream,
        thread,
    };

    #[test]
    fn a_session_flushes_before_eof_and_ends_after_one_stale_response() {
        let (mut client, server) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let worker = thread::spawn(move || {
            let mut calls = 0;
            let result = serve_session(&server, &server, |_| {
                calls += 1;
                if calls == 1 {
                    Response {
                        status: 0,
                        body: br#"{"ready":true}"#.to_vec(),
                        stale: false,
                    }
                } else {
                    Response::fault(api::Fault::new(api::SESSION_STALE, "reopen"), None)
                }
            });
            (result, calls)
        });
        client.write_all(b"{}\n").unwrap();
        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "{\"ready\":true}\n");
        // Stdin is still open. The third pipelined request must never reach its owner.
        reader.get_mut().write_all(b"{}\n{}\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line).unwrap()["error"]["code"],
            api::SESSION_STALE
        );
        let (result, calls) = worker.join().unwrap();
        assert_eq!(result.unwrap(), 1);
        assert_eq!(calls, 2);
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {}
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
            other => panic!("session did not close: {other:?}"),
        }
    }
}
