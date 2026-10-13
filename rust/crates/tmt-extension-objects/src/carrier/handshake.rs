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
//! tmt-object-channel: <1, 2 or 3>
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

/// Open a fresh stream for each attempt; a pre-reply failure permits one version-1
/// fallback within the same absolute setup budget. Nothing is cached or replayed.
pub fn negotiate(
    setup: Instant,
    mut open: impl FnMut() -> Result<(UnixStream, Offer), Fault>,
) -> Result<Link, Fault> {
    if Instant::now() >= setup {
        return Err(Fault::Timeout(Stage::Head));
    }
    for version in [2, 1] {
        let attempt = open()
            .map_err(|fault| (fault, false))
            .and_then(|(stream, offer)| initiate_version(stream, &offer, setup, version));
        match attempt {
            Ok(link) => return Ok(link),
            Err((fault, complete)) if complete || version == 1 || Instant::now() >= setup => {
                return Err(fault);
            }
            Err(_) => {}
        }
    }
    unreachable!("two attempts always return")
}

/// Native activation only: offer the highest version once. A fault never re-offers.
pub fn negotiate_sync(stream: UnixStream, offer: &Offer, setup: Instant) -> Result<Link, Fault> {
    initiate_version(stream, offer, setup, 3).map_err(|(fault, _)| fault)
}

/// Keep version-1 wire/refusal fixtures explicit without a public legacy entry point.
#[cfg(test)]
pub(super) fn version_one(
    stream: UnixStream,
    offer: &Offer,
    setup: Instant,
) -> Result<Link, Fault> {
    initiate_version(stream, offer, setup, 1).map_err(|(fault, _)| fault)
}

