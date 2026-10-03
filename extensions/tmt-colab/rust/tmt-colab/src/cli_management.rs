//! CLI adaptation to the existing root-local service; no transition policy here.
use clap::ArgMatches;
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    os::{
        fd::AsFd,
        unix::{
            fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
            net::UnixStream,
        },
    },
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tmt_colab::{
    Result, inspection,
    keyring::{Keyring, Layout, StateFault},
    management,
    registration::Registration,
    store::Store,
};
use tmt_colab_model::values;

#[derive(Debug)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
    pub correlation: Value,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Failure {}
fn fail(code: &'static str, message: &str) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(Failure {
        code,
        message: message.into(),
        correlation: json!({}),
    })
}
fn input(message: &str) -> Box<dyn std::error::Error + Send + Sync> {
    fail("COLAB_INPUT_INVALID", message)
}
fn text<'a>(args: &'a ArgMatches, name: &str) -> &'a str {
    args.get_one::<String>(name)
        .expect("required typed argument")
}
fn uuid(value: &str) -> Result<String> {
    values::generated_id(value).map_err(|_| input("Expected a canonical non-nil UUIDv4."))?;
    Ok(value.into())
}
fn fresh_id() -> Result<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|_| input("Could not generate operation identity."))?;
    b[6] = (b[6] & 15) | 64;
    b[8] = (b[8] & 63) | 128;
    let h: String = b.iter().map(|v| format!("{v:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}
fn source(path: &str, secret: bool) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if path == "-" {
        let stdin = std::io::stdin();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| input("Input deadline exceeded."))?;
            let ms = u16::try_from(remaining.as_millis())
                .unwrap_or(u16::MAX)
                .max(1);
            let mut fds = [nix::poll::PollFd::new(
                stdin.as_fd(),
                nix::poll::PollFlags::POLLIN,
            )];
            if nix::poll::poll(&mut fds, ms)? == 0 {
                return Err(input("Input deadline exceeded."));
            }
            let mut buffer = [0; 4096];
            let n = nix::unistd::read(&stdin, &mut buffer)?;
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..n]);
            if bytes.len() > 16 * 1024 {
                return Err(input("Input exceeds 16 KiB."));
            }
        }
    } else {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags((nix::fcntl::OFlag::O_NOFOLLOW | nix::fcntl::OFlag::O_NONBLOCK).bits())
            .open(path)?;
        let m = file.metadata()?;
        if !m.is_file()
            || (secret
                && (m.uid() != nix::unistd::Uid::effective().as_raw() || m.mode() & 0o777 != 0o600))
        {
            return Err(input(
                "Seed input must be an owned regular 0600 file; input symlinks are refused.",
            ));
        }
        file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 {
            return Err(input("Input exceeds 16 KiB."));
        }
    }
    Ok(bytes)
}
fn seed(args: &ArgMatches) -> Result<String> {
    let mut raw = source(text(args, "seed-file"), true)?;
    let result = (|| {
        let value = std::str::from_utf8(&raw)
            .map_err(|_| input("Seed must be canonical base64url seed32."))?
            .trim_end_matches(['\r', '\n']);
        let mut decoded = values::binary(value, 32)
            .map_err(|_| input("Seed must be canonical base64url seed32."))?;
        let valid = decoded.len() == 32;
        decoded.fill(0);
        if !valid {
            return Err(input("Seed must be canonical base64url seed32."));
        }
        Ok(value.to_owned())
    })();
    raw.fill(0);
    result
}
fn principal<'a>(detail: &'a Value, kind: &str, id: &str) -> Result<&'a Value> {
    detail[kind]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["id"] == id))
        .ok_or_else(|| {
            fail(
                "COLAB_PAGE_NOT_FOUND",
                "Member or link has no assignment on this page.",
            )
        })
}
fn selection(
    command: &str,
    args: &ArgMatches,
    page: &Value,
    detail: &Value,
) -> Result<Option<(&'static str, Value, bool)>> {
    let id = &page["pageId"];
    Ok(Some(match command {
        "archive" => ("page.archive", json!({"pageId":id}), false),
        "delete" => ("page.delete", json!({"pageId":id}), true),
        "retention" => {
            if let Some(days) = args.get_one::<u64>("days") {
                ("retention.set", json!({"pageId":id,"days":days}), false)
            } else if args.get_flag("forever") {
                ("retention.set", json!({"pageId":id,"days":null}), false)
            } else {
                return Ok(None);
            }
        }
        "share" => {
            let (action, selected) = args.subcommand().expect("required share action");
            match action {
                "mode" => {
                    let mode = text(selected, "mode");
                    let rank = |v: &str| match v {
                        "public" => 2,
                        "link" => 1,
                        _ => 0,
                    };
                    (
                        "page.share",
                        json!({"pageId":id,"mode":mode}),
                        rank(mode) > rank(page["sharing"].as_str().unwrap_or("private")),
                    )
                }
                "history" => {
                    let mode = text(selected, "mode");
                    (
                        "page.history",
                        json!({"pageId":id,"mode":mode}),
                        mode == "shared" && page["history"] == "current",
                    )
                }
                "members" => {
                    let (action, selected) = selected.subcommand().expect("required member action");
                    if action == "ls" {
                        return Ok(None);
                    }
                    if action == "add" {
                        let bytes = source(text(selected, "file"), false)?;
                        // Deserialize directly into the existing strict member DTO; this
                        // rejects duplicate/unknown fields before freezing selections.
                        let member: tmt_colab_model::payload::MemberAdd =
                            serde_json::from_slice(&bytes)
                                .map_err(|_| input("Invalid member selection JSON."))?;
                        let value = json!({"memberId":member.member_id,"role":match member.role {
                            tmt_colab_model::payload::Role::Viewer=>"viewer",
                            tmt_colab_model::payload::Role::Commenter=>"commenter",
                            tmt_colab_model::payload::Role::Editor=>"editor",
                        },"signKey":member.sign_key,"encKey":member.enc_key,"pages":member.pages.as_slice()});
                        ("member.add", value, true)
                    } else {
                        let member = uuid(text(selected, "member"))?;
                        let old = principal(detail, "members", &member)?;
                        if action == "remove" {
                            (
                                "member.remove",
                                json!({"memberId":member,"pages":old["pages"]}),
                                false,
                            )
                        } else {
                            let role = text(selected, "role");
                            let rank = |r: &str| match r {
                                "editor" => 2,
                                "commenter" => 1,
                                _ => 0,
                            };
                            (
                                "member.role",
                                json!({"memberId":member,"role":role,"pages":old["pages"]}),
                                rank(role) > rank(old["role"].as_str().unwrap_or("viewer")),
                            )
                        }
                    }
                }
                "link" => {
                    let (action, selected) = selected.subcommand().expect("required link action");
                    if action == "ls" {
                        return Ok(None);
                    }
                    let old = if action != "add" {
                        Some(principal(detail, "links", &uuid(text(selected, "link"))?)?)
                    } else {
                        None
                    };
                    let pages = old
                        .map(|r| r["pages"].clone())
                        .unwrap_or_else(|| json!([id]));
                    if action == "remove" {
                        (
                            "link.remove",
                            json!({"linkId":text(selected,"link"),"pages":pages,"replacement":null}),
                            false,
                        )
                    } else {
                        let link_id = selected
                            .get_one::<String>("link-id")
                            .map(|v| uuid(v))
                            .transpose()?
                            .map_or_else(fresh_id, Ok)?;
                        let new = json!({"linkId":link_id,"role":text(selected,"role"),"pages":pages,"seed":seed(selected)?});
                        if action == "add" {
                            ("link.add", new, true)
                        } else {
                            (
                                "link.remove",
                                json!({"linkId":text(selected,"link"),"pages":pages,"replacement":new}),
                                true,
                            )
                        }
                    }
                }
                _ => return Err(input("Unsupported sharing action.")),
            }
        }
        _ => return Err(input("Unsupported management command.")),
    }))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Acknowledgment {
    operation_id: String,
    membership_head: AcknowledgedHead,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AcknowledgedHead {
    revision: String,
    statement_hash: String,
}
fn remaining(deadline: Instant) -> Result<Duration> {
    deadline.checked_duration_since(Instant::now()).filter(|d|!d.is_zero()).ok_or_else(||fail("COLAB_OUTCOME_UNKNOWN","Management IPC deadline expired; effects may have committed. Inspect state before an exact retry."))
}
fn ready(socket: &UnixStream, flags: nix::poll::PollFlags, deadline: Instant) -> Result<()> {
    loop {
        let ms = u16::try_from(remaining(deadline)?.as_millis())
            .unwrap_or(u16::MAX)
            .max(1);
        let mut fds = [nix::poll::PollFd::new(socket.as_fd(), flags)];
        match nix::poll::poll(&mut fds, ms) {
            Ok(0) => return Err(input("IPC deadline exceeded.")),
            Ok(_) => return Ok(()),
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => return Err(e.into()),
        }
    }
}
fn ipc(layout: &Layout, body: &[u8]) -> Result<Vec<u8>> {
    let path = layout.directory.join(tmt_colab::socket::SOCKET);
    let m = std::fs::symlink_metadata(&path)?;
    if !m.file_type().is_socket()
        || m.uid() != nix::unistd::Uid::effective().as_raw()
        || m.mode() & 0o777 != 0o600
    {
        return Err(StateFault::UnsafeFile.into());
    }
    // The exchange deadline begins after this blocking local-path connect.
    // A full listener backlog can stall connect if serve stops accepting; that
    // failure needs diagnosis, never a fallback to an independent offline writer.
    let mut socket = UnixStream::connect(path)?;
    let deadline = Instant::now() + Duration::from_secs(4);
    let header = format!(
        "POST {} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        management::LOCAL_PATH,
        body.len()
    );
    socket.set_nonblocking(true)?;
    // After the first write, any transport failure is potentially committed.
    let exchange = (|| -> Result<Vec<u8>> {
        for mut bytes in [header.as_bytes(), body] {
            while !bytes.is_empty() {
                ready(&socket, nix::poll::PollFlags::POLLOUT, deadline)?;
                match socket.write(bytes) {
                    Ok(0) => return Err(input("IPC write stopped.")),
                    Ok(n) => bytes = &bytes[n..],
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => continue,
                    Err(e) => return Err(e.into()),
                }
            }
        }
        socket.shutdown(std::net::Shutdown::Write)?;
        let mut response = Vec::new();
        loop {
            ready(&socket, nix::poll::PollFlags::POLLIN, deadline)?;
            let mut buffer = [0;4096];
            match socket.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => response.extend_from_slice(&buffer[..n]),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => continue,
                Err(e) => return Err(e.into()),
            }
            if response.len() > crate::IPC_RESPONSE_BYTES { return Err(input("IPC response exceeds its bound.")); }
        }
        Ok(response)
    })().map_err(|cause| fail("COLAB_OUTCOME_UNKNOWN", &format!("Management IPC interrupted ({cause}); effects may have committed. Inspect state before retrying the same ID, revision and selections.")))?;
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut parsed = httparse::Response::new(&mut headers);
    let offset = match parsed.parse(&exchange) {
        Ok(httparse::Status::Complete(n)) => n,
        _ => {
            return Err(fail(
                "COLAB_OUTCOME_UNKNOWN",
                "Invalid management IPC response; effects may have committed.",
            ));
        }
    };
    let length = parsed
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("Content-Length"))
        .and_then(|h| std::str::from_utf8(h.value).ok())
        .and_then(|v| v.parse::<usize>().ok());
    if length != Some(exchange.len() - offset) {
        return Err(fail(
            "COLAB_OUTCOME_UNKNOWN",
            "Incomplete management IPC response; effects may have committed.",
        ));
    }
    if parsed.code != Some(200) {
        let value: Value = serde_json::from_slice(&exchange[offset..])
            .map_err(|_| fail("COLAB_OUTCOME_UNKNOWN", "Unrecognized management reply."))?;
        return Err(management_error(
            value["code"].as_str().unwrap_or("UNAVAILABLE"),
        ));
    }
    Ok(exchange[offset..].to_vec())
}
fn management_error(code: &str) -> Box<dyn std::error::Error + Send + Sync> {
    let (code, message) = match code {
        "INVALID" => ("COLAB_INVALID", "Management selections are invalid."),
        "DENIED" => ("COLAB_DENIED", "Management request denied."),
        "EXPIRED" => ("COLAB_EXPIRED", "Management request expired."),
        "CONFLICT" => (
            "COLAB_CONFLICT",
            "Operation ID conflicts with previously committed selections.",
        ),
        "STALE_HEAD" => (
            "COLAB_STALE_HEAD",
            "Owner state changed; review it before creating a new operation.",
        ),
        "CAPACITY" => (
            "COLAB_CAPACITY",
            "Management capacity exceeded; nothing is silently truncated.",
        ),
        _ => ("COLAB_UNAVAILABLE", "Management operation is unavailable."),
    };
    fail(code, message)
}
fn mutate(layout: &Layout, space: &str, body: &[u8]) -> Result<Vec<u8>> {
    match layout.serve_lock() {
        Ok(_lock) => {
            let key = Keyring::read(layout)?;
            let store = Store::open(layout)?;
            let mut service = Registration::new(store, key, std::env::current_exe()?)?;
            let now = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
            let result = management::local(&mut service, space, body, now)
                .map_err(|c| management_error(c.text()));
            let closed = service.close();
            // The service call may have committed before close failed.
            let bytes = result?;
            closed.map_err(|_| {
                fail(
                    "COLAB_OUTCOME_UNKNOWN",
                    "State close failed after management; inspect state before retrying.",
                )
            })?;
            Ok(bytes)
        }
        Err(e) if e.downcast_ref::<StateFault>() == Some(&StateFault::AlreadyServing) => {
            ipc(layout, body)
        }
        Err(e) => Err(e),
    }
}
pub fn run(command: &str, args: &ArgMatches, root: &Path, json_output: bool) -> Result<()> {
    let layout = Layout::existing(root)?;
    let Some(layout) = layout else {
        if command == "ls" {
            return output(
                &json!({"spaceId":null,"membershipHead":null,"pages":[]}),
                json_output,
            );
        }
        return Err(fail("COLAB_PAGE_NOT_FOUND", "Local page is not available."));
    };
    let exists = |name: &str| -> Result<bool> {
        match std::fs::symlink_metadata(layout.directory.join(name)) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    };
    if !exists("owner.key")? || !exists("space.db")? {
        if command == "ls" {
            return output(
                &json!({"spaceId":null,"membershipHead":null,"pages":[]}),
                json_output,
            );
        }
        return Err(fail("COLAB_PAGE_NOT_FOUND", "Local page is not available."));
    }
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    let mut catalog = inspection::catalog(&store, &key)?;
    if command == "ls" {
        let pages = catalog["pages"]
            .as_array_mut()
            .ok_or_else(|| input("Invalid local page catalog."))?;
        pages.retain(|p| args.get_flag("archived") || p["archived"] != true);
        for page in pages {
            inspection::title(&store, &key, page, std::env::current_exe()?)?;
        }
        if catalog["membershipHead"] != inspection::catalog(&store, &key)?["membershipHead"] {
            return Err(management_error("STALE_HEAD"));
        }
        return output(&catalog, json_output);
    }
    let mut page_args = args;
    while let Some((_, next)) = page_args.subcommand() {
        page_args = next;
    }
    let id = uuid(text(page_args, "page"))?;
    let replay_deleted = command == "delete"
        && args.get_one::<String>("operation-id").is_some()
        && args.get_one::<String>("expected-revision").is_some();
    let mut page = catalog["pages"]
        .as_array()
        .and_then(|rows| rows.iter().find(|p| p["pageId"] == id))
        .cloned()
        .or_else(|| replay_deleted.then(|| json!({"pageId":id})))
        .ok_or_else(|| fail("COLAB_PAGE_NOT_FOUND", "Local page is not available."))?;
    let detail = if replay_deleted {
        json!({"membershipHead":catalog["membershipHead"]})
    } else {
        inspection::detail(&store, &key, &page)?
    };
    if catalog["membershipHead"] != detail["membershipHead"] {
        return Err(management_error("STALE_HEAD"));
    }
    if command == "show" {
        inspection::title(&store, &key, &mut page, std::env::current_exe()?)?;
        if catalog["membershipHead"] != inspection::catalog(&store, &key)?["membershipHead"] {
            return Err(management_error("STALE_HEAD"));
        }
        let mut detail = detail;
        detail["page"] = page;
        return output(&detail, json_output);
    }
    let Some((operation, payload, widening)) = selection(command, args, &page, &detail)? else {
        return output(
            &if command == "share" {
                {
                    let group = args.subcommand_name().expect("required share group");
                    json!({"membershipHead":detail["membershipHead"],group:detail[group]})
                }
            } else {
                json!({"page":page,"membershipHead":detail["membershipHead"]})
            },
            json_output,
        );
    };
    if widening && !args.get_flag("yes") {
        return Err(fail(
            "COLAB_CONFIRMATION_REQUIRED",
            "Requires --yes after reviewing sharing/history disclosure in help. Delete ceases access and removes ciphertext; copied plaintext and previously public history cannot be recalled.",
        ));
    }
    let operation_id = args
        .get_one::<String>("operation-id")
        .map(|v| uuid(v))
        .transpose()?
        .map_or_else(fresh_id, Ok)?;
    let revision = args
        .get_one::<String>("expected-revision")
        .map(String::as_str)
        .unwrap_or_else(|| detail["membershipHead"]["revision"].as_str().unwrap_or("0"));
    values::decimal(revision, false)
        .map_err(|_| input("Expected a positive canonical owner revision."))?;
    let mut correlation = json!({"operationId":operation_id,"expectedRevision":revision});
    let link_id = match operation {
        "link.add" => payload.get("linkId"),
        "link.remove" => payload["replacement"].get("linkId"),
        _ => None,
    };
    if let Some(id) = link_id {
        correlation["linkId"] = id.clone();
    }
    let body = serde_json::to_vec(
        &json!({"space":key.space_id,"page":id,"expectedRevision":revision,"operationId":operation_id,
        "operation":operation,"payload":values::encode_binary(&serde_json::to_vec(&payload)?)}),
    )?;
    store.close()?;
    let outcome = (|| -> Result<Value> {
        let bytes = mutate(&layout, &key.space_id, &body)?;
        let ack: Acknowledgment = serde_json::from_slice(&bytes).map_err(|_| fail(
            "COLAB_OUTCOME_UNKNOWN", "Invalid management acknowledgment; effects may have committed."))?;
        if ack.operation_id != operation_id
            || values::decimal(&ack.membership_head.revision, false).is_err()
            || values::binary(&ack.membership_head.statement_hash, 32).is_err()
            || values::binary(&ack.membership_head.statement_hash, 32)?.len()!=32 {
            return Err(fail("COLAB_OUTCOME_UNKNOWN", "Mismatched management acknowledgment; inspect state."));
        }
        let mut value = json!({"operationId":ack.operation_id,
            "membershipHead":{"revision":ack.membership_head.revision,"statementHash":ack.membership_head.statement_hash}});
        value["expectedRevision"] = json!(revision);
        if let Some(id) = link_id { value["linkId"] = id.clone(); }
        Ok(value)
    })()
    .map_err(|e| {
        if let Some(f) = e.downcast_ref::<Failure>() {
            Box::new(Failure {
                code: f.code,
                message: f.message.clone(),
                correlation: correlation.clone(),
            }) as Box<dyn std::error::Error + Send + Sync>
        } else {
            Box::new(Failure {
                code: crate::error_code(e.as_ref()),
                message: e.to_string(),
                correlation: correlation.clone(),
            }) as Box<dyn std::error::Error + Send + Sync>
        }
    })?;
    output(&outcome, json_output)
}
fn output(value: &Value, json_output: bool) -> Result<()> {
    let mut out = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(out, "{value}")?;
        return Ok(());
    }
    let terminal = out.terminal();
    if let Some(pages) = value["pages"].as_array() {
        use tmt_cli_style::table::{Cell, Column, Table};
        let mut rows = Table::new(&[Column::Fixed, Column::Detail, Column::Fixed]);
        for page in pages {
            rows.row([
                Cell::from(page["pageId"].as_str().unwrap_or("")),
                Cell::from(page["title"].as_str().unwrap_or("title unavailable")),
                Cell::from(format!(
                    "{} / {}{}",
                    page["sharing"].as_str().unwrap_or(""),
                    page["history"].as_str().unwrap_or(""),
                    if page["archived"] == true {
                        " / archived"
                    } else {
                        ""
                    }
                )),
            ]);
        }
        tmt_cli_style::list::Section {title:"PAGES",count:Some(pages.len()),rows,note:Some("Expiry times unavailable pending #1350; local data is never automatically deleted.".into()),hint:None}.write(&mut out,terminal)?;
    } else {
        let fields = value
            .as_object()
            .ok_or_else(|| input("Invalid CLI result."))?
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str(),
                    v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string()),
                )
            })
            .collect::<Vec<_>>();
        tmt_cli_style::detail::write(&mut out, terminal, "COLAB", &fields)?;
        if value.get("page").is_some() {
            let mut stderr = tmt_cli_style::stream::stderr();
            let terminal = stderr.terminal();
            tmt_cli_style::message::warning(
                &mut stderr,
                terminal,
                "Expiry times unavailable pending #1350; local data is never automatically deleted.",
                None,
            )?;
        }
    }
    Ok(())
}
