//! The route-owned upgrade. Remote offers it on an extension's owner-only socket;
//! the extension accepts it. Both ends parse strictly: exactly the fields below, each
//! once, with the exact values, and nothing pipelined before the reply.
//!
//! ```text
//! GET /.tmt/remote/object-channel-v1 HTTP/1.1      HTTP/1.1 101 Switching Protocols
//! Host: <the admitted mount host>                   Connection: Upgrade
//! Connection: Upgrade                               Upgrade: tmt-object-channel-v1
//! Upgrade: tmt-object-channel-v1                    tmt-object-generation: <same>
//! Content-Length: 0
//! tmt-mount: <the selected mount>
//! tmt-object-channel: 1
//! tmt-object-generation: <lowercase UUIDv4>
//! ```
use super::{Budgets, Fault, Link, PROTOCOL, ROUTE, Refusal, Role, Stage, bounded, halves};
use crate::Uuid4;
use std::{
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};

/// Longest head and most fields either end reads.
const HEAD_BYTES: usize = 8 * 1024;
const HEAD_FIELDS: usize = 32;
/// Longest `Host` or mount value an offer may carry.
const VALUE_BYTES: usize = 255;
const REQUEST_FIELDS: [&str; 7] = [
    "host",
    "connection",
    "upgrade",
    "content-length",
    "tmt-mount",
    "tmt-object-channel",
    "tmt-object-generation",
];
const REPLY_FIELDS: [&str; 3] = ["connection", "upgrade", "tmt-object-generation"];

/// What the initiator offers: the mount it selected and a generation it just made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub host: String,
    pub mount: String,
    pub generation: Uuid4,
}
/// What the acceptor requires of an offer. The generation is the offer's to choose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expect {
    pub host: String,
    pub mount: String,
}

fn within(now: Instant, budget: Duration, outer: Instant) -> Instant {
    (now + budget).min(outer)
}

/// A header value that can sit in one line without ambiguity.
fn plain(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= VALUE_BYTES
        && value
            .bytes()
            .all(|byte| byte == b' ' || byte.is_ascii_graphic())
        && value.trim() == value
}

