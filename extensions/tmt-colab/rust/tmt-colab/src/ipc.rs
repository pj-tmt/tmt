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
        return Err(match read_reply(socket, limits::RESPONSE) {
            Ok((code, body)) => EarlyReply { code, body }.into(),
            Err(_) => error.into(),
        });
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
    socket.shutdown(std::net::Shutdown::Write)?;
    read_reply(socket, wait)
}
fn read_reply(mut socket: UnixStream, wait: std::time::Duration) -> Result<(u16, Vec<u8>)> {
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
            Err(_) => return Err(Fault::Unavailable.into()),
        };
        if n == 0 {
            break;
        }
        response.extend_from_slice(&chunk[..n]);
        if response.len() > limits::HEADER_BYTES + limits::HTTP_BODY_BYTES {
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
    for header in parsed.headers.iter() {
        if header.name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(Fault::Unavailable.into());
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
    Ok((code, response[end..].to_vec()))
}