fn initiate_version(
    stream: UnixStream,
    offer: &Offer,
    setup: Instant,
    version: u8,
) -> Result<Link, (Fault, bool)> {
    if !plain(&offer.host) || !plain(&offer.mount) {
        return Err((Fault::Handshake(Refusal::BadValue("host")), true));
    }
    let (mut reader, mut writer) = halves(stream, Vec::new()).map_err(|fault| (fault, false))?;
    let request = format!(
        "GET {ROUTE} HTTP/1.1\r\nHost: {}\r\nConnection: Upgrade\r\nUpgrade: {PROTOCOL}\r\nContent-Length: 0\r\ntmt-mount: {}\r\ntmt-object-channel: {version}\r\ntmt-object-generation: {}\r\n\r\n",
        offer.host, offer.mount, offer.generation
    );
    writer
        .send_by(request.as_bytes(), setup, Stage::Reply)
        .map_err(|fault| (fault, false))?;
    let mut complete = false;
    let (head, first_bytes) = reader
        .head_observed(HEAD_BYTES, setup, &mut complete)
        .map_err(|fault| (fault, complete))?;
    let mut headers = [httparse::EMPTY_HEADER; HEAD_FIELDS];
    let mut reply = httparse::Response::new(&mut headers);
    let refused = |refusal| (Fault::Handshake(refusal), true);
    match reply.parse(&head) {
        Ok(httparse::Status::Complete(end)) if end == head.len() => {}
        _ => return Err(refused(Refusal::Head)),
    }
    if reply.version != Some(1) {
        return Err(refused(Refusal::Version));
    }
    // Legacy refusal precedence is part of the existing outcome contract.
    if version != 3 && (reply.code != Some(101) || reply.reason != Some("Switching Protocols")) {
        return Err(refused(Refusal::Status));
    }
    let selected_fields = [
        "connection",
        "upgrade",
        "tmt-object-channel",
        "tmt-object-generation",
    ];
    if version == 3 && reply.code == Some(426) {
        if reply.reason != Some("Upgrade Required") || !first_bytes.is_empty() {
            return Err(refused(Refusal::Status));
        }
        let names = [
            "tmt-object-channel-error",
            "tmt-object-generation",
            "content-length",
        ];
        let seen = fields(reply.headers, &names).map_err(refused)?;
        require(&seen, "tmt-object-channel-error", "unsupported-version").map_err(refused)?;
        require(&seen, "content-length", "0").map_err(refused)?;
        if generation(&seen).map_err(refused)? != offer.generation {
            return Err(refused(Refusal::Generation));
        }
        return Err(refused(Refusal::UnsupportedVersion));
    }
    let seen = fields(
        reply.headers,
        if version == 3 {
            &selected_fields
        } else {
            &REPLY_FIELDS
        },
    )
    .map_err(refused)?;
    let selected = if version == 3 {
        match value(&seen, "tmt-object-channel") {
            "1" => 1,
            "2" => 2,
            "3" => 3,
            _ => return Err(refused(Refusal::BadValue("tmt-object-channel"))),
        }
    } else {
        version
    };
    if reply.code != Some(101) || reply.reason != Some("Switching Protocols") {
        return Err(refused(Refusal::Status));
    }
    token(&seen, "connection", "upgrade").map_err(refused)?;
    require(&seen, "upgrade", PROTOCOL).map_err(refused)?;
    if generation(&seen).map_err(refused)? != offer.generation {
        return Err(refused(Refusal::Generation));
    }
    // The extension may start sending frames right after its reply.
    reader.set_version(selected);
    reader.keep(first_bytes);
    Ok(Link {
        generation: offer.generation,
        role: Role::Remote,
        version: selected,
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
    let (generation, version) = admitted(&head, &rest, expect).map_err(Fault::Handshake)?;
    replied(reader, writer, generation, version, budgets, setup)
}

/// Accept the channel as the extension for a caller whose router already read one
/// complete request `head` (through the blank line, within its own bound) and
/// dispatched on its path. `rest` is whatever the caller read after the head; it must be
/// empty. The head gets exactly the checks [`accept`] applies. A valid unsupported
/// version receives the typed refusal; malformed offers write nothing. The
/// head-read bound belongs to the caller.
pub fn accept_head(
    stream: UnixStream,
    head: &[u8],
    rest: &[u8],
    expect: &Expect,
    budgets: &Budgets,
    setup: Instant,
) -> Result<Link, Fault> {
    let (generation, version) = admitted(head, rest, expect).map_err(Fault::Handshake)?;
    let (reader, writer) = halves(stream, Vec::new())?;
    replied(reader, writer, generation, version, budgets, setup)
}

/// The one validation of an offered head, shared by both entry points: the generation
/// it offers, or why it is refused.
fn admitted(head: &[u8], rest: &[u8], expect: &Expect) -> Result<(Uuid4, u8), Refusal> {
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
    let version = match value(&seen, "tmt-object-channel") {
        "1" => 1,
        "2" => 2,
        "3" => 3,
        text => {
            let number = text
                .parse::<u8>()
                .map_err(|_| Refusal::BadValue("tmt-object-channel"))?;
            if number == 0 || number.to_string() != text {
                return Err(Refusal::BadValue("tmt-object-channel"));
            }
            number
        }
    };
    let generation = generation(&seen)?;
    if !rest.is_empty() {
        return Err(Refusal::Pipelined);
    }
    Ok((generation, version))
}

/// Write the one reply within its bound and hand the link over.
fn replied(
    mut reader: bounded::Reader,
    mut writer: bounded::Writer,
    generation: Uuid4,
    version: u8,
    budgets: &Budgets,
    setup: Instant,
) -> Result<Link, Fault> {
    if version > 3 {
        let reply = format!(
            "HTTP/1.1 426 Upgrade Required\r\ntmt-object-channel-error: unsupported-version\r\ntmt-object-generation: {generation}\r\nContent-Length: 0\r\n\r\n"
        );
        writer.send_by(
            reply.as_bytes(),
            within(Instant::now(), budgets.reply, setup),
            Stage::Reply,
        )?;
        return Err(Fault::Handshake(Refusal::UnsupportedVersion));
    }
    let selected = if version == 3 {
        "tmt-object-channel: 3\r\n"
    } else {
        ""
    };
    let reply = format!(
        "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: {PROTOCOL}\r\n{selected}tmt-object-generation: {generation}\r\n\r\n"
    );
    writer.send_by(
        reply.as_bytes(),
        within(Instant::now(), budgets.reply, setup),
        Stage::Reply,
    )?;
    reader.set_version(version);
    Ok(Link {
        generation,
        role: Role::Extension,
        version,
        reader,
        writer,
    })
}

#[cfg(test)]
mod tests;