/// The header fields of a parsed head as `(lowercase name, value)`, every one of
/// `allowed` present exactly once and nothing else.
fn fields<'a>(
    headers: &'a [httparse::Header<'a>],
    allowed: &[&'static str],
) -> Result<Vec<(&'static str, &'a str)>, Refusal> {
    let mut seen: Vec<(&'static str, &str)> = Vec::new();
    for header in headers {
        let name = header.name.to_ascii_lowercase();
        let Some(&known) = allowed.iter().find(|&&candidate| candidate == name) else {
            return Err(Refusal::UnknownHeader);
        };
        if seen.iter().any(|(have, _)| *have == known) {
            return Err(Refusal::DuplicateHeader);
        }
        let value = std::str::from_utf8(header.value).map_err(|_| Refusal::BadValue(known))?;
        seen.push((known, value));
    }
    if let Some(&missing) = allowed
        .iter()
        .find(|&&name| !seen.iter().any(|(have, _)| *have == name))
    {
        return Err(Refusal::MissingHeader(missing));
    }
    Ok(seen)
}
fn value<'a>(fields: &[(&'static str, &'a str)], name: &str) -> &'a str {
    fields
        .iter()
        .find(|(have, _)| *have == name)
        .map_or("", |(_, value)| value)
}
fn require(
    fields: &[(&'static str, &str)],
    name: &'static str,
    expected: &str,
) -> Result<(), Refusal> {
    (value(fields, name) == expected)
        .then_some(())
        .ok_or(Refusal::BadValue(name))
}
fn token(
    fields: &[(&'static str, &str)],
    name: &'static str,
    expected: &str,
) -> Result<(), Refusal> {
    value(fields, name)
        .eq_ignore_ascii_case(expected)
        .then_some(())
        .ok_or(Refusal::BadValue(name))
}
fn generation(fields: &[(&'static str, &str)]) -> Result<Uuid4, Refusal> {
    Uuid4::parse(value(fields, "tmt-object-generation"))
        .map_err(|_| Refusal::BadValue("tmt-object-generation"))
}

/// Open the channel as the initiator on `stream`, which is connected to the
/// extension's socket. `setup` is the absolute end of the whole setup.
pub fn initiate(stream: UnixStream, offer: &Offer, setup: Instant) -> Result<Link, Fault> {
    if !plain(&offer.host) || !plain(&offer.mount) {
        return Err(Fault::Handshake(Refusal::BadValue("host")));
    }
    let (mut reader, mut writer) = halves(stream, Vec::new())?;
    let request = format!(
        "GET {ROUTE} HTTP/1.1\r\nHost: {}\r\nConnection: Upgrade\r\nUpgrade: {PROTOCOL}\r\nContent-Length: 0\r\ntmt-mount: {}\r\ntmt-object-channel: 1\r\ntmt-object-generation: {}\r\n\r\n",
        offer.host, offer.mount, offer.generation
    );
    writer.send_by(request.as_bytes(), setup, Stage::Reply)?;
    let (head, first_bytes) = reader.head(HEAD_BYTES, setup)?;
    let mut headers = [httparse::EMPTY_HEADER; HEAD_FIELDS];
    let mut reply = httparse::Response::new(&mut headers);
    let refused = |refusal| Fault::Handshake(refusal);
    match reply.parse(&head) {
        Ok(httparse::Status::Complete(end)) if end == head.len() => {}
        _ => return Err(refused(Refusal::Head)),
    }
    if reply.version != Some(1) {
        return Err(refused(Refusal::Version));
    }
    if reply.code != Some(101) || reply.reason != Some("Switching Protocols") {
        return Err(refused(Refusal::Status));
    }
    let seen = fields(reply.headers, &REPLY_FIELDS).map_err(refused)?;
    token(&seen, "connection", "upgrade").map_err(refused)?;
    require(&seen, "upgrade", PROTOCOL).map_err(refused)?;
    if generation(&seen).map_err(refused)? != offer.generation {
        return Err(refused(Refusal::Generation));
    }
    // The extension may start sending frames right after its reply.
    reader.keep(first_bytes);
    Ok(Link {
        generation: offer.generation,
        role: Role::Remote,
        reader,
        writer,
    })
}

/// Accept the channel as the extension on `stream`, an accepted connection: read one
/// head within the head bound, admit it with [`accept_head`]'s checks and reply.
pub fn accept(
    stream: UnixStream,
    expect: &Expect,
    budgets: &Budgets,
    setup: Instant,
) -> Result<Link, Fault> {
    let (mut reader, writer) = halves(stream, Vec::new())?;
    let (head, rest) = reader.head(HEAD_BYTES, within(Instant::now(), budgets.head, setup))?;
    let generation = admitted(&head, &rest, expect).map_err(Fault::Handshake)?;
    replied(reader, writer, generation, budgets, setup)
}

/// Accept the channel as the extension for a caller whose router already read one
/// complete request `head` (through the blank line, within its own bound) and
/// dispatched on its path. `rest` is whatever the caller read after the head; it must be
/// empty. The head gets exactly the checks [`accept`] applies and nothing is written
/// to `stream` on a refusal; the head-read bound belongs to the caller.
pub fn accept_head(
    stream: UnixStream,
    head: &[u8],
    rest: &[u8],
    expect: &Expect,
    budgets: &Budgets,
    setup: Instant,
) -> Result<Link, Fault> {
    let generation = admitted(head, rest, expect).map_err(Fault::Handshake)?;
    let (reader, writer) = halves(stream, Vec::new())?;
    replied(reader, writer, generation, budgets, setup)
}

/// The one validation of an offered head, shared by both entry points: the generation
/// it offers, or why it is refused.
fn admitted(head: &[u8], rest: &[u8], expect: &Expect) -> Result<Uuid4, Refusal> {
    if head.len() > HEAD_BYTES {
        return Err(Refusal::Head);
    }
    let mut headers = [httparse::EMPTY_HEADER; HEAD_FIELDS];
    let mut request = httparse::Request::new(&mut headers);
    match request.parse(head) {
        Ok(httparse::Status::Complete(end)) if end == head.len() => {}
        _ => return Err(Refusal::Head),
    }
    if request.method != Some("GET") {
        return Err(Refusal::Method);
    }
    if request.path != Some(ROUTE) {
        return Err(Refusal::Path);
    }
    if request.version != Some(1) {
        return Err(Refusal::Version);
    }
    let seen = fields(request.headers, &REQUEST_FIELDS)?;
    require(&seen, "host", &expect.host)?;
    token(&seen, "connection", "upgrade")?;
    require(&seen, "upgrade", PROTOCOL)?;
    require(&seen, "content-length", "0")?;
    require(&seen, "tmt-mount", &expect.mount)?;
    require(&seen, "tmt-object-channel", "1")?;
    let generation = generation(&seen)?;
    if !rest.is_empty() {
        return Err(Refusal::Pipelined);
    }
    Ok(generation)
}

/// Write the one reply within its bound and hand the link over.
fn replied(
    reader: bounded::Reader,
    mut writer: bounded::Writer,
    generation: Uuid4,
    budgets: &Budgets,
    setup: Instant,
) -> Result<Link, Fault> {
    let reply = format!(
        "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: {PROTOCOL}\r\ntmt-object-generation: {generation}\r\n\r\n"
    );
    writer.send_by(
        reply.as_bytes(),
        within(Instant::now(), budgets.reply, setup),
        Stage::Reply,
    )?;
    Ok(Link {
        generation,
        role: Role::Extension,
        reader,
        writer,
    })
}

#[cfg(test)]
mod tests;
