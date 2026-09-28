use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    process::ExitCode,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tmt_adapters::config::ConfigPaths;
use tmt_adapters::office_profile::mutation_value as local_profile_mutation;
use tmt_adapters::office_profile::snapshot_value as local_profile_snapshot;
use tmt_adapters::office_service;
use tmt_adapters::office_service::ServiceReceipt;
use tmt_adapters::storage::LocalProfileError;
use tmt_adapters::storage::Storage;
use tmt_adapters::tmux::BindingSession;
use tmt_adapters::tmux::CallerEnvironment;
use tmt_adapters::tmux::Tmux;
use tmt_core::binding;
use tmt_office_model::codec::office_avatar::ValidatedAvatarPack;
use tmt_office_model::codec::office_avatar::validate_pack as validate_avatar_pack;
use tmt_office_model::codec::office_profile_wire;
use tmt_office_model::codec::office_prop::PACK_INPUT_LIMIT;
use tmt_office_model::codec::office_prop::ValidatedPropPack;
use tmt_office_model::codec::office_prop::validate_pack as validate_prop_pack;
use tmt_office_model::office_protocol::OfficeInvocation;

use crate::local_assets;

mod dispatch;
mod notebooks;
mod props;
mod request_history;
mod rooms;
#[cfg(test)]
mod test_fixture;
mod whiteboard;
mod world;

const HEADER_LIMIT: usize = 16 * 1024;
const BODY_LIMIT: usize = 64 * 1024;
const PREVIEW_LIMIT: usize = 4;
const PREVIEW_LIFETIME: Duration = Duration::from_secs(5 * 60);
// Preserve the retained-catalog envelope and reserve one maximum bundled pack.
const AVATAR_CATALOG_OUTPUT_LIMIT: usize =
    256 * 1024 + tmt_office_model::codec::office_avatar::PACK_INPUT_LIMIT;
const MAX_CONNECTIONS: usize = 16;
const REQUEST_DEADLINE: Duration = Duration::from_secs(3);
const RESPONSE_DEADLINE: Duration = Duration::from_secs(15);

enum ServeError {
    PortUnavailable(io::Error),
    Other(io::Error),
}

impl From<io::Error> for ServeError {
    fn from(error: io::Error) -> Self {
        Self::Other(error)
    }
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

struct ActiveConnection(Arc<AtomicUsize>);

struct PreviewEntry {
    pack: PreviewPack,
    expires: Instant,
    expires_at_ms: u64,
}

#[derive(Clone)]
enum PreviewPack {
    Prop(ValidatedPropPack),
    Avatar(ValidatedAvatarPack),
}

impl PreviewPack {
    fn digest(&self) -> &str {
        match self {
            Self::Prop(pack) => pack.digest(),
            Self::Avatar(pack) => pack.digest(),
        }
    }
    fn ui_kind(&self) -> &'static str {
        match self {
            Self::Prop(_) => "props",
            Self::Avatar(_) => "avatars",
        }
    }
    fn value(&self) -> serde_json::Value {
        match self {
            Self::Prop(pack) => json!({"digest":pack.digest(),"pack":pack.pack()}),
            Self::Avatar(pack) => json!({"digest":pack.digest(),"pack":pack.pack()}),
        }
    }
}

#[derive(Clone, Copy)]
enum PreviewKind {
    Prop,
    Avatar,
}

type Previews = Arc<Mutex<HashMap<String, PreviewEntry>>>;

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        let mut values = self
            .headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name));
        let value = values.next()?.1.as_str();
        values.next().is_none().then_some(value)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApplyProfileInput {
    expected_revision: u64,
    profile: Value,
}

