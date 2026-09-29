use std::{
    io::{self, Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

/// Serve one bounded plain-HTTP response and return the request headers seen.
///
/// This is shared by discovery and remote transport tests so they exercise the
/// same one-shot listener/read/write behavior without introducing a second
/// server implementation.
pub(crate) fn with_http_response<T>(
    response: Vec<u8>,
    operation: impl FnOnce(String) -> T,
) -> (T, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::scope(|scope| {
        let server = scope.spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                assert!(request.len() < 16 * 1024);
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            // Drain a bounded POST body before closing the socket. Closing with
            // unread request bytes can reset TCP and disguise a valid response
            // as the truncation failure that another scenario intends to test.
            let headers = String::from_utf8(request).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            assert!(length <= 16 * 1024);
            stream.read_exact(&mut vec![0; length]).unwrap();
            stream.write_all(&response).unwrap();
            headers
        });
        let result = operation(origin);
        (result, server.join().unwrap())
    })
}
