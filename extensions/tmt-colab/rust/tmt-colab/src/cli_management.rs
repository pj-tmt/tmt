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
pub struct ManagementFault {
    pub code: &'static str,
    pub message: String,
    pub correlation: Value,
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}
impl std::fmt::Display for ManagementFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ManagementFault {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|error| error.as_ref() as _)
    }
}
fn fail(code: &'static str, message: &str) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(ManagementFault {
        code,
        message: message.into(),
        correlation: json!({}),
        source: None,
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
fn source(path: &str) -> Result<Vec<u8>> {
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
            || m.uid() != nix::unistd::Uid::effective().as_raw()
            || m.mode() & 0o777 != 0o600
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
/// A caller-held seed from `--seed-file`, or a fresh one from the OS RNG when the flag is absent.
fn seed(args: &ArgMatches) -> Result<String> {
    let Some(path) = args.get_one::<String>("seed-file") else {
        let mut fresh = [0u8; 32];
        getrandom::fill(&mut fresh).map_err(|_| input("Could not generate a sharing seed."))?;
        let encoded = values::encode_binary(&fresh);
        fresh.fill(0);
        return Ok(encoded);
    };
    let mut raw = source(path)?;
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
                "Principal has no assignment on this page.",
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
                "member" => {
                    let (action, selected) = selected.subcommand().expect("required member action");
                    let member = uuid(text(selected, "member"))?;
                    match action {
                        "add" => {
                            let payload = json!({"memberId":member,"role":text(selected,"role"),
                                "signKey":text(selected,"sign-key"),"encKey":text(selected,"enc-key"),"pages":[id]});
                            tmt_colab_model::payload::decode("member.add", &serde_json::to_vec(&payload)?)
                                .map_err(|_| input("Expected canonical member ID, role and valid Ed25519/X25519 public keys."))?;
                            if values::binary(text(selected, "enc-key"), 32)?
                                .iter()
                                .all(|v| *v == 0)
                            {
                                return Err(input("Encryption public key cannot be all zero."));
                            }
                            ("member.add", payload, true)
                        }
                        "remove" | "role" => {
                            let old = principal(detail, "members", &member)?;
                            let mut payload = json!({"memberId":member,"pages":old["pages"]});
                            if action == "remove" {
                                ("member.remove", payload, false)
                            } else {
                                let role = text(selected, "role");
                                let rank = |v: &str| match v {
                                    "editor" => 2,
                                    "commenter" => 1,
                                    _ => 0,
                                };
                                let widening =
                                    rank(role) > rank(old["role"].as_str().unwrap_or("viewer"));
                                payload["role"] = json!(role);
                                ("member.role", payload, widening)
                            }
                        }
                        _ => return Err(input("Unsupported member action.")),
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
                        let new = json!({"linkId":link_id,"role":"viewer","pages":pages,"seed":seed(selected)?});
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
        "retention" => {
            let Some(days) = args.get_one::<String>("days") else {
                return Ok(None);
            };
            let days = if days == "forever" {
                Value::Null
            } else {
                let count = values::decimal(days, false).map_err(|_| {
                    input("Expected a positive canonical safe-integer day count or forever.")
                })?;
                values::time(count)
                    .map_err(|_| input("Day count exceeds the safe-integer bound."))?;
                json!(count)
            };
            ("retention.set", json!({"pageId":id,"days":days}), false)
        }
        "archive" => ("page.archive", json!({"pageId":id}), false),
        "delete" => ("page.delete", json!({"pageId":id}), true),
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
/// The reader link grammar is owned by the colab-v1 contract. The path is relative to the
/// Remote door address, like `page create`'s `path`; everything secret is in the fragment.
fn reader_path(space: &str, page: &str, link: &Value, head: &AcknowledgedHead) -> String {
    format!(
        "x/colab/read#v=1&space={space}&page={page}&link={}&rev={}&st={}&seed={}",
        link["linkId"].as_str().unwrap_or_default(),
        head.revision,
        head.statement_hash,
        link["seed"].as_str().unwrap_or_default(),
    )
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
        // The management socket's error body is the exact textual Code, not JSON.
        let code = match &exchange[offset..] {
            b"INVALID" => management::Code::Invalid,
            b"DENIED" => management::Code::Denied,
            b"EXPIRED" => management::Code::Expired,
            b"CONFLICT" => management::Code::Conflict,
            b"STALE_HEAD" => management::Code::StaleHead,
            b"CAPACITY" => management::Code::Capacity,
            b"UNAVAILABLE" => management::Code::Unavailable,
            _ => {
                return Err(fail(
                    "COLAB_OUTCOME_UNKNOWN",
                    "Unrecognized management reply.",
                ));
            }
        };
        if parsed.code != Some(management::status(code)) {
            return Err(fail(
                "COLAB_OUTCOME_UNKNOWN",
                "Mismatched management reply status.",
            ));
        }
        return Err(management_error(code.text()));
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
fn offline(store: Store, key: Keyring, space: &str, body: &[u8]) -> Result<Vec<u8>> {
    let mut service = Registration::new(store, key, std::env::current_exe()?)?;
    let now = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    let result =
        management::local(&mut service, space, body, now).map_err(|c| management_error(c.text()));
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
fn mutate(layout: &Layout, space: &str, body: &[u8]) -> Result<Vec<u8>> {
    match layout.serve_lock() {
        Ok(_lock) => {
            let key = Keyring::read(layout)?;
            let store = Store::open(layout)?;
            offline(store, key, space, body)
        }
        Err(e) if e.downcast_ref::<StateFault>() == Some(&StateFault::AlreadyServing) => {
            ipc(layout, body)
        }
        Err(e) => Err(e),
    }
}
/// Creation is the only CLI command that initializes missing local state.
pub fn create_page(root: &Path, args: &ArgMatches, source: String) -> Result<()> {
    let title = text(args, "title");
    if title.is_empty() {
        return Err(input("Page title cannot be empty."));
    }
    if title.len() > tmt_colab::decoder::BASELINE_TITLE_BYTES {
        return Err(management_error("CAPACITY"));
    }
    let publisher_agent = crate::core::publisher_agent();
    let operation_id = fresh_id()?;
    let page_id = fresh_id()?;
    let layout = Layout::open(root)?;
    let correlation = json!({"operationId":operation_id,"pageId":page_id});
    let mut payload = json!({"pageId":page_id,"title":title,"source":source});
    if let Some(agent) = publisher_agent {
        payload["publisherAgent"] = json!(agent);
    }
    let request = |key: &Keyring, store: &Store| -> Result<Vec<u8>> {
        let revision = store
            .owner_head(&key.space_id, &key.owner_public())?
            .map_or(0, |h| h.revision)
            .to_string();
        Ok(serde_json::to_vec(
            &json!({"space":key.space_id,"page":page_id,
            "expectedRevision":revision,"operationId":operation_id,"operation":"page.create",
            "payload":values::encode_binary(&serde_json::to_vec(&payload)?)}),
        )?)
    };
    let result = (|| -> Result<Value> {
        let (space, bytes) = match layout.serve_lock() {
            Ok(_lock) => {
                let key = Keyring::open(&layout)?;
                let store = Store::open(&layout)?;
                let body = request(&key, &store)?;
                let space = key.space_id.clone();
                let bytes = offline(store, key, &space, &body)?;
                (space, bytes)
            }
            Err(error) if error.downcast_ref::<StateFault>() == Some(&StateFault::AlreadyServing) => {
                let key = Keyring::read(&layout)?;
                let store = Store::read(&layout)?;
                store.require_current_schema()?;
                let body = request(&key, &store)?;
                store.close()?;
                (key.space_id.clone(), ipc(&layout, &body)?)
            }
            Err(error) => return Err(error),
        };
        let ack: Acknowledgment = serde_json::from_slice(&bytes)
            .map_err(|_| fail("COLAB_OUTCOME_UNKNOWN", "Invalid creation acknowledgment; inspect pages before retrying."))?;
        if ack.operation_id != operation_id
            || values::decimal(&ack.membership_head.revision, false).is_err()
            || values::binary(&ack.membership_head.statement_hash, 32).map_or(true, |hash| hash.len() != 32) {
            return Err(fail("COLAB_OUTCOME_UNKNOWN", "Mismatched creation acknowledgment; inspect pages before retrying."));
        }
        Ok(json!({"spaceId":space,"pageId":page_id,"title":title,
            "path":format!("x/colab/#space={space}&path=%2Fpages%2F{page_id}"),
            "operationId":operation_id,"membershipHead":{
                "revision":ack.membership_head.revision,"statementHash":ack.membership_head.statement_hash
            }}))
    })().map_err(|error| {
        Box::new(ManagementFault {
            code:crate::error_code(error.as_ref()),message:error.to_string(),correlation:correlation.clone(),source:Some(error),
        }) as Box<dyn std::error::Error + Send + Sync>
    })?;
    // The effect is already committed. A catalog-read failure must not turn it into a failed create;
    // the full UUID alias remains safe when uniqueness cannot be observed.
    let ids = (|| -> Result<Vec<String>> {
        let layout = Layout::existing(root)?.ok_or_else(|| input("Missing local space."))?;
        let key = Keyring::read(&layout)?;
        let store = Store::read(&layout)?;
        Ok(catalog_ids(&inspection::catalog(&store, &key)?))
    })()
    .unwrap_or_default();
    let mut result = result;
    let relative = result["path"]
        .as_str()
        .ok_or_else(|| input("Missing created page path."))?
        .to_owned();
    let reach = crate::reach::Reach::gather().with_pages(&ids);
    if args.get_flag("json") {
        reach.annotate(&mut result, &relative);
        return output(&result, true);
    }
    let mut out = tmt_cli_style::stream::stdout(false);
    let terminal = out.terminal();
    // The page opens in the browser unless the setting, a flag or the environment says not to.
    let mut shown = reach.text(&relative);
    let mut warnings = Vec::new();
    if let Some(link) = reach.short_link(&relative) {
        // The page is committed: unreadable settings are the defaults, never a failed command.
        let settings = tmt_colab::settings::read_or_default(root);
        let outcome =
            crate::open::open_link(&link, crate::open::Flag::of(args), settings.open(), false);
        let (text, failed) = crate::open::describe(&outcome, &link);
        shown = text;
        warnings.extend(failed);
        if settings.malformed {
            warnings.push(tmt_colab::settings::UNREADABLE.to_owned());
        }
    }
    let mut rows = vec![
        ("page", page_id),
        ("title", title.to_owned()),
        ("open", shown),
    ];
    rows.extend(reach.step().map(|step| ("pair", step.to_owned())));
    tmt_cli_style::detail::write(&mut out, terminal, "PAGE CREATED", &rows)?;
    for what in &warnings {
        let mut err = tmt_cli_style::stream::stderr();
        let terminal = err.terminal();
        tmt_cli_style::message::warning(&mut err, terminal, what, None)?;
    }
    Ok(())
}
/// Names a page's title, or, when that page cannot be opened, records why on that page alone so
/// the other pages stay listed and shown (#1627).
fn title_or_error(store: &Store, key: &Keyring, page: &mut Value) -> Result<()> {
    if let Err(error) = inspection::title(store, key, page, std::env::current_exe()?) {
        page["title"] = Value::Null;
        page["error"] = json!({
            "code": crate::error_code(error.as_ref()),
            "message": error.to_string(),
        });
        if let Some(warnings) = page["warnings"].as_array_mut() {
            warnings.push(json!("page-unavailable"));
        }
    }
    Ok(())
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
    store.require_current_schema()?;
    let mut catalog = inspection::catalog(&store, &key)?;
    let ids = catalog_ids(&catalog);
    if command == "ls" {
        let pages = catalog["pages"]
            .as_array_mut()
            .ok_or_else(|| input("Invalid local page catalog."))?;
        pages.retain(|p| args.get_flag("archived") || p["archived"] != true);
        for page in pages {
            title_or_error(&store, &key, page)?;
        }
        if catalog["membershipHead"] != inspection::catalog(&store, &key)?["membershipHead"] {
            return Err(management_error("STALE_HEAD"));
        }
        // One door lookup for the whole listing; each page carries its own path and link.
        let reach = crate::reach::Reach::gather().with_pages(&ids);
        for page in catalog["pages"].as_array_mut().into_iter().flatten() {
            let path =
                crate::reach::Reach::path(&key.space_id, page["pageId"].as_str().unwrap_or(""));
            if json_output {
                reach.annotate_link(page, &path);
            } else {
                // Human-only: the text under each row, which JSON replaces with `path` and `link`.
                page["linkText"] = json!(reach.text(&path));
            }
        }
        let listed = catalog["pages"].as_array().is_some_and(|p| !p.is_empty());
        if json_output {
            let mut facts = json!({});
            reach.annotate(&mut facts, "");
            catalog["paired"] = facts["paired"].take();
            catalog["next"] = facts["next"].take();
        } else if let (true, Some(step)) = (listed, reach.step()) {
            catalog["pairStep"] = json!(step);
        }
        return output(&catalog, json_output);
    }
    let mut page_args = args;
    while let Some((_, next)) = page_args.subcommand() {
        page_args = next;
    }
    let id = uuid(text(page_args, "page"))?;
    let selected_page = catalog["pages"]
        .as_array()
        .and_then(|rows| rows.iter().find(|p| p["pageId"] == id))
        .cloned();
    // Deletion removes the visible catalog row, not its operation receipt. Only
    // an explicit frozen retry may reach the engine without a current page view.
    let delete_retry = command == "delete"
        && args.get_one::<String>("operation-id").is_some()
        && args.get_one::<String>("expected-revision").is_some();
    let (mut page, detail) = match selected_page {
        Some(page) => {
            let detail = inspection::detail(&store, &key, &page)?;
            (page, detail)
        }
        None if delete_retry => (
            json!({"pageId":id}),
            json!({"membershipHead":catalog["membershipHead"]}),
        ),
        None => return Err(fail("COLAB_PAGE_NOT_FOUND", "Local page is not available.")),
    };
    if catalog["membershipHead"] != detail["membershipHead"] {
        return Err(management_error("STALE_HEAD"));
    }
    if command == "show" {
        title_or_error(&store, &key, &mut page)?;
        if catalog["membershipHead"] != inspection::catalog(&store, &key)?["membershipHead"] {
            return Err(management_error("STALE_HEAD"));
        }
        let mut detail = detail;
        let reach = crate::reach::Reach::gather().with_pages(&ids);
        let path = crate::reach::Reach::path(&key.space_id, &id);
        detail["page"] = page;
        if json_output {
            reach.annotate(&mut detail, &path);
            return output(&detail, true);
        }
        return output_with(&detail, false, &page_rows(&reach, &path));
    }
    let Some((operation, payload, widening)) = selection(command, args, &page, &detail)? else {
        if command == "retention" {
            return output(
                &json!({"membershipHead":detail["membershipHead"],"page":{
                "pageId":id,"retentionDays":page["retentionDays"],"lastUpdateAtMs":page["lastUpdateAtMs"],
                "expiresAtMs":page["expiresAtMs"],"warnings":page["warnings"]}}),
                json_output,
            );
        }
        return output(
            &json!({"membershipHead":detail["membershipHead"],"links":detail["links"]}),
            json_output,
        );
    };
    if widening && !args.get_flag("yes") {
        return Err(fail(
            "COLAB_CONFIRMATION_REQUIRED",
            "Requires --yes after reviewing this command's disclosure in help. Deletion is permanent; copied plaintext and previously public history cannot be recalled.",
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
        // The one place a seed leaves the CLI: inside the reader link's fragment.
        let created = match operation {
            "link.add" => Some(&payload),
            "link.remove" => payload.get("replacement").filter(|r| !r.is_null()),
            _ => None,
        };
        if let Some(created) = created {
            value["readerPath"] = json!(reader_path(
                &key.space_id,
                &id,
                created,
                &ack.membership_head
            ));
        }
        Ok(value)
    })()
    .map_err(|e| {
        if let Some(f) = e.downcast_ref::<ManagementFault>() {
            Box::new(ManagementFault {
                code: f.code,
                message: f.message.clone(),
                correlation: correlation.clone(),
                source: Some(e),
            }) as Box<dyn std::error::Error + Send + Sync>
        } else {
            Box::new(ManagementFault {
                code: crate::error_code(e.as_ref()),
                message: e.to_string(),
                correlation: correlation.clone(),
                source: Some(e),
            }) as Box<dyn std::error::Error + Send + Sync>
        }
    })?;
    // The door is looked up after the commit, so a slow answer never delays the effect.
    let reach = crate::reach::Reach::gather().with_pages(&ids);
    let path = crate::reach::Reach::path(&key.space_id, &id);
    let mut outcome = outcome;
    if let Some(url) = outcome["readerPath"]
        .as_str()
        .and_then(|reader| reach.link(reader))
    {
        outcome["readerUrl"] = json!(url);
    }
    if json_output {
        reach.annotate(&mut outcome, &path);
        return output(&outcome, true);
    }
    let mut rows = page_rows(&reach, &path);
    if let Some(reader) = outcome["readerPath"].as_str() {
        rows.push(("reader link", reach.text(reader)));
    }
    output_with(&outcome, false, &rows)
}
/// Capture all IDs before presentation filters; archived pages still make a prefix ambiguous.
fn catalog_ids(catalog: &Value) -> Vec<String> {
    catalog["pageIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|page| page["pageId"].as_str().map(str::to_owned))
        .collect()
}
/// The rows every page-naming command shares: where to open the page, and the pairing step.
fn page_rows(reach: &crate::reach::Reach, path: &str) -> Vec<(&'static str, String)> {
    let mut rows = vec![("link", reach.text(path))];
    rows.extend(reach.step().map(|step| ("pair", step.to_owned())));
    rows
}
const LOCAL_COPY_NOTE: &str = "Expiry never deletes your local copy.";
const DAY_MS: u64 = 86_400_000;

/// Whole elapsed units, matching the browser's retention interval (not UTC calendar days).
fn retention_interval(duration_ms: u64) -> String {
    let days = duration_ms / DAY_MS;
    if days >= 1 {
        format!("{days} {}", if days == 1 { "day" } else { "days" })
    } else if duration_ms >= 3_600_000 {
        format!("{} h", duration_ms / 3_600_000)
    } else {
        "less than an hour".into()
    }
}
fn last_edit_text(updated_ms: u64, now_ms: u64) -> String {
    let duration = updated_ms.abs_diff(now_ms);
    if duration < 60_000 {
        return if updated_ms > now_ms {
            "in less than a minute".into()
        } else {
            "just now".into()
        };
    }
    let interval = if duration < 3_600_000 {
        format!("{} min", duration / 60_000)
    } else {
        retention_interval(duration)
    };
    if updated_ms > now_ms {
        format!("in {interval}")
    } else {
        format!("{interval} ago")
    }
}
/// CLI display values named by UX; retention intervals match browser expiryText.
/// List/detail/warning callers share this copy owner, without final periods.
fn expiry_text(page: &Value, now_ms: u64, listing: bool) -> String {
    if page["retentionDays"].is_null() {
        return "kept forever".into();
    }
    if page["warnings"]
        .as_array()
        .is_some_and(|w| w.iter().any(|v| v == "expiry-out-of-range"))
    {
        return "beyond the supported range".into();
    }
    let Some(ms) = page["expiresAtMs"].as_u64() else {
        return "starts after the next edit".into();
    };
    let interval = retention_interval(ms.abs_diff(now_ms));
    if ms <= now_ms {
        format!("expired {interval} ago")
    } else {
        let mark = if ms - now_ms <= 7 * DAY_MS {
            "◷ "
        } else {
            ""
        };
        let verb = if listing { "expires " } else { "" };
        format!("{mark}{verb}in {interval}")
    }
}
fn warn_expiry(page: &Value, now_ms: u64) -> Result<()> {
    if page["warnings"]
        .as_array()
        .is_some_and(|w| w.iter().any(|v| v == "expires-soon" || v == "expired"))
    {
        let mut out = tmt_cli_style::stream::stderr();
        let terminal = out.terminal();
        tmt_cli_style::message::warning(
            &mut out,
            terminal,
            &format!(
                "{}: {} · {LOCAL_COPY_NOTE}",
                page["pageId"].as_str().unwrap_or("Page"),
                expiry_text(page, now_ms, false)
            ),
            None,
        )?;
    }
    Ok(())
}
/// Readable lines for a management result: no raw JSON blobs, full IDs where a command needs them.
fn human_fields(value: &Value, now_ms: u64) -> Result<Vec<(String, String)>> {
    let object = value
        .as_object()
        .ok_or_else(|| input("Invalid CLI result."))?;
    let text = |v: &Value| {
        v.as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| v.to_string())
    };
    let principals = |rows: &Value| match rows.as_array() {
        Some(rows) if !rows.is_empty() => rows
            .iter()
            .map(|row| {
                let mut line = text(&row["id"]);
                if let Some(role) = row["role"].as_str() {
                    line = format!("{line} {role}");
                }
                if row["revoked"] == true {
                    line.push_str(" (revoked)");
                }
                line
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => "–".to_owned(),
    };
    let mut fields = Vec::new();
    for (key, v) in object {
        match key.as_str() {
            "page" if v.is_object() => {
                if let Some(title) = v["title"].as_str() {
                    fields.push(("title".to_owned(), title.to_owned()));
                }
                fields.push(("page".to_owned(), text(&v["pageId"])));
                if let (Some(sharing), Some(history)) =
                    (v["sharing"].as_str(), v["history"].as_str())
                {
                    fields.push((
                        "audience".to_owned(),
                        format!("{sharing} · {history} history"),
                    ));
                }
                if let Some(epoch) = v["epoch"].as_str() {
                    fields.push(("epoch".to_owned(), epoch.to_owned()));
                }
                if let Some(days) = v.get("retentionDays") {
                    fields.push((
                        "retention".to_owned(),
                        if days.is_null() {
                            "forever".to_owned()
                        } else if v["warnings"]
                            .as_array()
                            .is_some_and(|w| w.iter().any(|v| v == "expiry-out-of-range"))
                        {
                            "out of range".to_owned()
                        } else {
                            format!("{} days", text(days))
                        },
                    ));
                }
                if let Some(updated) = v["lastUpdateAtMs"].as_u64() {
                    fields.push(("last edit".to_owned(), last_edit_text(updated, now_ms)));
                }
                fields.push(("expiry".to_owned(), expiry_text(v, now_ms, false)));
                if v["archived"] == true {
                    fields.push(("archived".to_owned(), "yes".to_owned()));
                }
                if let Some(message) = v["error"]["message"].as_str() {
                    fields.push(("unavailable".to_owned(), message.to_owned()));
                }
            }
            "membershipHead" if v.is_object() => {
                let hash = v["statementHash"].as_str().unwrap_or_default();
                fields.push((
                    "membership".to_owned(),
                    format!(
                        "revision {} · {}",
                        text(&v["revision"]),
                        hash.get(..12).unwrap_or(hash)
                    ),
                ));
            }
            // The link rows carry the space; the reader link is added by the caller.
            "spaceId" | "readerUrl" | "readerPath" => {}
            "operationId" => fields.push(("operation".to_owned(), text(v))),
            "expectedRevision" => fields.push(("expected revision".to_owned(), text(v))),
            "linkId" => fields.push(("link".to_owned(), text(v))),
            // An unavailable value is a missing one, written like the other missing values.
            "discussions" if v == "not-available" => fields.push((key.clone(), "–".to_owned())),
            "members" => fields.push(("members".to_owned(), principals(v))),
            "links" => fields.push(("links".to_owned(), principals(v))),
            _ => fields.push((key.clone(), text(v))),
        }
    }
    Ok(fields)
}
fn output(value: &Value, json_output: bool) -> Result<()> {
    output_with(value, json_output, &[])
}
/// Human output for a result; `extra` rows (the page link, the pairing step) follow the title,
/// or the other rows when the result has none.
fn output_with(value: &Value, json_output: bool, extra: &[(&str, String)]) -> Result<()> {
    let mut out = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(out, "{value}")?;
        return Ok(());
    }
    // One clock for every page, detail and warning in this human result. JSON is untouched.
    let now_ms = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())
        .map_err(|_| input("Local clock is beyond the supported range."))?;
    let terminal = out.terminal();
    if let Some(pages) = value["pages"].as_array() {
        use tmt_cli_style::{
            palette::Token,
            table::{Cell, Column, Table, escape},
        };
        writeln!(
            out,
            "{} {}",
            terminal.paint(Token::Title, "PAGES"),
            terminal.paint(Token::Dim, &pages.len().to_string())
        )?;
        // The audience column is padded to one width so separate rows still line up. Under each
        // row sit the link and the expiry line: titles truncate, links never do.
        let audience = |page: &Value| {
            format!(
                "{} / {}{}",
                page["sharing"].as_str().unwrap_or(""),
                page["history"].as_str().unwrap_or(""),
                if page["archived"] == true {
                    " / archived"
                } else {
                    ""
                }
            )
        };
        let width = pages
            .iter()
            .map(|p| audience(p).chars().count())
            .max()
            .unwrap_or(0);
        for page in pages {
            let mut rows = Table::new(&[Column::Fixed, Column::Fixed, Column::Name]);
            rows.row([
                Cell::from(page["pageId"].as_str().unwrap_or("")),
                Cell::from(format!("{:<width$}", audience(page))),
                match (page["title"].as_str(), page["error"]["code"].as_str()) {
                    (Some(title), _) => Cell::from(title),
                    // The page is intact but too big to open; the code stays in --json and on stderr.
                    (None, Some("COLAB_CAPACITY")) => Cell::styled("too large to open", Token::Dim),
                    (None, Some(code)) => Cell::from(format!("unavailable ({code})")),
                    (None, None) => Cell::from("title unavailable"),
                },
            ]);
            rows.write(&mut out, terminal)?;
            if let Some(text) = page["linkText"].as_str() {
                writeln!(out, "    {}", escape(text))?;
            }
            writeln!(
                out,
                "    {}",
                terminal.paint(Token::Dim, &escape(&expiry_text(page, now_ms, true)))
            )?;
        }
        // A footer, set off from the rows at the list indent: what to do, then the standing note.
        writeln!(out)?;
        if let Some(step) = value["pairStep"].as_str() {
            writeln!(out, "  {}", terminal.paint(Token::Dim, &escape(step)))?;
        }
        writeln!(out, "  {}", terminal.paint(Token::Dim, LOCAL_COPY_NOTE))?;
        for page in pages {
            warn_expiry(page, now_ms)?;
        }
        // Each unavailable page explains itself; the others above are unaffected.
        for page in pages {
            if let Some(message) = page["error"]["message"].as_str() {
                let mut stderr = tmt_cli_style::stream::stderr();
                let terminal = stderr.terminal();
                tmt_cli_style::message::warning(&mut stderr, terminal, message, None)?;
            }
        }
    } else {
        let mut fields = human_fields(value, now_ms)?;
        // The title leads, then where to open it.
        if let Some(at) = fields.iter().position(|(key, _)| key == "title") {
            let title = fields.remove(at);
            fields.insert(0, title);
        }
        let at = fields
            .iter()
            .position(|(key, _)| key == "title")
            .map_or(fields.len(), |at| at + 1);
        let extra = extra.iter().map(|(k, v)| (k.to_string(), v.clone()));
        fields.splice(at..at, extra);
        let fields = fields
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect::<Vec<_>>();
        tmt_cli_style::detail::write(&mut out, terminal, "COLAB", &fields)?;
        if let Some(page) = value.get("page") {
            writeln!(
                out,
                "\n  {}",
                terminal.paint(tmt_cli_style::palette::Token::Dim, LOCAL_COPY_NOTE)
            )?;
            warn_expiry(page, now_ms)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod expiry_tests {
    use super::{DAY_MS, expiry_text, human_fields, last_edit_text};
    use serde_json::{Value, json};

    fn page(expires: Option<u64>, warnings: &[&str]) -> Value {
        json!({"retentionDays":30,"expiresAtMs":expires,"warnings":warnings})
    }
    #[test]
    fn expiry_uses_browser_intervals_and_cli_values_at_boundaries() {
        let now = 30 * DAY_MS;
        for (expires, detail, list) in [
            (now + 30 * DAY_MS, "in 30 days", "expires in 30 days"),
            (now + 7 * DAY_MS + 1, "in 7 days", "expires in 7 days"),
            (now + 7 * DAY_MS, "◷ in 7 days", "◷ expires in 7 days"),
            (now + DAY_MS, "◷ in 1 day", "◷ expires in 1 day"),
            (now + DAY_MS - 1, "◷ in 23 h", "◷ expires in 23 h"),
            (now + 3_600_000, "◷ in 1 h", "◷ expires in 1 h"),
            (
                now + 3_599_999,
                "◷ in less than an hour",
                "◷ expires in less than an hour",
            ),
            (
                now + 1,
                "◷ in less than an hour",
                "◷ expires in less than an hour",
            ),
            (
                now,
                "expired less than an hour ago",
                "expired less than an hour ago",
            ),
            (now - 2 * DAY_MS, "expired 2 days ago", "expired 2 days ago"),
        ] {
            assert_eq!(expiry_text(&page(Some(expires), &[]), now, false), detail);
            assert_eq!(expiry_text(&page(Some(expires), &[]), now, true), list);
        }
    }
    #[test]
    fn unavailable_states_use_ux_display_values_and_forever_has_no_warning() {
        for listing in [false, true] {
            assert_eq!(
                expiry_text(&page(None, &["expiry-unavailable"]), 0, listing),
                "starts after the next edit"
            );
            assert_eq!(
                expiry_text(&page(None, &["expiry-out-of-range"]), 0, listing),
                "beyond the supported range"
            );
            let mut forever = page(None, &["expiry-out-of-range"]);
            forever["retentionDays"] = Value::Null;
            assert_eq!(expiry_text(&forever, 0, listing), "kept forever");
        }
    }
    #[test]
    fn last_edit_handles_whole_units_and_clock_rollback_without_overflow() {
        let now = 30 * DAY_MS;
        for (updated, expected) in [
            (now, "just now"),
            (now - 59_999, "just now"),
            (now - 60_000, "1 min ago"),
            (now - 120_999, "2 min ago"),
            (now - 3_600_000, "1 h ago"),
            (now - DAY_MS, "1 day ago"),
            (now + 1, "in less than a minute"),
            (now + 120_000, "in 2 min"),
        ] {
            assert_eq!(last_edit_text(updated, now), expected);
        }
        assert_eq!(last_edit_text(0, u64::MAX), "213503982334 days ago");
        assert_eq!(last_edit_text(u64::MAX, 0), "in 213503982334 days");
        assert_eq!(
            expiry_text(&page(Some(u64::MAX), &[]), 0, false),
            "in 213503982334 days"
        );
        assert_eq!(
            expiry_text(&page(Some(0), &[]), u64::MAX, false),
            "expired 213503982334 days ago"
        );
    }
    #[test]
    fn details_display_verified_out_of_range_without_changing_retention() {
        let source = json!({"page":{"pageId":"page", "retentionDays":9_007_199_254_740_991u64,"lastUpdateAtMs":1_000,"expiresAtMs":null,"warnings":["expiry-out-of-range"]}});
        let before = source.clone();
        let fields = human_fields(&source, 121_000).unwrap();
        assert!(fields.contains(&("retention".into(), "out of range".into())));
        assert!(fields.contains(&("expiry".into(), "beyond the supported range".into())));
        assert!(
            !fields
                .iter()
                .any(|(_, value)| value.contains("9007199254740991"))
        );
        assert_eq!(source, before);
    }
    #[test]
    fn details_use_the_supplied_clock_and_leave_the_projection_exact() {
        let source = json!({"page":{"pageId":"page", "retentionDays":30,"lastUpdateAtMs":1_000,"expiresAtMs":30 * DAY_MS + 1_000,"warnings":[]}});
        let before = source.clone();
        let fields = human_fields(&source, 121_000).unwrap();
        assert!(fields.contains(&("last edit".into(), "2 min ago".into())));
        assert!(fields.contains(&("expiry".into(), "in 29 days".into())));
        assert_eq!(source, before);
    }
}
