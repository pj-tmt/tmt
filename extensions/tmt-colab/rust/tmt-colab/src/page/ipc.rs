//! One bounded request to the existing owned serve socket. Never retries or falls back.
use super::{Fault, Prepared, Receipt};
use crate::{Result, keyring::Layout, limits};
use nix::poll::{PollFd, PollFlags, poll};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    os::fd::AsFd,
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    },
    time::Instant,
};
pub const PATH: &str = "/.tmt/colab/local/page-write";
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteError {
    error: Detail,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Detail {
    code: String,
    message: String,
}
impl WriteError {
    pub fn code(&self) -> &str {
        &self.error.code
    }
    pub(crate) fn status(&self) -> u16 {
        match self.code() {
            "COLAB_DENIED" => 403,
            "COLAB_STALE_BASE" | "COLAB_PAGE_INACTIVE" | "COLAB_STATE_MISSING" => 409,
            "COLAB_CAPACITY" => 413,
            "COLAB_INPUT_INVALID" => 400,
            _ => 503,
        }
    }
    pub(crate) fn from_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self {
            error: Detail {
                code: error
                    .downcast_ref::<Fault>()
                    .unwrap_or(&Fault::Unavailable)
                    .code()
                    .into(),
                message: error.to_string(),
            },
        }
    }
}
impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.error.message)
    }
}
impl std::error::Error for WriteError {}
pub fn write(layout: &Layout, prepared: &Prepared) -> Result<Receipt> {
    let body = serde_json::to_vec(prepared)?;
    if body.len() > limits::http_body_bytes(PATH) {
        return Err(Fault::Capacity.into());
    }
    let path = layout.directory.join(crate::socket::SOCKET);
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| Fault::Unavailable)?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(Fault::Unavailable.into());
    }
    let mut socket = UnixStream::connect(path).map_err(|_| Fault::Unavailable)?;
    socket.set_write_timeout(Some(limits::ACQUISITION))?;
    write!(
        socket,
        "POST {PATH} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )?;
    socket.write_all(&body)?;
    socket.shutdown(std::net::Shutdown::Write)?;
    let deadline = Instant::now() + limits::ACQUISITION + limits::RESPONSE;
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
    if parsed.code != Some(200) {
        let failure: WriteError =
            serde_json::from_slice(&response[end..]).map_err(|_| Fault::Unavailable)?;
        if ![
            Fault::StaleBase,
            Fault::Invalid,
            Fault::Capacity,
            Fault::Missing,
            Fault::Inactive,
            Fault::Denied,
            Fault::Unavailable,
        ]
        .iter()
        .any(|f| f.code() == failure.code())
        {
            return Err(Fault::Unavailable.into());
        }
        return Err(failure.into());
    }
    let receipt: Receipt =
        serde_json::from_slice(&response[end..]).map_err(|_| Fault::Unavailable)?;
    if receipt.space_id != prepared.space_id
        || receipt.page_id != prepared.page_id
        || receipt.epoch != prepared.epoch
    {
        return Err(Fault::Unavailable.into());
    }
    Ok(receipt)
}
