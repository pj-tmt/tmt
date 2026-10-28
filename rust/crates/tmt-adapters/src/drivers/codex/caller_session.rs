//! Codex environment/index/header formats are implementation evidence only.
//! Unknown versions and missing, held or incompatible state silently refuse.
use crate::{
    runtime::lifecycle::{CallerSession, CallerSessionRefusal},
    skill_installation::ProviderEnvironment,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::{
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

// The `threads` columns the content query reads, with their captured shape
// (name, type, not-null, primary key). Each must appear exactly once; other
// columns are additive and ignored (#5494). See fixture provenance.
const READ_COLUMNS: &[(&str, &str, i64, i64)] = &[
    ("id", "TEXT", 0, 1),
    ("rollout_path", "TEXT", 1, 0),
    ("source", "TEXT", 1, 0),
];
// Codex 0.165 links a thread attached to a parent here. When the column
// exists it is read, and a linked thread is not a root caller session.
const PARENT_COLUMN: (&str, &str, i64, i64) = ("attached_thread_parent_id", "TEXT", 0, 0);

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

/// Constructed only after the exact main-thread header and native caller were
/// verified. Hook starts retain their stricter provider-reported transition.
pub(super) struct Observation(pub(super) tmt_core::binding::session::ProviderSessionId);

impl crate::runtime::lifecycle::LifecycleObservation for Observation {
    fn session(&self) -> &tmt_core::binding::session::ProviderSessionId {
        &self.0
    }
    fn starting(&self) -> bool {
        true
    }
    fn driver_state(
        &self,
        previous: Option<&tmt_core::binding::session::DriverState>,
        now_ms: u64,
    ) -> Option<tmt_core::binding::session::DriverState> {
        // Discovery reports no model, so it clears the observed one.
        crate::runtime::driver_state::after_start(None, None, previous, now_ms)
    }
    fn propose(
        &self,
        current: &tmt_core::binding::session::BindingSessionState,
        process: &tmt_core::endpoint::ProcessIncarnation,
        previous: tmt_core::binding::session::RuntimeLiveness,
        host: crate::runtime::lifecycle::HostEvidence,
        owned_resume: bool,
    ) -> Option<tmt_core::binding::session::BindingSessionState> {
        use tmt_core::binding::session::{RuntimeState, SessionTransition};
        let turnover = current.state == RuntimeState::Running
            && current.key.as_ref().is_some_and(|key| {
                key.incarnation == *process
                    && key
                        .provider_session
                        .as_ref()
                        .is_some_and(|session| session != &self.0)
            });
        super::CodexObservation {
            session: self.0.clone(),
            model: None,
            starting: true,
            transition: if turnover {
                SessionTransition::Resumed
            } else {
                SessionTransition::Started
            },
        }
        .propose_with_resume(current, process, previous, host.shared(), owned_resume)
    }
}

fn sqlite_home(environment: &ProviderEnvironment) -> Option<PathBuf> {
    let home = super::codex_home(environment);
    let config = home.join("config.toml");
    let bytes = match crate::bounded_file::read_no_follow(&config, 64 * 1024) {
        Ok(bytes) => bytes,
        Err(crate::bounded_file::FileReadError::Io(error))
            if error.kind() == std::io::ErrorKind::NotFound =>
        {
            return Some(sqlite_default(environment));
        }
        _ => return None,
    };
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
    let shaped = |expected: &(&str, &str, i64, i64)| -> Result<bool, CallerSessionRefusal> {
        let mut found = columns.iter().filter(|column| column.0 == expected.0);
        match (found.next(), found.next()) {
            (None, _) => Ok(false),
            (Some(column), None)
                if column.1 == expected.1 && column.2 == expected.2 && column.3 == expected.3 =>
            {
                Ok(true)
            }
            _ => Err(IndexShape),
        }
    };
    for expected in READ_COLUMNS {
        if !shaped(expected)? {
            return Err(IndexShape);
        }
    }
    let parent = if shaped(&PARENT_COLUMN)? {
        "attached_thread_parent_id IS NOT NULL"
    } else {
        "0"
    };
    // The only content SELECT is parameterized by this exact thread ID.
    let row = connection
        .query_row(
            &format!("SELECT rollout_path, source, {parent} FROM threads WHERE id = ?1"),
            [session],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|_| IndexUnavailable)?
        .ok_or(IndexUnavailable)?;
    if row.2 {
        return Err(NotRoot);
    }
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
