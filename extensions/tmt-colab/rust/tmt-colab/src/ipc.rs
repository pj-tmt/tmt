//! One bounded request from a root-local command to the serve socket of this data root.
//! Never retries or falls back; any doubt about the exchange is `Unavailable`.
use crate::{Result, keyring::Layout, limits, page::Fault};
use nix::poll::{PollFd, PollFlags, poll};
use std::{
    io::{Read, Write},
    os::fd::AsFd,
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    },
    time::Instant,
};

/// POST `body` to `path` on the owner-only socket and return the status and the framed body.
pub(crate) fn exchange(layout: &Layout, path: &str, body: &[u8]) -> Result<(u16, Vec<u8>)> {
    receive(
        send(layout, path, body)?,
        limits::ACQUISITION + limits::RESPONSE,
    )
}
/// Connect and write the whole request. An error here means the server cannot have acted: it
/// acts only on a complete body.
pub(crate) fn send(layout: &Layout, path: &str, body: &[u8]) -> Result<UnixStream> {
    let socket_path = layout.directory.join(crate::socket::SOCKET);
    let metadata = std::fs::symlink_metadata(&socket_path).map_err(|_| Fault::Unavailable)?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(Fault::Unavailable.into());
    }
    let mut socket = UnixStream::connect(socket_path).map_err(|_| Fault::Unavailable)?;
    socket.set_write_timeout(Some(limits::ACQUISITION))?;
    write!(
        socket,
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )?;
    if let Err(error) = socket.write_all(body) {
        // A server may refuse a request it will not read (a body over its cap) and close. The
        // refusal is then already waiting; without it the failure is only a failed send.
        return Err(
            match read_reply(socket, limits::RESPONSE, limits::HTTP_BODY_BYTES) {
                Ok(Reply { code, body, .. }) => EarlyReply { code, body }.into(),
                Err(_) => error.into(),
            },
        );
    }
    Ok(socket)
}
/// The server's answer, received while the request body was still being sent: it refused the
/// request before reading it all, so it cannot have acted on it.
#[derive(Debug)]
pub(crate) struct EarlyReply {
    pub code: u16,
    pub body: Vec<u8>,
}
impl std::fmt::Display for EarlyReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "The server refused the request with status {}.",
            self.code
        )
    }
}
impl std::error::Error for EarlyReply {}
/// Finish the request and read one framed reply within `wait`. Any error here leaves the
/// outcome in doubt.
pub(crate) fn receive(socket: UnixStream, wait: std::time::Duration) -> Result<(u16, Vec<u8>)> {
    let Reply { code, body, .. } = receive_up_to(socket, wait, limits::HTTP_BODY_BYTES)?;
    Ok((code, body))
}
/// One framed reply: its status, its body and the `Content-Type` the server named, if any.
pub(crate) struct Reply {
    pub code: u16,
    pub body: Vec<u8>,
    pub content_type: Option<String>,
}
/// `receive` for a route whose reply body may reach `body_cap` bytes.
pub(crate) fn receive_up_to(
    socket: UnixStream,
    wait: std::time::Duration,
    body_cap: usize,
) -> Result<Reply> {
    match socket.shutdown(std::net::Shutdown::Write) {
        Ok(()) => {}
        // A server that already answered and closed leaves nothing to half-close (macOS says
        // so); its reply is still waiting to be read.
        Err(error) if error.kind() == std::io::ErrorKind::NotConnected => {}
        Err(error) => return Err(error.into()),
    }
    read_reply(socket, wait, body_cap)
}
fn read_reply(mut socket: UnixStream, wait: std::time::Duration, body_cap: usize) -> Result<Reply> {
    let deadline = Instant::now() + wait;
    let mut response = Vec::new();
    let mut chunk = [0; 4096];
    socket.set_nonblocking(true)?;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or(Fault::Unavailable)?;
        let n = match socket.read(&mut chunk) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let mut fds = [PollFd::new(socket.as_fd(), PollFlags::POLLIN)];
                match poll(&mut fds, remaining.as_millis().min(u16::MAX as u128) as u16) {
                    Ok(0) => return Err(Fault::Unavailable.into()),
                    Ok(_) | Err(nix::errno::Errno::EINTR) => continue,
                    Err(_) => return Err(Fault::Unavailable.into()),
                }
            }
            // A peer that closed with request bytes unread resets the connection (Linux), and the
            // error follows the reply it already sent. Whatever was read is judged below: a
            // complete, exactly framed reply stands, a truncated one is refused.
            Err(_) => break,
        };
        if n == 0 {
            break;
        }
        response.extend_from_slice(&chunk[..n]);
        if response.len() > limits::HEADER_BYTES + body_cap {
            return Err(Fault::Unavailable.into());
        }
    }
    let mut headers = [httparse::EMPTY_HEADER; limits::HEADER_FIELDS];
    let mut parsed = httparse::Response::new(&mut headers);
    let httparse::Status::Complete(end) =
        parsed.parse(&response).map_err(|_| Fault::Unavailable)?
    else {
        return Err(Fault::Unavailable.into());
    };
    let mut size = None;
    let mut content_type = None;
    for header in parsed.headers.iter() {
        if header.name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(Fault::Unavailable.into());
        }
        if header.name.eq_ignore_ascii_case("content-type") {
            content_type = Some(std::str::from_utf8(header.value)?.to_owned());
        }
        if header.name.eq_ignore_ascii_case("content-length") {
            if size.is_some() {
                return Err(Fault::Unavailable.into());
            }
            size = Some(std::str::from_utf8(header.value)?.parse::<usize>()?);
        }
    }
    if end > limits::HEADER_BYTES || size != Some(response.len() - end) {
        return Err(Fault::Unavailable.into());
    }
    let code = parsed.code.ok_or(Fault::Unavailable)?;
    Ok(Reply {
        code,
        body: response[end..].to_vec(),
        content_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const REPLY: &[u8] = b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 9\r\nConnection: close\r\n\r\nTOO LARGE";

    /// The server answers and closes before the writer half-closes: macOS refuses that shutdown
    /// with `NotConnected`, which used to discard the waiting reply.
    #[test]
    fn a_reply_already_sent_and_closed_is_still_read() {
        let (client, mut server) = UnixStream::pair().unwrap();
        server.write_all(REPLY).unwrap();
        drop(server);
        let (code, body) = receive(client, Duration::from_secs(5)).unwrap();
        assert_eq!((code, body.as_slice()), (413, b"TOO LARGE".as_slice()));
    }

    /// The reply's `Content-Type` is handed to the caller, which is how a read learns the
    /// verified media type; a reply that names none leaves `None`.
    #[test]
    fn a_reply_reports_the_content_type_it_names() {
        for (head, expected) in [
            ("Content-Type: image/png\r\n", Some("image/png")),
            ("", None),
        ] {
            let (client, mut server) = UnixStream::pair().unwrap();
            server
                .write_all(
                    format!("HTTP/1.1 200 OK\r\n{head}Content-Length: 2\r\n\r\nhi").as_bytes(),
                )
                .unwrap();
            drop(server);
            let reply = receive_up_to(client, Duration::from_secs(5), 16).unwrap();
            assert_eq!(reply.content_type.as_deref(), expected);
            assert_eq!(reply.body, b"hi");
        }
    }

    /// The server refuses a request it did not read to the end: closing with request bytes
    /// unread resets the connection on Linux, and the reset used to turn the reply it had sent
    /// into a failed read.
    #[test]
    fn a_reply_followed_by_a_reset_from_unread_request_bytes_is_still_read() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        client.write_all(&[7; 4096]).unwrap();
        server.write_all(REPLY).unwrap();
        drop(server);
        let Reply { code, body, .. } =
            read_reply(client, Duration::from_secs(5), limits::HTTP_BODY_BYTES).unwrap();
        assert_eq!((code, body.as_slice()), (413, b"TOO LARGE".as_slice()));
    }

    /// Only a reply that is complete by its own length stands: a reset or close mid-reply, or
    /// before any reply, is a failed read.
    #[test]
    fn a_truncated_or_missing_reply_is_still_a_failed_read() {
        for sent in [&REPLY[..REPLY.len() - 1], &REPLY[..20], b"".as_slice()] {
            let (mut client, mut server) = UnixStream::pair().unwrap();
            client.write_all(&[7; 4096]).unwrap();
            server.write_all(sent).unwrap();
            drop(server);
            assert!(receive(client, Duration::from_secs(5)).is_err(), "{sent:?}");
        }
    }
}