pub fn run() -> ExitCode {
    match serve() {
        Ok(()) => ExitCode::SUCCESS,
        Err(ServeError::PortUnavailable(error)) => {
            let mut stdout = io::stdout().lock();
            let _ = writeln!(stdout, "TMT-OFFICE-SERVICE/1 PORT-UNAVAILABLE");
            let _ = stdout.flush();
            let _ = writeln!(io::stderr().lock(), "Local Office service failed: {error}");
            ExitCode::FAILURE
        }
        Err(ServeError::Other(error)) => {
            let _ = writeln!(io::stderr().lock(), "Local Office service failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn serve() -> Result<(), ServeError> {
    let paths = ConfigPaths::discover().map_err(io::Error::other)?;
    let _service_lock = office_service::service_lock(&paths)?;
    let requested = std::env::var("TMT_OFFICE_SERVICE_PORT")
        .map_err(io::Error::other)?
        .parse::<u16>()
        .map_err(io::Error::other)?;
    let listener =
        TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, requested)).map_err(|error| {
            if matches!(
                error.kind(),
                io::ErrorKind::AddrInUse | io::ErrorKind::PermissionDenied
            ) {
                ServeError::PortUnavailable(error)
            } else {
                ServeError::Other(error)
            }
        })?;
    let port = listener.local_addr()?.port();
    // Initialize the shared schema before concurrent browser requests arrive.
    // Readiness means storage is usable, not only that the port can accept a socket.
    let mut storage = Storage::open(&paths.database).map_err(io::Error::other)?;
    storage.close().map_err(io::Error::other)?;
    let receipt = ServiceReceipt {
        schema_version: 1,
        pid: std::process::id(),
        port,
        nonce: secret()?,
        browser_token: secret()?,
        control_token: secret()?,
        running_version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    office_service::write_receipt(&paths, &receipt)?;
    writeln!(io::stdout().lock(), "TMT-OFFICE-SERVICE/1 READY")?;
    io::stdout().lock().flush()?;
    let stopping = Arc::new(AtomicBool::new(false));
    let active = Arc::new(AtomicUsize::new(0));
    let previews: Previews = Arc::new(Mutex::new(HashMap::new()));
    let mut workers: Vec<std::thread::JoinHandle<()>> = Vec::new();
    listener.set_nonblocking(true)?;
    while !stopping.load(Ordering::Acquire) {
        let mut index = 0;
        while index < workers.len() {
            if workers[index].is_finished() {
                let worker: std::thread::JoinHandle<()> = workers.swap_remove(index);
                let _ = worker.join();
            } else {
                index += 1;
            }
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                if stopping.load(Ordering::Acquire) {
                    continue;
                }
                // Listener nonblocking mode is platform-specific for accepted
                // sockets. Normalize each bounded worker stream before writing
                // assets larger than the kernel send buffer.
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                if active
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                        (count < MAX_CONNECTIONS).then_some(count + 1)
                    })
                    .is_err()
                {
                    continue;
                }
                let guard = ActiveConnection(Arc::clone(&active));
                let paths = paths.clone();
                let receipt = receipt.clone();
                let stopping = Arc::clone(&stopping);
                let previews = Arc::clone(&previews);
                workers.push(std::thread::spawn(move || {
                    let _guard = guard;
                    let _ = serve_connection(&mut stream, &paths, &receipt, &stopping, &previews);
                }));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    drop(listener);
    for worker in workers {
        let _ = worker.join();
    }
    office_service::remove_matching_receipt(&paths, &receipt)
        .map_err(io::Error::other)
        .map_err(ServeError::Other)
}

fn serve_connection(
    stream: &mut TcpStream,
    paths: &ConfigPaths,
    receipt: &ServiceReceipt,
    stopping: &AtomicBool,
    previews: &Previews,
) -> io::Result<()> {
    let _ = stream.set_write_timeout(Some(RESPONSE_DEADLINE));
    match handle(stream, paths, receipt, stopping, previews) {
        Ok(()) => Ok(()),
        Err(_) => response(
            stream,
            400,
            "application/json",
            br#"{"error":"BAD_REQUEST"}"#,
        ),
    }
}

fn handle(
    stream: &mut TcpStream,
    paths: &ConfigPaths,
    receipt: &ServiceReceipt,
    stopping: &AtomicBool,
    previews: &Previews,
) -> io::Result<()> {
    let request = read_request(stream, Instant::now() + REQUEST_DEADLINE)?;
    if request.method == "GET" && !request.body.is_empty() {
        return response(
            stream,
            400,
            "application/json",
            br#"{"error":"BAD_REQUEST"}"#,
        );
    }
    let host = format!("127.0.0.1:{}", receipt.port);
    if request.header("host") != Some(host.as_str()) {
        return response(
            stream,
            421,
            "application/json",
            br#"{"error":"HOST_REJECTED"}"#,
        );
    }
    if request.path.starts_with("/control/v1/") {
        return control(stream, &request, receipt, stopping, previews);
    }
    if request.path.starts_with("/api/v1/") {
        return api(stream, request, paths, receipt, previews);
    }
    if request.method != "GET" {
        return response(
            stream,
            405,
            "text/plain; charset=utf-8",
            b"Method not allowed",
        );
    }
    let asset = local_assets::find(&request.path).or_else(|| {
        (request.path == "/" || request.path.starts_with("/local/")).then(local_assets::index)
    });
    match asset {
        Some((content_type, bytes)) => response(stream, 200, content_type, bytes),
        None => response(stream, 404, "text/plain; charset=utf-8", b"Not found"),
    }
}

fn control(
    stream: &mut TcpStream,
    request: &Request,
    receipt: &ServiceReceipt,
    stopping: &AtomicBool,
    previews: &Previews,
) -> io::Result<()> {
    let authorization = format!("Bearer {}", receipt.control_token);
    if request.method != "POST"
        || request.header("authorization") != Some(authorization.as_str())
        || request.header("x-tmt-office-nonce") != Some(receipt.nonce.as_str())
    {
        return response(
            stream,
            401,
            "application/json",
            br#"{"error":"UNAUTHORIZED"}"#,
        );
    }
    match request.path.as_str() {
        "/control/v1/health" if request.body.is_empty() => {
            response(stream, 200, "application/json", br#"{"ok":true}"#)
        }
        "/control/v1/stop" if request.body.is_empty() => {
            let result = response(stream, 200, "application/json", br#"{"ok":true}"#);
            if result.is_ok() {
                stopping.store(true, Ordering::Release);
            }
            result
        }
        "/control/v1/prop-previews"
            if request.header("content-type") == Some("application/json") =>
        {
            create_preview(stream, request, receipt, previews, PreviewKind::Prop)
        }
        "/control/v1/avatar-previews"
            if request.header("content-type") == Some("application/json") =>
        {
            create_preview(stream, request, receipt, previews, PreviewKind::Avatar)
        }
        _ => response(stream, 404, "application/json", br#"{"error":"NOT_FOUND"}"#),
    }
}

fn create_preview(
    stream: &mut TcpStream,
    request: &Request,
    receipt: &ServiceReceipt,
    previews: &Previews,
    kind: PreviewKind,
) -> io::Result<()> {
    let pack = match kind {
        PreviewKind::Prop => validate_prop_pack(&request.body)
            .map(PreviewPack::Prop)
            .map_err(|_| ()),
        PreviewKind::Avatar => validate_avatar_pack(&request.body)
            .map(PreviewPack::Avatar)
            .map_err(|_| ()),
    };
    let pack = match pack {
        Ok(pack) => pack,
        Err(_) => {
            let body = match kind {
                PreviewKind::Prop => br#"{"error":"OFFICE_PROP_INVALID"}"#.as_slice(),
                PreviewKind::Avatar => br#"{"error":"OFFICE_AVATAR_INVALID"}"#.as_slice(),
            };
            return response(stream, 400, "application/json", body);
        }
    };
    let mut previews = previews
        .lock()
        .map_err(|_| io::Error::other("preview lock poisoned"))?;
    let now = Instant::now();
    previews.retain(|_, entry| entry.expires > now);
    if let Some((preview_id, entry)) = previews
        .iter()
        .find(|(_, entry)| entry.pack.digest() == pack.digest())
    {
        return preview_response(
            stream,
            receipt,
            preview_id,
            &entry.pack,
            entry.expires_at_ms,
        );
    }
    if previews.len() >= PREVIEW_LIMIT {
        let body = match kind {
            PreviewKind::Prop => br#"{"error":"OFFICE_PROP_PREVIEW_LIMIT"}"#.as_slice(),
            PreviewKind::Avatar => br#"{"error":"OFFICE_AVATAR_PREVIEW_LIMIT"}"#.as_slice(),
        };
        return response(stream, 429, "application/json", body);
    }
    let preview_id = secret()?;
    let expires_at_ms = unix_time_ms()?.saturating_add(
        u64::try_from(PREVIEW_LIFETIME.as_millis()).expect("preview lifetime fits u64"),
    );
    previews.insert(
        preview_id.clone(),
        PreviewEntry {
            pack: pack.clone(),
            expires: now + PREVIEW_LIFETIME,
            expires_at_ms,
        },
    );
    preview_response(stream, receipt, &preview_id, &pack, expires_at_ms)
}

fn preview_response(
    stream: &mut TcpStream,
    receipt: &ServiceReceipt,
    preview_id: &str,
    pack: &PreviewPack,
    expires_at_ms: u64,
) -> io::Result<()> {
    let url = format!(
        "{}/local/{}/preview/{preview_id}#token={}",
        receipt.endpoint(),
        pack.ui_kind(),
        receipt.browser_token
    );
    let body = serde_json::to_vec(&json!({
        "digest":pack.digest(),
        "previewId":preview_id,
        "expiresAtMs":expires_at_ms,
        "url":url
    }))?;
    response(stream, 200, "application/json", &body)
}

fn unix_time_ms() -> io::Result<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_millis()
        .try_into()
        .map_err(io::Error::other)
}

fn api(
    stream: &mut TcpStream,
    request: Request,
    paths: &ConfigPaths,
    receipt: &ServiceReceipt,
    previews: &Previews,
) -> io::Result<()> {
    let authorization = format!("Bearer {}", receipt.browser_token);
    if request.header("authorization") != Some(authorization.as_str()) {
        return response(
            stream,
            401,
            "application/json",
            br#"{"error":"UNAUTHORIZED"}"#,
        );
    }
    if let Some(preview_id) = request.path.strip_prefix("/api/v1/local/prop-previews/") {
        if request.method != "GET" || preview_id.is_empty() || preview_id.contains('/') {
            return response(stream, 404, "application/json", br#"{"error":"NOT_FOUND"}"#);
        }
        let mut previews = previews
            .lock()
            .map_err(|_| io::Error::other("preview lock poisoned"))?;
        previews.retain(|_, entry| entry.expires > Instant::now());
        let Some(entry) = previews.get(preview_id) else {
            return response(stream, 404, "application/json", br#"{"error":"NOT_FOUND"}"#);
        };
        if !matches!(&entry.pack, PreviewPack::Prop(_)) {
            return response(stream, 404, "application/json", br#"{"error":"NOT_FOUND"}"#);
        }
        let body = serde_json::to_vec(&entry.pack.value())?;
        return response(stream, 200, "application/json", &body);
    }
    if let Some(preview_id) = request.path.strip_prefix("/api/v1/local/avatar-previews/") {
        if request.method != "GET" || preview_id.is_empty() || preview_id.contains('/') {
            return response(stream, 404, "application/json", br#"{"error":"NOT_FOUND"}"#);
        }
        let mut previews = previews
            .lock()
            .map_err(|_| io::Error::other("preview lock poisoned"))?;
        previews.retain(|_, entry| entry.expires > Instant::now());
        let Some(entry) = previews.get(preview_id) else {
            return response(stream, 404, "application/json", br#"{"error":"NOT_FOUND"}"#);
        };
        if !matches!(&entry.pack, PreviewPack::Avatar(_)) {
            return response(stream, 404, "application/json", br#"{"error":"NOT_FOUND"}"#);
        }
        let body = serde_json::to_vec(&entry.pack.value())?;
        return response(stream, 200, "application/json", &body);
    }
    if request.method == "POST" && request.path == "/api/v1/local/props/resolve" {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ResolveInput {
            digests: Vec<String>,
        }
        let input: ResolveInput = match serde_json::from_slice::<ResolveInput>(&request.body) {
            Ok(input) if input.digests.len() <= 16 => input,
            _ => {
                return response(
                    stream,
                    400,
                    "application/json",
                    br#"{"error":"OFFICE_PROP_INVALID"}"#,
                );
            }
        };
        let unique = input.digests.iter().collect::<HashSet<_>>();
        if unique.len() != input.digests.len()
            || input.digests.iter().any(|digest| {
                tmt_office_model::codec::office_prop::parse_pack_digest(digest).is_none()
            })
        {
            return response(
                stream,
                400,
                "application/json",
                br#"{"error":"OFFICE_PROP_INVALID"}"#,
            );
        }
        let mut storage = match Storage::open(&paths.database) {
            Ok(storage) => storage,
            Err(_) => {
                return response(
                    stream,
                    500,
                    "application/json",
                    br#"{"error":"STORAGE_UNAVAILABLE"}"#,
                );
            }
        };
        let resolution = match storage.resolve_local_prop_packs(&input.digests) {
            Ok(resolution) => resolution,
            Err(_) => {
                return response(
                    stream,
                    500,
                    "application/json",
                    br#"{"error":"STORAGE_UNAVAILABLE"}"#,
                );
            }
        };
        let packs = resolution
            .packs
            .iter()
            .map(|snapshot| json!({"digest":snapshot.pack.digest(),"pack":snapshot.pack.pack()}))
            .collect::<Vec<_>>();
        let body = serde_json::to_vec(&json!({
            "catalogRevision":resolution.catalog_revision,
            "packs":packs,
            "unavailable":resolution.unavailable
        }))?;
        if storage.close().is_err() {
            return response(
                stream,
                500,
                "application/json",
                br#"{"error":"STORAGE_UNAVAILABLE"}"#,
            );
        }
        return response(stream, 200, "application/json", &body);
    }
    if request.path.starts_with("/api/v1/local/board/") {
        return board_api(stream, request, paths, receipt);
    }
    if request.path.starts_with(notebooks::PREFIX) {
        return notebooks::api(stream, request, paths);
    }
    if props::handles(&request.path) {
        return props::api(stream, request, paths, receipt);
    }
    if request.path == dispatch::PATH {
        return dispatch::api(stream, request, paths, receipt);
    }
    if request.path == world::PATH {
        return world::api(stream, request, paths, receipt);
    }
    if request_history::handles(&request.path) {
        return request_history::api(stream, request, paths, receipt);
    }
    if request.path == rooms::PATH || request.path.starts_with("/api/v1/local/rooms/") {
        return rooms::api(stream, request, paths, receipt);
    }
    if request.path.starts_with("/api/v1/local/whiteboards/")
        || request
            .path
            .starts_with("/api/v1/local/whiteboard-snapshots/")
    {
        return whiteboard::api(stream, request, paths, receipt);
    }
    if request.method == "GET" && request.path == "/api/v1/local/avatar-catalog" {
        return avatar_catalog_api(stream, paths);
    }
    if request.path == "/api/v1/local/profiles"
        || request.path.starts_with("/api/v1/local/profiles/")
    {
        return profile_api(stream, request, paths, receipt);
    }
    response(stream, 404, "application/json", br#"{"error":"NOT_FOUND"}"#)
}

fn avatar_catalog_api(stream: &mut TcpStream, paths: &ConfigPaths) -> io::Result<()> {
    let mut storage = match Storage::open(&paths.database) {
        Ok(storage) => storage,
        Err(_) => {
            return response(
                stream,
                500,
                "application/json",
                br#"{"error":"STORAGE_UNAVAILABLE"}"#,
            );
        }
    };
    let mut cursor = None;
    let mut revision = None;
    let mut packs = Vec::new();
    loop {
        let page = match storage.list_local_avatar_packs(20, cursor.as_deref()) {
            Ok(page) if revision.is_none_or(|value| value == page.catalog_revision) => page,
            _ => {
                let _ = storage.close();
                return response(
                    stream,
                    500,
                    "application/json",
                    br#"{"error":"STORAGE_UNAVAILABLE"}"#,
                );
            }
        };
        revision = Some(page.catalog_revision);
        if cursor.is_none() {
            packs.extend(page.builtins.into_iter().map(
                |snapshot| json!({"digest":snapshot.pack.digest(),"pack":snapshot.pack.pack()}),
            ));
        }
        packs.extend(
            page.packs
                .into_iter()
                .filter(|snapshot| {
                    tmt_office_model::codec::office_avatar::builtin_by_digest(
                        snapshot.pack.digest(),
                    )
                    .is_none()
                })
                .map(
                    |snapshot| json!({"digest":snapshot.pack.digest(),"pack":snapshot.pack.pack()}),
                ),
        );
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    let body = serde_json::to_vec(&json!({
        "catalogRevision":revision.unwrap_or(0),
        "packs":packs
    }))?;
    if body.len() > AVATAR_CATALOG_OUTPUT_LIMIT || storage.close().is_err() {
        return response(
            stream,
            500,
            "application/json",
            br#"{"error":"STORAGE_UNAVAILABLE"}"#,
        );
    }
    response(stream, 200, "application/json", &body)
}

fn profile_api(
    stream: &mut TcpStream,
    request: Request,
    paths: &ConfigPaths,
    receipt: &ServiceReceipt,
) -> io::Result<()> {
    if request.method == "GET" && request.path == "/api/v1/local/profiles" {
        let mut storage = match Storage::open(&paths.database) {
            Ok(storage) => storage,
            Err(_) => {
                return response(
                    stream,
                    500,
                    "application/json",
                    br#"{"error":"STORAGE_UNAVAILABLE"}"#,
                );
            }
        };
        // A persisted binding is not presence: verify the recorded tmux
        // endpoints without collapsing unavailable evidence into offline.
        let tmux = Tmux::default();
        let environment = CallerEnvironment::current();
        let mut endpoint = BindingSession::new(&tmux);
        let presence = match binding::list_presence(
            &mut storage,
            &mut endpoint,
            environment.selected_socket(),
        ) {
            Ok(rows) => rows
                .into_iter()
                .map(|row| (row.identity.id, (row.presence, row.identity.lifetime)))
                .collect::<HashMap<_, _>>(),
            Err(_) => {
                return response(
                    stream,
                    500,
                    "application/json",
                    br#"{"error":"PRESENCE_UNAVAILABLE"}"#,
                );
            }
        };
        let profiles = match storage.list_active_local_profiles() {
            Ok(profiles) => profiles,
            Err(_) => {
                return response(
                    stream,
                    500,
                    "application/json",
                    br#"{"error":"STORAGE_UNAVAILABLE"}"#,
                );
            }
        };
        let statuses = match storage.list_active_identity_statuses() {
            Ok(statuses) => statuses,
            Err(_) => {
                return response(
                    stream,
                    500,
                    "application/json",
                    br#"{"error":"STORAGE_UNAVAILABLE"}"#,
                );
            }
        };
        let observed_at_ms = tmt_adapters::request_runtime::wall_time_ms();
        let mut values = Vec::with_capacity(profiles.len());
        for profile in profiles {
            let Some((observation, lifetime)) = presence.get(&profile.identity_id) else {
                return response(
                    stream,
                    409,
                    "application/json",
                    br#"{"error":"IDENTITY_DIRECTORY_CHANGED"}"#,
                );
            };
            let status = tmt_adapters::identity_status::status_value(
                statuses.get(&profile.identity_id),
                observed_at_ms,
            );
            let mut value = local_profile_snapshot(profile);
            value["presence"] = json!(observation.as_str());
            value["lifetime"] = json!(lifetime.as_str());
            value["selfReportedStatus"] = status;
            values.push(value);
        }
        let body = serde_json::to_vec(&values)?;
        if storage.close().is_err() {
            return response(
                stream,
                500,
                "application/json",
                br#"{"error":"STORAGE_UNAVAILABLE"}"#,
            );
        }
        return response(stream, 200, "application/json", &body);
    }
    let Some(identity_id) = request.path.strip_prefix("/api/v1/local/profiles/") else {
        return response(stream, 404, "application/json", br#"{"error":"NOT_FOUND"}"#);
    };
    if uuid::Uuid::parse_str(identity_id)
        .ok()
        .is_none_or(|id| id.to_string() != identity_id)
    {
        return response(stream, 404, "application/json", br#"{"error":"NOT_FOUND"}"#);
    }
    let put = request.method == "PUT";
    if request.method != "GET" && !put {
        return response(
            stream,
            405,
            "application/json",
            br#"{"error":"METHOD_NOT_ALLOWED"}"#,
        );
    }
    let edit = if put {
        let origin = format!("http://127.0.0.1:{}", receipt.port);
        if request.header("origin") != Some(origin.as_str())
            || request
                .header("content-type")
                .is_none_or(|value| value.split(';').next() != Some("application/json"))
        {
            return response(
                stream,
                403,
                "application/json",
                br#"{"error":"ORIGIN_REJECTED"}"#,
            );
        }
        if request.body.len() > tmt_office_model::office_profile::MAX_PROFILE_FILE_BYTES {
            return response(
                stream,
                400,
                "application/json",
                br#"{"error":"PROFILE_INVALID"}"#,
            );
        }
        let input: ApplyProfileInput = match serde_json::from_slice(&request.body) {
            Ok(input) => input,
            Err(_) => {
                return response(
                    stream,
                    400,
                    "application/json",
                    br#"{"error":"PROFILE_INVALID"}"#,
                );
            }
        };
        if input.expected_revision > tmt_office_model::office_profile::MAX_REVISION {
            return response(
                stream,
                400,
                "application/json",
                br#"{"error":"PROFILE_INVALID"}"#,
            );
        }
        let profile = match office_profile_wire::decode_value(input.profile) {
            Ok(profile) => profile,
            Err(_) => {
                return response(
                    stream,
                    400,
                    "application/json",
                    br#"{"error":"PROFILE_INVALID"}"#,
                );
            }
        };
        Some((input.expected_revision, profile))
    } else {
        None
    };
    let mut storage = match Storage::open(&paths.database) {
        Ok(storage) => storage,
        Err(_) => {
            return response(
                stream,
                500,
                "application/json",
                br#"{"error":"STORAGE_UNAVAILABLE"}"#,
            );
        }
    };
    let result = match edit {
        Some((expected_revision, profile)) => storage
            .apply_local_profile(identity_id, expected_revision, &profile)
            .map(local_profile_mutation),
        None => storage
            .show_local_profile(identity_id)
            .map(local_profile_snapshot),
    };
    let (status, body) = match result {
        Ok(profile) => (200, serde_json::to_vec(&profile)?),
        Err(LocalProfileError::ProfileInvalid) => (400, br#"{"error":"PROFILE_INVALID"}"#.to_vec()),
        Err(LocalProfileError::AvatarUnavailable) => {
            (409, br#"{"error":"AVATAR_UNAVAILABLE"}"#.to_vec())
        }
        Err(LocalProfileError::RevisionConflict | LocalProfileError::RevisionExhausted) => {
            (409, br#"{"error":"REVISION_CONFLICT"}"#.to_vec())
        }
        Err(LocalProfileError::IdentityInactive) => (404, br#"{"error":"NOT_FOUND"}"#.to_vec()),
        Err(_) => (500, br#"{"error":"STORAGE_UNAVAILABLE"}"#.to_vec()),
    };
    let close = storage.close();
    if status == 200 && close.is_err() {
        return response(
            stream,
            500,
            "application/json",
            br#"{"error":"STORAGE_UNAVAILABLE"}"#,
        );
    }
    response(stream, status, "application/json", &body)
}

fn board_api(
    stream: &mut TcpStream,
    request: Request,
    paths: &ConfigPaths,
    receipt: &ServiceReceipt,
) -> io::Result<()> {
    let origin = format!("http://127.0.0.1:{}", receipt.port);
    let (operation, value) = match prepare_board_request(&request, &origin) {
        Ok(value) => value,
        Err((status, body)) => return response(stream, status, "application/json", body),
    };
    let body = tmt_adapters::office_board::execute_at(
        operation,
        &serde_json::to_vec(&value)?,
        &paths.database,
    );
    let code = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned));
    let status = match code.as_deref() {
        None => 200,
        Some("BOARD_THREAD_NOT_FOUND" | "BOARD_ENTRY_NOT_FOUND") => 404,
        Some("BOARD_FORBIDDEN") => 403,
        Some(
            "BOARD_REVISION_CONFLICT"
            | "BOARD_IDEMPOTENCY_CONFLICT"
            | "BOARD_CURSOR_INVALID"
            | "BOARD_CURSOR_STALE",
        ) => 409,
        Some("BOARD_INVALID") => 400,
        _ => 500,
    };
    response(stream, status, "application/json", &body)
}

fn require_json_origin(request: &Request, origin: &str) -> Result<(), (u16, &'static [u8])> {
    require_content_origin(request, origin, "application/json")
}

fn require_content_origin(
    request: &Request,
    origin: &str,
    content_type: &str,
) -> Result<(), (u16, &'static [u8])> {
    if request.header("origin") != Some(origin)
        || request
            .header("content-type")
            .is_none_or(|value| value.split(';').next() != Some(content_type))
    {
        return Err((403, br#"{"error":"ORIGIN_REJECTED"}"#));
    }
    Ok(())
}

fn prepare_board_request(
    request: &Request,
    origin: &str,
) -> Result<(OfficeInvocation, Value), (u16, &'static [u8])> {
    require_json_origin(request, origin)?;
    let (operation, actor, path_id) = match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/api/v1/local/board/categories/list") => {
            (OfficeInvocation::BoardCategories, false, None)
        }
        ("POST", "/api/v1/local/board/threads/list") => (OfficeInvocation::BoardList, false, None),
        ("POST", "/api/v1/local/board/threads/show") => (OfficeInvocation::BoardShow, false, None),
        ("POST", "/api/v1/local/board/threads") => (OfficeInvocation::BoardPost, true, None),
        ("POST", path)
            if path.starts_with("/api/v1/local/board/threads/") && path.ends_with("/replies") =>
        {
            let id = &path[28..path.len() - 8];
            (OfficeInvocation::BoardReply, true, Some(("threadId", id)))
        }
        ("PUT", path) if path.starts_with("/api/v1/local/board/entries/") => (
            OfficeInvocation::BoardEdit,
            true,
            Some(("entryId", &path[28..])),
        ),
        ("DELETE", path) if path.starts_with("/api/v1/local/board/entries/") => (
            OfficeInvocation::BoardDelete,
            true,
            Some(("entryId", &path[28..])),
        ),
        _ => return Err((404, br#"{"error":"NOT_FOUND"}"#)),
    };
    let mut value: Value = match serde_json::from_slice(&request.body) {
        Ok(Value::Object(object)) => Value::Object(object),
        _ => {
            return Err((400, br#"{"error":"BOARD_INVALID"}"#));
        }
    };
    let object = value.as_object_mut().expect("validated object");
    if actor {
        object.insert("actor".into(), json!({"kind":"owner"}));
    }
    if let Some((key, id)) = path_id {
        if uuid::Uuid::parse_str(id)
            .ok()
            .is_none_or(|parsed| parsed.to_string() != id)
        {
            return Err((404, br#"{"error":"NOT_FOUND"}"#));
        }
        object.insert(key.into(), json!(id));
    }
    Ok((operation, value))
}

fn read_request(stream: &mut TcpStream, deadline: Instant) -> io::Result<Request> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 2048];
    let header_end = loop {
        if bytes.len() >= HEADER_LIMIT {
            return Err(io::Error::other("request headers too large"));
        }
        let read = read_before(stream, &mut chunk, deadline)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request ended",
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let end = index + 4;
            if end > HEADER_LIMIT {
                return Err(io::Error::other("request headers too large"));
            }
            break end;
        }
    };
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut parsed = httparse::Request::new(&mut headers);
    if parsed
        .parse(&bytes[..header_end])
        .map_err(io::Error::other)?
        != httparse::Status::Complete(header_end)
    {
        return Err(io::Error::other("incomplete request"));
    }
    if parsed.version != Some(1) {
        return Err(io::Error::other("HTTP/1.1 is required"));
    }
    let method = parsed
        .method
        .ok_or_else(|| io::Error::other("missing method"))?
        .to_owned();
    let path = parsed
        .path
        .ok_or_else(|| io::Error::other("missing path"))?
        .to_owned();
    if !path.starts_with('/') || path.contains('?') || path.contains('#') {
        return Err(io::Error::other("invalid path"));
    }
    let headers = parsed
        .headers
        .iter()
        .map(|header| {
            let value = std::str::from_utf8(header.value).map_err(io::Error::other)?;
            Ok((header.name.to_owned(), value.trim().to_owned()))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let transfer = headers
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case("transfer-encoding"));
    if transfer {
        return Err(io::Error::other("transfer encoding is unsupported"));
    }
    let lengths = headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.parse::<usize>().map_err(io::Error::other))
        .collect::<io::Result<Vec<_>>>()?;
    if lengths.len() > 1 {
        return Err(io::Error::other("duplicate content length"));
    }
    let content_length = lengths.first().copied().unwrap_or(0);
    let body_limit = if method == "POST" && path == "/control/v1/prop-previews" {
        PACK_INPUT_LIMIT
    } else if method == "POST" && path == "/control/v1/avatar-previews" {
        tmt_office_model::codec::office_avatar::PACK_INPUT_LIMIT
    } else if let Some(limit) = whiteboard::input_limit(&method, &path) {
        limit
    } else if method == "POST" && path == dispatch::PATH {
        tmt_adapters::dispatch::INPUT_LIMIT
    } else if method == "POST" && request_history::handles(&path) {
        tmt_adapters::request_history::HISTORY_INPUT_LIMIT
    } else if let Some(limit) = rooms::input_limit(&method, &path) {
        limit
    } else if let Some(limit) = world::input_limit(&method, &path) {
        limit
    } else if let Some(limit) = props::input_limit(&method, &path) {
        limit
    } else {
        BODY_LIMIT
    };
    if content_length > body_limit {
        return Err(io::Error::other("request body too large"));
    }
    while bytes.len() < header_end + content_length {
        let read = read_before(stream, &mut chunk, deadline)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request body ended",
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > header_end + body_limit {
            return Err(io::Error::other("request body too large"));
        }
    }
    if bytes.len() != header_end + content_length {
        return Err(io::Error::other("unexpected bytes after request body"));
    }
    Ok(Request {
        method,
        path,
        headers,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

fn read_before(stream: &mut TcpStream, buffer: &mut [u8], deadline: Instant) -> io::Result<usize> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "request deadline exceeded"))?;
    stream.set_read_timeout(Some(remaining.max(Duration::from_millis(1))))?;
    stream.read(buffer)
}

fn response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        421 => "Misdirected Request",
        429 => "Too Many Requests",
        _ => "Internal Server Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'self'; connect-src 'self'; img-src 'self' data: blob:; style-src 'self'; script-src 'self'; base-uri 'none'; frame-ancestors 'none'\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nCross-Origin-Resource-Policy: same-origin\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn secret() -> io::Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeSet, thread};

    pub(super) fn test_receipt() -> ServiceReceipt {
        ServiceReceipt {
            schema_version: 1,
            pid: std::process::id(),
            port: 1234,
            nonce: "n".into(),
            browser_token: "browser".into(),
            control_token: "control".into(),
            running_version: "test".into(),
        }
    }

    fn call_handler(run: impl FnOnce(&mut TcpStream) -> io::Result<()>) -> String {
        String::from_utf8(call_handler_bytes(run)).unwrap()
    }

    fn call_handler_bytes(run: impl FnOnce(&mut TcpStream) -> io::Result<()>) -> Vec<u8> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let receiver = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let (mut stream, _) = listener.accept().unwrap();
        run(&mut stream).unwrap();
        drop(stream);
        receiver.join().unwrap()
    }

    pub(super) fn response_value(response: &str) -> Value {
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    fn avatar_preview_bytes(index: usize) -> Vec<u8> {
        let mut value: Value = serde_json::from_slice(include_bytes!(
            "../../../contracts/avatar-pack-v1-sample.tmtavatar.json"
        ))
        .unwrap();
        value["label"] = json!(format!("Preview {index}"));
        serde_json::to_vec(&value).unwrap()
    }

    fn call_preview(
        previews: &Previews,
        receipt: &ServiceReceipt,
        body: Vec<u8>,
        kind: PreviewKind,
    ) -> String {
        let route_kind = match kind {
            PreviewKind::Prop => "prop",
            PreviewKind::Avatar => "avatar",
        };
        let request = Request {
            method: "POST".into(),
            path: format!("/control/v1/{route_kind}-previews"),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body,
        };
        call_handler(|stream| create_preview(stream, &request, receipt, previews, kind))
    }

    pub(super) fn parse_wire(wire: &[u8]) -> io::Result<Request> {
        let wire = wire.to_vec();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let sender = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream.set_write_timeout(Some(REQUEST_DEADLINE)).unwrap();
            stream.write_all(&wire)
        });
        let (mut stream, _) = listener.accept().unwrap();
        let result = read_request(&mut stream, Instant::now() + REQUEST_DEADLINE);
        // Rejection may happen from Content-Length before the body is drained.
        // Close the receiver before joining so a large sender cannot deadlock.
        drop(stream);
        let sent = sender.join().unwrap();
        if result.is_ok() {
            sent.unwrap();
        }
        result
    }

    fn wire_with_body(path: &str, body_len: usize) -> &'static [u8] {
        let mut wire = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Type: application/json\r\nContent-Length: {body_len}\r\n\r\n"
        )
        .into_bytes();
        wire.resize(wire.len() + body_len, b' ');
        Box::leak(wire.into_boxed_slice())
    }

    #[test]
    fn preview_routes_have_their_exact_distinct_body_caps() {
        assert!(parse_wire(wire_with_body("/control/v1/prop-previews", BODY_LIMIT + 1)).is_ok());
        assert!(parse_wire(wire_with_body("/api/v1/local/profiles", BODY_LIMIT + 1)).is_err());
        assert!(
            parse_wire(wire_with_body(
                "/control/v1/prop-previews",
                PACK_INPUT_LIMIT + 1
            ))
            .is_err()
        );
        assert!(
            parse_wire(wire_with_body(
                "/control/v1/avatar-previews",
                tmt_office_model::codec::office_avatar::PACK_INPUT_LIMIT
            ))
            .is_ok()
        );
        assert!(
            parse_wire(wire_with_body(
                "/control/v1/avatar-previews",
                tmt_office_model::codec::office_avatar::PACK_INPUT_LIMIT + 1
            ))
            .is_err()
        );
    }

    #[test]
    fn preview_registry_has_one_shared_limit_and_exact_retries_reuse_the_slot() {
        let previews: Previews = Arc::new(Mutex::new(HashMap::new()));
        let receipt = test_receipt();
        let prop = include_bytes!("../../../contracts/builtin-props-v1.tmtprop.json");
        assert!(
            call_preview(&previews, &receipt, prop.to_vec(), PreviewKind::Prop)
                .starts_with("HTTP/1.1 200")
        );
        let first = call_preview(
            &previews,
            &receipt,
            avatar_preview_bytes(0),
            PreviewKind::Avatar,
        );
        let first_id = response_value(&first)["previewId"]
            .as_str()
            .unwrap()
            .to_owned();
        for index in 1..3 {
            assert!(
                call_preview(
                    &previews,
                    &receipt,
                    avatar_preview_bytes(index),
                    PreviewKind::Avatar,
                )
                .starts_with("HTTP/1.1 200")
            );
        }
        assert_eq!(previews.lock().unwrap().len(), PREVIEW_LIMIT);
        let limited = call_preview(
            &previews,
            &receipt,
            avatar_preview_bytes(3),
            PreviewKind::Avatar,
        );
        assert!(limited.starts_with("HTTP/1.1 429"));
        let retried = call_preview(
            &previews,
            &receipt,
            avatar_preview_bytes(0),
            PreviewKind::Avatar,
        );
        assert_eq!(
            response_value(&retried)["previewId"].as_str(),
            Some(first_id.as_str())
        );
        assert_eq!(previews.lock().unwrap().len(), PREVIEW_LIMIT);
    }

    #[test]
    fn preview_routes_hide_the_other_kind_and_prune_expired_ids() {
        let previews: Previews = Arc::new(Mutex::new(HashMap::new()));
        let receipt = test_receipt();
        let created = call_preview(
            &previews,
            &receipt,
            avatar_preview_bytes(0),
            PreviewKind::Avatar,
        );
        let preview_id = response_value(&created)["previewId"]
            .as_str()
            .unwrap()
            .to_owned();
        let unused_root =
            std::env::temp_dir().join(format!("tmt-preview-route-test-{}", uuid::Uuid::new_v4()));
        let paths = ConfigPaths::resolve(&unused_root, &unused_root, None, None);
        let request = |kind: &str| Request {
            method: "GET".into(),
            path: format!("/api/v1/local/{kind}-previews/{preview_id}"),
            headers: vec![("Authorization".into(), "Bearer browser".into())],
            body: Vec::new(),
        };
        let mismatch =
            call_handler(|stream| api(stream, request("prop"), &paths, &receipt, &previews));
        assert!(mismatch.starts_with("HTTP/1.1 404"));
        assert_eq!(previews.lock().unwrap().len(), 1);
        previews
            .lock()
            .unwrap()
            .get_mut(&preview_id)
            .unwrap()
            .expires = Instant::now() - Duration::from_secs(1);
        let expired =
            call_handler(|stream| api(stream, request("avatar"), &paths, &receipt, &previews));
        assert!(expired.starts_with("HTTP/1.1 404"));
        assert!(previews.lock().unwrap().is_empty());
    }

    #[test]
    fn invalid_avatar_preview_does_not_mutate_the_registry() {
        let previews: Previews = Arc::new(Mutex::new(HashMap::new()));
        let receipt = test_receipt();
        assert!(
            call_preview(
                &previews,
                &receipt,
                avatar_preview_bytes(0),
                PreviewKind::Avatar,
            )
            .starts_with("HTTP/1.1 200")
        );
        let before = previews
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let invalid = call_preview(
            &previews,
            &receipt,
            br#"{"formatVersion":1}"#.to_vec(),
            PreviewKind::Avatar,
        );
        assert!(invalid.starts_with("HTTP/1.1 400"));
        assert_eq!(
            previews
                .lock()
                .unwrap()
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>(),
            before
        );
    }

    fn board_request(method: &str, path: &str, body: Value) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            headers: vec![
                ("Origin".into(), "http://127.0.0.1:1234".into()),
                ("Content-Type".into(), "application/json".into()),
            ],
            body: serde_json::to_vec(&body).unwrap(),
        }
    }

    #[test]
    fn board_routes_are_exact_and_browser_authority_overwrites_spoofs() {
        let id = "11111111-1111-4111-8111-111111111111";
        for (method, path, operation) in [
            (
                "POST",
                "/api/v1/local/board/categories/list",
                OfficeInvocation::BoardCategories,
            ),
            (
                "POST",
                "/api/v1/local/board/threads/list",
                OfficeInvocation::BoardList,
            ),
            (
                "POST",
                "/api/v1/local/board/threads/show",
                OfficeInvocation::BoardShow,
            ),
            (
                "POST",
                "/api/v1/local/board/threads",
                OfficeInvocation::BoardPost,
            ),
            (
                "POST",
                &format!("/api/v1/local/board/threads/{id}/replies"),
                OfficeInvocation::BoardReply,
            ),
            (
                "PUT",
                &format!("/api/v1/local/board/entries/{id}"),
                OfficeInvocation::BoardEdit,
            ),
            (
                "DELETE",
                &format!("/api/v1/local/board/entries/{id}"),
                OfficeInvocation::BoardDelete,
            ),
        ] {
            let request = board_request(
                method,
                path,
                json!({"actor":{"kind":"identity"},"threadId":"22222222-2222-4222-8222-222222222222","entryId":"22222222-2222-4222-8222-222222222222"}),
            );
            let (actual, value) = prepare_board_request(&request, "http://127.0.0.1:1234").unwrap();
            assert_eq!(actual, operation);
            if matches!(
                operation,
                OfficeInvocation::BoardPost
                    | OfficeInvocation::BoardReply
                    | OfficeInvocation::BoardEdit
                    | OfficeInvocation::BoardDelete
            ) {
                assert_eq!(value["actor"], json!({"kind":"owner"}));
            }
            if matches!(operation, OfficeInvocation::BoardReply) {
                assert_eq!(value["threadId"], id);
            }
            if matches!(
                operation,
                OfficeInvocation::BoardEdit | OfficeInvocation::BoardDelete
            ) {
                assert_eq!(value["entryId"], id);
            }
        }
    }

    #[test]
    fn board_routes_require_exact_origin_json_uuid_and_known_route() {
        let mut request = board_request("POST", "/api/v1/local/board/categories/list", json!({}));
        request.headers.clear();
        assert_eq!(
            prepare_board_request(&request, "http://127.0.0.1:1234")
                .unwrap_err()
                .0,
            403
        );
        for path in [
            "/api/v1/local/board/unknown",
            "/api/v1/local/board/entries/not-a-uuid",
        ] {
            assert_eq!(
                prepare_board_request(
                    &board_request("PUT", path, json!({})),
                    "http://127.0.0.1:1234"
                )
                .unwrap_err()
                .0,
                404
            );
        }
    }

    pub(super) fn call_api(
        request: Request,
        paths: &ConfigPaths,
        receipt: &ServiceReceipt,
    ) -> String {
        String::from_utf8(call_api_bytes(request, paths, receipt)).unwrap()
    }

    pub(super) fn call_api_bytes(
        request: Request,
        paths: &ConfigPaths,
        receipt: &ServiceReceipt,
    ) -> Vec<u8> {
        let previews = Arc::new(Mutex::new(HashMap::new()));
        call_handler_bytes(|stream| api(stream, request, paths, receipt, &previews))
    }

    #[test]
    fn authenticated_http_board_survives_handler_restart_and_storage_reopen() {
        let root = std::env::temp_dir().join(format!("tmt-board-http-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = ConfigPaths::resolve(&root, &root, Some(&root), None);
        let receipt = test_receipt();
        let headers = || {
            vec![
                ("Authorization".into(), "Bearer browser".into()),
                ("Origin".into(), "http://127.0.0.1:1234".into()),
                ("Content-Type".into(), "application/json".into()),
            ]
        };
        let post=Request{method:"POST".into(),path:"/api/v1/local/board/threads".into(),headers:headers(),body:serde_json::to_vec(&json!({"category":{"kind":"general"},"actor":{"kind":"identity","identityId":"11111111-1111-4111-8111-111111111111","name":"spoof"},"title":"hello","body":"body","operationId":"22222222-2222-4222-8222-222222222222"})).unwrap()};
        let posted = call_api(post, &paths, &receipt);
        assert!(posted.starts_with("HTTP/1.1 200"));
        let body = posted.split("\r\n\r\n").nth(1).unwrap();
        let value: Value = serde_json::from_str(body).unwrap();
        let thread = value["threadId"].as_str().unwrap();
        let show = Request {
            method: "POST".into(),
            path: "/api/v1/local/board/threads/show".into(),
            headers: headers(),
            body: serde_json::to_vec(
                &json!({"threadId":thread,"replyLimit":20,"replyCursor":null}),
            )
            .unwrap(),
        };
        let shown = call_api(show, &paths, &receipt);
        assert!(shown.starts_with("HTTP/1.1 200"));
        assert!(shown.contains("\"title\":\"hello\""));
        assert!(shown.contains("\"kind\":\"owner\""));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn protected_profile_http_uses_shared_cas_and_survives_reopen() {
        let root = std::env::temp_dir().join(format!("tmt-profile-http-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = ConfigPaths::resolve(&root, &root, Some(&root), None);
        let mut storage = Storage::open(&paths.database).unwrap();
        let id = tmt_core::identity::create_or_resolve(
            &mut storage,
            "Alice",
            tmt_core::identity::Lifetime::Saved,
        )
        .unwrap()
        .identity
        .id;
        let avatar = validate_avatar_pack(include_bytes!(
            "../../../contracts/avatar-pack-v1-sample.tmtavatar.json"
        ))
        .unwrap();
        storage.install_local_avatar_pack(0, &avatar).unwrap();
        let avatar_ref = format!("{}/{}", avatar.digest(), avatar.pack().avatars[0].key);
        let identity_status = tmt_core::identity_status::set_identity_status(
            &mut storage,
            &id,
            "Reviewing the room".into(),
            Some("focused".into()),
            tmt_adapters::request_runtime::wall_time_ms(),
            60_000,
        )
        .unwrap();
        storage.close().unwrap();
        let receipt = ServiceReceipt {
            schema_version: 1,
            pid: std::process::id(),
            port: 1234,
            nonce: "n".into(),
            browser_token: "browser".into(),
            control_token: "control".into(),
            running_version: "test".into(),
        };
        let headers = || {
            vec![
                ("Authorization".into(), "Bearer browser".into()),
                ("Origin".into(), "http://127.0.0.1:1234".into()),
                ("Content-Type".into(), "application/json".into()),
            ]
        };
        let unauthorized = Request {
            method: "GET".into(),
            path: format!("/api/v1/local/profiles/{id}"),
            headers: vec![],
            body: vec![],
        };
        assert!(call_api(unauthorized, &paths, &receipt).starts_with("HTTP/1.1 401"));
        let get = Request {
            method: "GET".into(),
            path: format!("/api/v1/local/profiles/{id}"),
            headers: headers(),
            body: vec![],
        };
        let missing = call_api(get, &paths, &receipt);
        assert!(missing.starts_with("HTTP/1.1 200"));
        assert!(missing.contains("\"exists\":false"));
        let catalog = call_api(
            Request {
                method: "GET".into(),
                path: "/api/v1/local/avatar-catalog".into(),
                headers: headers(),
                body: vec![],
            },
            &paths,
            &receipt,
        );
        assert!(catalog.starts_with("HTTP/1.1 200"));
        let catalog_body = response_value(&catalog);
        assert_eq!(catalog_body["catalogRevision"], 1);
        let packs = catalog_body["packs"].as_array().unwrap();
        // The bundled robot pack is projected alongside the installed fixture;
        // it does not consume an installation catalog revision.
        assert_eq!(packs.len(), 2);
        assert_eq!(packs[0]["pack"]["formatVersion"], 2);
        assert_eq!(packs[0]["pack"]["avatars"].as_array().unwrap().len(), 4);
        assert_eq!(packs[1]["digest"], avatar.digest());
        let profile = json!({"displayLabel":"","description":"Architecture review","appearance":{"hairStyle":"short","hairColor":"ink","skinTone":"medium","shirtColor":"blue","shirtMark":"AI"},"avatarRef":avatar_ref});
        let rejected = Request {
            method: "PUT".into(),
            path: format!("/api/v1/local/profiles/{id}"),
            headers: vec![
                ("Authorization".into(), "Bearer browser".into()),
                ("Origin".into(), "https://attacker.invalid".into()),
                ("Content-Type".into(), "application/json".into()),
            ],
            body: serde_json::to_vec(&json!({"expectedRevision":0,"profile":profile})).unwrap(),
        };
        assert!(call_api(rejected, &paths, &receipt).starts_with("HTTP/1.1 403"));
        let put = Request {
            method: "PUT".into(),
            path: format!("/api/v1/local/profiles/{id}"),
            headers: headers(),
            body: serde_json::to_vec(&json!({"expectedRevision":0,"profile":profile})).unwrap(),
        };
        let created = call_api(put, &paths, &receipt);
        assert!(created.starts_with("HTTP/1.1 200"));
        let json_body = |response: &str| {
            serde_json::from_str::<Value>(response.split("\r\n\r\n").nth(1).unwrap()).unwrap()
        };
        let created_body = json_body(&created);
        assert_eq!(created_body["revision"], 1);
        assert_eq!(created_body["changed"], true);
        assert!(created_body.get("selfReportedStatus").is_none());
        let created_at = created_body["updatedAtMs"].as_u64().unwrap();
        let retry = Request {
            method: "PUT".into(),
            path: format!("/api/v1/local/profiles/{id}"),
            headers: headers(),
            body: serde_json::to_vec(&json!({"expectedRevision":0,"profile":profile})).unwrap(),
        };
        let retried = call_api(retry, &paths, &receipt);
        assert!(retried.starts_with("HTTP/1.1 200"));
        let retried_body = json_body(&retried);
        assert_eq!(retried_body["revision"], 1);
        assert_eq!(retried_body["changed"], false);
        assert_eq!(retried_body["updatedAtMs"], created_at);
        let noop = Request {
            method: "PUT".into(),
            path: format!("/api/v1/local/profiles/{id}"),
            headers: headers(),
            body: serde_json::to_vec(&json!({"expectedRevision":1,"profile":profile})).unwrap(),
        };
        let noop_body = json_body(&call_api(noop, &paths, &receipt));
        assert_eq!(noop_body["changed"], false);
        assert_eq!(noop_body["updatedAtMs"], created_at);
        let updated_profile = json!({"displayLabel":"Lead","description":"Architecture review","appearance":{"hairStyle":"short","hairColor":"ink","skinTone":"medium","shirtColor":"blue","shirtMark":"AI"},"avatarRef":avatar_ref});
        let update = Request {
            method: "PUT".into(),
            path: format!("/api/v1/local/profiles/{id}"),
            headers: headers(),
            body: serde_json::to_vec(&json!({"expectedRevision":1,"profile":updated_profile}))
                .unwrap(),
        };
        let updated_body = json_body(&call_api(update, &paths, &receipt));
        assert_eq!(updated_body["revision"], 2);
        assert_eq!(updated_body["changed"], true);
        let unavailable_profile = json!({"displayLabel":"Lead","description":"Architecture review","appearance":{"hairStyle":"short","hairColor":"ink","skinTone":"medium","shirtColor":"blue","shirtMark":"AI"},"avatarRef":format!("sha256:{}/missing", "0".repeat(64))});
        let unavailable = Request {
            method: "PUT".into(),
            path: format!("/api/v1/local/profiles/{id}"),
            headers: headers(),
            body: serde_json::to_vec(&json!({"expectedRevision":2,"profile":unavailable_profile}))
                .unwrap(),
        };
        let unavailable_response = call_api(unavailable, &paths, &receipt);
        assert!(unavailable_response.starts_with("HTTP/1.1 409"));
        assert_eq!(
            response_value(&unavailable_response)["error"],
            "AVATAR_UNAVAILABLE"
        );
        let stale = Request {
            method: "PUT".into(),
            path: format!("/api/v1/local/profiles/{id}"),
            headers: headers(),
            body: serde_json::to_vec(&json!({"expectedRevision":1,"profile":profile})).unwrap(),
        };
        assert!(call_api(stale, &paths, &receipt).starts_with("HTTP/1.1 409"));
        let list = Request {
            method: "GET".into(),
            path: "/api/v1/local/profiles".into(),
            headers: headers(),
            body: vec![],
        };
        let listed = call_api(list, &paths, &receipt);
        assert!(listed.contains("\"presence\":\"offline\""));
        assert!(listed.contains("\"lifetime\":\"saved\""));
        assert!(listed.contains(&avatar_ref));
        assert_eq!(
            response_value(&listed)[0]["selfReportedStatus"],
            tmt_adapters::identity_status::status_value(
                Some(&identity_status),
                identity_status.updated_at_ms
            )
        );
        let mut storage = Storage::open(&paths.database).unwrap();
        tmt_core::identity_status::clear_identity_status(&mut storage, &id).unwrap();
        storage.close().unwrap();
        let cleared = call_api(
            Request {
                method: "GET".into(),
                path: "/api/v1/local/profiles".into(),
                headers: headers(),
                body: vec![],
            },
            &paths,
            &receipt,
        );
        assert_eq!(
            response_value(&cleared)[0]["selfReportedStatus"],
            Value::Null
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn request_parser_rejects_ambiguous_framing_and_headers() {
        let duplicate_host =
            parse_wire(b"GET /local HTTP/1.1\r\nHost: 127.0.0.1:1\r\nHost: attacker\r\n\r\n")
                .unwrap();
        assert_eq!(duplicate_host.header("host"), None);
        assert!(parse_wire(
            b"PUT /api/v1/local/world HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n"
        )
        .is_err());
        assert!(parse_wire(
            b"PUT /api/v1/local/world HTTP/1.1\r\nHost: 127.0.0.1:1\r\nTransfer-Encoding: chunked\r\n\r\n"
        )
        .is_err());
    }

    #[test]
    fn every_response_has_private_browser_security_headers() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let receiver = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).unwrap();
            String::from_utf8(bytes).unwrap()
        });
        let (mut stream, _) = listener.accept().unwrap();
        response(&mut stream, 200, "application/json", b"{}").unwrap();
        drop(stream);
        let output = receiver.join().unwrap();
        assert!(output.contains("Cache-Control: no-store\r\n"));
        assert!(output.contains("Content-Security-Policy:"));
        assert!(output.contains("Referrer-Policy: no-referrer\r\n"));
        assert!(!output.contains("Access-Control-Allow-Origin"));
    }
}
