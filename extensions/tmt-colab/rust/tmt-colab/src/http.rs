//! Bounded loopback HTTP workers. L2a denies every API and WebSocket upgrade.
use crate::{Result, limits};
use nix::poll::{PollFd, PollFlags, poll};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::{Ipv4Addr, Shutdown, TcpListener, TcpStream},
    os::fd::AsFd,
    sync::atomic::{AtomicBool, Ordering},
    thread::{self, JoinHandle},
    time::Instant,
};

pub struct Door {
    listener: TcpListener,
    pub address: String,
    host: String,
    origin: String,
}
struct Worker {
    socket: TcpStream,
    handle: JoinHandle<()>,
}
impl Door {
    pub fn bind(port: u16) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AddrInUse {
                std::io::Error::new(
                    error.kind(),
                    format!("Loopback port {port} is busy; choose another with --port, or use --port 0 for a free port."),
                )
            } else {
                error
            }
        })?;
        listener.set_nonblocking(true)?;
        let host = listener.local_addr()?.to_string();
        let origin = format!("http://{host}");
        Ok(Self {
            listener,
            address: format!("{origin}/"),
            host,
            origin,
        })
    }
    pub fn run(self, stop: &AtomicBool) -> Result<()> {
        let mut workers: Vec<Worker> = Vec::new();
        let result = (|| -> Result<()> {
            while !stop.load(Ordering::Acquire) {
                for i in (0..workers.len()).rev() {
                    if workers[i].handle.is_finished() {
                        workers
                            .swap_remove(i)
                            .handle
                            .join()
                            .map_err(|_| "HTTP worker panicked.")?;
                    }
                }
                let mut events = [PollFd::new(self.listener.as_fd(), PollFlags::POLLIN)];
                match poll(&mut events, 100u16) {
                    Ok(_) => {}
                    Err(nix::errno::Errno::EINTR) => continue,
                    Err(e) => return Err(e.into()),
                }
                for _ in 0..limits::SOCKETS {
                    let (mut socket, _) = match self.listener.accept() {
                        Ok(c) => c,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(e.into()),
                    };
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    socket.set_nonblocking(false)?;
                    if workers.len() == limits::SOCKETS {
                        socket.set_nonblocking(true)?;
                        let _ = response(&mut socket, 429, b"CAPACITY", false);
                        continue;
                    }
                    let retained = socket.try_clone()?;
                    let host = self.host.clone();
                    let origin = self.origin.clone();
                    let handle = thread::Builder::new().name("colab-http".into()).spawn(move || {
                        let result = acquire(&mut socket,&host,&origin);
                        let (status,body,html) = match result {
                            Ok(true) => (200,b"<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>TMT Colab</title><h1>TMT Colab</h1><p>Local space is running. Sign-in and sync are not available in this pilot slice.</p></html>".as_slice(),true),
                            Ok(false) => (403,b"DENIED".as_slice(),false),
                            Err(status) => (status,b"INVALID".as_slice(),false),
                        };
                        let _ = response(&mut socket,status,body,html);
                    })?;
                    workers.push(Worker {
                        socket: retained,
                        handle,
                    });
                }
            }
            Ok(())
        })();
        drop(self.listener);
        // Close retained handles before joining, interrupting blocked reads/writes.
        for worker in &workers {
            let _ = worker.socket.shutdown(Shutdown::Both);
        }
        let mut panicked = false;
        for worker in workers {
            panicked |= worker.handle.join().is_err();
        }
        if panicked {
            return Err("HTTP worker cleanup failed.".into());
        }
        result
    }
}
fn response(socket: &mut TcpStream, status: u16, body: &[u8], html: bool) -> std::io::Result<()> {
    let deadline = Instant::now() + limits::RESPONSE;
    let kind = if html {
        "text/html; charset=utf-8"
    } else {
        "text/plain; charset=utf-8"
    };
    let bytes = format!(
        "HTTP/1.1 {status} Response\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; base-uri 'none'; frame-ancestors 'none'\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len()
    );
    for mut bytes in [bytes.as_bytes(), body] {
        while !bytes.is_empty() {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
                .ok_or(std::io::ErrorKind::TimedOut)?;
            socket.set_write_timeout(Some(remaining))?;
            match socket.write(bytes) {
                Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
                Ok(n) => bytes = &bytes[n..],
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }
    socket.shutdown(Shutdown::Write)?;
    // Discard only immediately available bounded request bytes after the response.
    // Closing with unread input may reset even a fully written capacity reply.
    socket.set_nonblocking(true)?;
    let mut discarded = [0; 1024];
    for _ in 0..(limits::HEADER_BYTES + limits::HTTP_BODY_BYTES) / discarded.len() {
        match socket.read(&mut discarded) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
    Ok(())
}
/// One request only; no pipelining, forwarded authority or HTTP transfer encoding.
fn acquire(socket: &mut TcpStream, host: &str, origin: &str) -> std::result::Result<bool, u16> {
    let deadline = Instant::now() + limits::ACQUISITION;
    let mut bytes = Vec::new();
    let mut chunk = [0; 1024];
    let end = loop {
        if bytes.len() >= limits::HEADER_BYTES {
            return Err(413);
        }
        read(socket, &mut bytes, &mut chunk, deadline)?;
        if let Some(p) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
    };
    if end > limits::HEADER_BYTES {
        return Err(413);
    }
    if bytes[..end].iter().enumerate().any(|(i, b)| {
        (*b == b'\n' && (i == 0 || bytes[i - 1] != b'\r'))
            || (*b == b'\r' && bytes.get(i + 1) != Some(&b'\n'))
    }) {
        return Err(400);
    }
    let mut fields = [httparse::EMPTY_HEADER; limits::HEADER_FIELDS];
    let mut request = httparse::Request::new(&mut fields);
    if request.parse(&bytes[..end]).map_err(|_| 400u16)? != httparse::Status::Complete(end)
        || request.version != Some(1)
    {
        return Err(400);
    }
    let mut seen = BTreeSet::new();
    let mut actual_host = None;
    let mut actual_origin = None;
    let mut size = 0;
    let mut upgrade = false;
    for header in request.headers.iter() {
        let name = header.name.to_ascii_lowercase();
        if !seen.insert(name.clone())
            || [
                "transfer-encoding",
                "forwarded",
                "x-forwarded-host",
                "x-forwarded-proto",
            ]
            .contains(&name.as_str())
        {
            return Err(400);
        }
        let value = std::str::from_utf8(header.value).map_err(|_| 400u16)?;
        match name.as_str() {
            "host" => actual_host = Some(value),
            "origin" => actual_origin = Some(value),
            "upgrade" => upgrade = true,
            "connection" => {
                upgrade |= value
                    .split(',')
                    .any(|v| v.trim().eq_ignore_ascii_case("upgrade"))
            }
            "content-length" => {
                if value.is_empty()
                    || !value.bytes().all(|b| b.is_ascii_digit())
                    || (value.len() > 1 && value.starts_with('0'))
                {
                    return Err(400);
                }
                size = value.parse::<usize>().map_err(|_| 413u16)?;
                if size > limits::HTTP_BODY_BYTES {
                    return Err(413);
                }
            }
            _ => {}
        }
    }
    if actual_host != Some(host) {
        return Err(403);
    }
    let path = request.path.ok_or(400u16)?;
    if !path.starts_with('/') || path.contains(['?', '#', '%']) {
        return Err(400);
    }
    let placeholder = request.method == Some("GET") && path == "/" && !upgrade;
    if (!placeholder || actual_origin.is_some()) && actual_origin != Some(origin) {
        return Err(403);
    }
    if placeholder && size != 0 {
        return Err(400);
    }
    // Drop header borrows before acquiring the bounded body; no body is interpreted.
    while bytes.len() < end + size {
        read(socket, &mut bytes, &mut chunk, deadline)?;
    }
    if bytes.len() != end + size {
        return Err(400);
    }
    Ok(placeholder)
}
fn read(
    socket: &mut TcpStream,
    bytes: &mut Vec<u8>,
    chunk: &mut [u8],
    deadline: Instant,
) -> std::result::Result<(), u16> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or(408u16)?;
    socket
        .set_read_timeout(Some(remaining))
        .map_err(|_| 400u16)?;
    let n = socket.read(chunk).map_err(|_| 408u16)?;
    if n == 0 {
        return Err(400);
    }
    bytes.extend_from_slice(&chunk[..n]);
    Ok(())
}
