//! Codex environment/index/header formats are implementation evidence only.
//! Unknown versions and missing, held or incompatible state silently refuse.
use crate::{
    runtime::lifecycle::{CallerSession, CallerSessionRefusal},
    skill_installation::ProviderEnvironment,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::{
    io::Read,
    os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

// Captured state_5.sqlite table shape; unknown schemas refuse. See fixture provenance.
const INDEX_COLUMNS: &[(&str, &str, i64, i64)] = &[
    ("id", "TEXT", 0, 1),
    ("rollout_path", "TEXT", 1, 0),
    ("created_at", "INTEGER", 1, 0),
    ("updated_at", "INTEGER", 1, 0),
    ("source", "TEXT", 1, 0),
    ("model_provider", "TEXT", 1, 0),
    ("cwd", "TEXT", 1, 0),
    ("title", "TEXT", 1, 0),
    ("sandbox_policy", "TEXT", 1, 0),
    ("approval_mode", "TEXT", 1, 0),
    ("tokens_used", "INTEGER", 1, 0),
    ("has_user_event", "INTEGER", 1, 0),
    ("archived", "INTEGER", 1, 0),
    ("archived_at", "INTEGER", 0, 0),
    ("git_sha", "TEXT", 0, 0),
    ("git_branch", "TEXT", 0, 0),
    ("git_origin_url", "TEXT", 0, 0),
    ("cli_version", "TEXT", 1, 0),
    ("first_user_message", "TEXT", 1, 0),
    ("agent_nickname", "TEXT", 0, 0),
    ("agent_role", "TEXT", 0, 0),
    ("memory_mode", "TEXT", 1, 0),
    ("model", "TEXT", 0, 0),
    ("reasoning_effort", "TEXT", 0, 0),
    ("agent_path", "TEXT", 0, 0),
    ("created_at_ms", "INTEGER", 0, 0),
    ("updated_at_ms", "INTEGER", 0, 0),
    ("thread_source", "TEXT", 0, 0),
    ("preview", "TEXT", 1, 0),
    ("recency_at", "INTEGER", 1, 0),
    ("recency_at_ms", "INTEGER", 1, 0),
    ("history_mode", "TEXT", 1, 0),
    ("name", "TEXT", 0, 0),
    ("is_pinned", "INTEGER", 1, 0),
    ("thread_section_id", "TEXT", 0, 0),
    ("section_position", "INTEGER", 0, 0),
    ("section_entered_at_ms", "INTEGER", 0, 0),
    ("project_id", "TEXT", 0, 0),
    ("originator", "TEXT", 0, 0),
    ("daybreak_enabled", "BOOLEAN", 0, 0),
    ("creator_user_id", "TEXT", 0, 0),
    ("creator_account_id", "TEXT", 0, 0),
];

pub(super) fn coordinates(session: Option<&std::ffi::OsStr>) -> Option<CallerSession> {
    let text = session?.to_str()?;
    if uuid::Uuid::parse_str(text).ok()?.to_string() != text {
        return None;
    }
    Some(CallerSession {
        session: tmt_core::binding::session::ProviderSessionId::new(text).ok()?,
        runtime_pid: None,
    })
}

fn sqlite_home(environment: &ProviderEnvironment) -> Option<PathBuf> {
    let home = super::codex_home(environment);
    let config = home.join("config.toml");
    let flags = nix::fcntl::OFlag::O_NOFOLLOW | nix::fcntl::OFlag::O_NONBLOCK;
    let file = match std::fs::File::options()
        .read(true)
        .custom_flags(flags.bits())
        .open(&config)
    {
        Ok(file) if file.metadata().ok()?.is_file() => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(sqlite_default(environment));
        }
        _ => return None,
    };
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 64 * 1024 {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    if document.get("profiles").is_some() {
        return None;
    }
    match document.get("sqlite_home") {
        None => Some(sqlite_default(environment)),
        Some(item) => {
            let path = Path::new(item.as_str()?);
            path.is_absolute().then(|| path.to_owned())
        }
    }
}

fn sqlite_default(environment: &ProviderEnvironment) -> PathBuf {
    // Codex config takes precedence over CODEX_SQLITE_HOME; the environment
    // fallback trims whitespace and resolves relative paths against its cwd.
    environment
        .var("CODEX_SQLITE_HOME")
        .and_then(|path| path.to_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map_or_else(
            || super::codex_home(environment),
            |path| environment.resolve(Path::new(path)),
        )
}

fn indexed_path(database: &Path, session: &str) -> Result<PathBuf, CallerSessionRefusal> {
    use CallerSessionRefusal::{IndexShape, IndexUnavailable, NotRoot};
    if !std::fs::symlink_metadata(database)
        .map_err(|_| IndexUnavailable)?
        .file_type()
        .is_file()
    {
        return Err(IndexUnavailable);
    }
    let wal = database.with_extension("sqlite-wal");
    let shm = database.with_extension("sqlite-shm");
    if wal.try_exists().map_err(|_| IndexUnavailable)?
        && !shm.try_exists().map_err(|_| IndexUnavailable)?
    {
        return Err(IndexUnavailable);
    }
    // Resolve directory aliases (macOS /var, or an explicitly relocated home),
    // after refusing a linked database itself. SQLite NOFOLLOW also checks
    // every ancestor, unlike the regular-file boundary above.
    let canonical = database.canonicalize().map_err(|_| IndexUnavailable)?;
    let encoded: String = canonical
        .as_os_str()
        .as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"/-._~".contains(byte) {
                (*byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    // OPEN_READONLY protects the database but SQLite may otherwise create or
    // write shared memory. Refuse unavailable WAL state instead of repairing it.
    let uri = format!("file:{encoded}?mode=ro&readonly_shm=1");
    let connection = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|_| IndexUnavailable)?;
    connection
        .busy_timeout(Duration::ZERO)
        .map_err(|_| IndexUnavailable)?;
    let mut shape = connection
        .prepare("PRAGMA table_info(threads)")
        .map_err(|_| IndexUnavailable)?;
    let columns = shape
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|_| IndexUnavailable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| IndexUnavailable)?;
    if columns.len() != INDEX_COLUMNS.len()
        || !columns.iter().zip(INDEX_COLUMNS).all(|(found, expected)| {
            found.0 == expected.0
                && found.1 == expected.1
                && found.2 == expected.2
                && found.3 == expected.3
        })
    {
        return Err(IndexShape);
    }
    // The only content SELECT is parameterized by this exact thread ID.
    let row = connection
        .query_row(
            "SELECT rollout_path, source FROM threads WHERE id = ?1",
            [session],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|_| IndexUnavailable)?
        .ok_or(IndexUnavailable)?;
    if matches!(row.1.as_str(), "cli" | "exec") {
        Ok(PathBuf::from(row.0))
    } else {
        Err(NotRoot)
    }
}

pub(super) fn root(
    coordinates: &CallerSession,
    environment: &ProviderEnvironment,
    deadline: Instant,
) -> Result<(), CallerSessionRefusal> {
    use CallerSessionRefusal::{Configuration, HeaderShape, HeaderUnavailable, NotRoot};
    let home = sqlite_home(environment).ok_or(Configuration)?;
    if Instant::now() >= deadline {
        return Err(HeaderUnavailable);
    }
    let path = indexed_path(&home.join("state_5.sqlite"), coordinates.session.as_str())?;
    let header = crate::runtime::caller_header::read(
        &super::codex_home(environment).join("sessions"),
        &path,
        deadline,
    )
    .ok_or(HeaderUnavailable)?;
    #[derive(serde::Deserialize)]
    struct Header {
        #[serde(rename = "type")]
        kind: String,
        payload: Metadata,
    }
    #[derive(serde::Deserialize)]
    struct Metadata {
        id: String,
        session_id: String,
        source: serde_json::Value,
        parent_thread_id: Option<String>,
    }
    let h = serde_json::from_slice::<Header>(&header).map_err(|_| HeaderShape)?;
    if h.kind != "session_meta" {
        return Err(HeaderShape);
    }
    if h.payload.id != coordinates.session.as_str()
        || h.payload.session_id != coordinates.session.as_str()
        || !matches!(h.payload.source.as_str(), Some("cli" | "exec"))
        || h.payload.parent_thread_id.is_some()
    {
        return Err(NotRoot);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
