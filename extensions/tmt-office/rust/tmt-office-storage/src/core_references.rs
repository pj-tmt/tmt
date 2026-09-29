//! UUID-keyed reads of core-owned identities and rooms used as Office
//! preflight. Office never reads core tables in its own transactions.
//!
//! These are same-user consistency checks, not an authorization boundary.
//! A reference retired between preflight and commit leaves the same state as
//! the serial order "Office commit, then retirement": identities are never
//! deleted, rooms are retired rather than removed, and core retirement never
//! changes Office rows. Production reads go through the invoking `tmt`
//! ([`ProcessReferences`]); Office never opens the core database for them.

use serde_json::{Value, json};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tmt_adapters::{
    process::{CommandFailure, CommandRequest, CommandRunner, UnixCommandRunner},
    storage::{StorageError, StorageErrorCode},
};
use tmt_core::identity::Lifetime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreIdentity {
    pub id: String,
    pub name: String,
    pub lifetime: Lifetime,
    pub retired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreRoom {
    pub id: String,
    pub retired: bool,
}

pub trait CoreReferences {
    /// Any identity with this UUID, including a retired one.
    fn identity(&self, id: &str) -> Result<Option<CoreIdentity>, StorageError>;
    /// Every active identity in core's canonical name order.
    fn active_identities(&self) -> Result<Vec<CoreIdentity>, StorageError>;
    /// Any room with this UUID, including a retired one.
    fn room(&self, id: &str) -> Result<Option<CoreRoom>, StorageError>;
    /// Many lookups at once, in input order; `None` means not found.
    #[allow(clippy::type_complexity)]
    fn resolve(
        &self,
        identities: &[String],
        rooms: &[String],
    ) -> Result<(Vec<Option<CoreIdentity>>, Vec<Option<CoreRoom>>), StorageError> {
        Ok((
            identities
                .iter()
                .map(|id| self.identity(id))
                .collect::<Result<_, _>>()?,
            rooms
                .iter()
                .map(|id| self.room(id))
                .collect::<Result<_, _>>()?,
        ))
    }
}

/// Core references through the invoking `tmt`: `tmt api references.resolve`
/// for UUID lookups (at most 256 per call) and `tmt identity list --json` for
/// the active roster. `TMT_EXECUTABLE` selects the executable; a direct run
/// falls back to `tmt` on PATH, never to this binary itself.
pub struct ProcessReferences {
    tmt: PathBuf,
}

const CORE_DEADLINE: Duration = Duration::from_secs(30);
const CORE_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
const RESOLVE_LIMIT: usize = 256;

fn unavailable(message: impl Into<String>) -> StorageError {
    StorageError::new(StorageErrorCode::Unknown, message.into())
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

impl ProcessReferences {
    pub fn discover() -> Result<Self, StorageError> {
        let tmt = match std::env::var_os("TMT_EXECUTABLE").map(PathBuf::from) {
            Some(path) if path.is_absolute() && executable(&path) => path,
            Some(_) => {
                return Err(unavailable(
                    "TMT_EXECUTABLE must select an executable absolute tmt path.",
                ));
            }
            None => std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|directory| directory.join("tmt"))
                .find(|candidate| executable(candidate))
                .ok_or_else(|| {
                    unavailable("Could not find tmt to read core identities and rooms.")
                })?,
        };
        if let (Ok(selected), Ok(this)) = (
            std::fs::canonicalize(&tmt),
            std::env::current_exe().and_then(std::fs::canonicalize),
        ) && selected == this
        {
            return Err(unavailable(
                "The selected tmt is this Office binary; refusing recursion.",
            ));
        }
        let references = Self { tmt };
        // Core initializes and migrates its own storage on first use; Office
        // never creates or opens the core database for these reads.
        references.api("references.resolve", json!({}))?;
        Ok(references)
    }

    fn run(&self, args: &[OsString], input: &[u8]) -> Result<Value, StorageError> {
        let output = match UnixCommandRunner.execute(CommandRequest {
            program: self.tmt.as_os_str(),
            args,
            input,
            deadline: Instant::now() + CORE_DEADLINE,
            max_output_bytes: CORE_OUTPUT_LIMIT,
        }) {
            Ok(output) => output.stdout,
            Err(error) => {
                let stdout = error.output.as_ref().map(|output| output.stdout.clone());
                let message = stdout
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .and_then(|value| value["error"]["message"].as_str().map(str::to_owned));
                let code = if matches!(error.kind, CommandFailure::Timeout) {
                    StorageErrorCode::Busy
                } else {
                    StorageErrorCode::Unknown
                };
                return Err(StorageError::new(
                    code,
                    message.unwrap_or_else(|| "Core could not be reached through tmt.".into()),
                ));
            }
        };
        serde_json::from_slice(&output).map_err(|_| unavailable("tmt returned no JSON document."))
    }

    fn api(&self, operation: &str, input: Value) -> Result<Value, StorageError> {
        let request = json!({"version": 1, "operation": operation, "input": input}).to_string();
        self.run(&[OsString::from("api")], request.as_bytes())
    }
}

fn lifetime(value: &Value) -> Result<Lifetime, StorageError> {
    match value.as_str() {
        Some("saved") => Ok(Lifetime::Saved),
        Some("temporary") => Ok(Lifetime::Temporary),
        _ => Err(unavailable("Core reported an unknown identity lifetime.")),
    }
}

fn identity_state(value: &Value) -> Result<Option<CoreIdentity>, StorageError> {
    if value["found"] != true {
        return Ok(None);
    }
    Ok(Some(CoreIdentity {
        id: value["id"].as_str().unwrap_or_default().to_owned(),
        name: value["name"].as_str().unwrap_or_default().to_owned(),
        lifetime: lifetime(&value["lifetime"])?,
        retired: value["retired"].as_bool().unwrap_or(true),
    }))
}

impl CoreReferences for ProcessReferences {
    fn identity(&self, id: &str) -> Result<Option<CoreIdentity>, StorageError> {
        Ok(self.resolve(&[id.to_owned()], &[])?.0.pop().flatten())
    }

    fn active_identities(&self) -> Result<Vec<CoreIdentity>, StorageError> {
        let args: Vec<OsString> = ["--json", "identity", "list"].map(OsString::from).to_vec();
        let value = self.run(&args, b"")?;
        value["identities"]
            .as_array()
            .ok_or_else(|| unavailable("Core identity list omitted identities."))?
            .iter()
            .map(|identity| {
                Ok(CoreIdentity {
                    id: identity["id"].as_str().unwrap_or_default().to_owned(),
                    name: identity["name"].as_str().unwrap_or_default().to_owned(),
                    lifetime: lifetime(&identity["lifetime"])?,
                    retired: false,
                })
            })
            .collect()
    }

    fn room(&self, id: &str) -> Result<Option<CoreRoom>, StorageError> {
        Ok(self.resolve(&[], &[id.to_owned()])?.1.pop().flatten())
    }

    fn resolve(
        &self,
        identities: &[String],
        rooms: &[String],
    ) -> Result<(Vec<Option<CoreIdentity>>, Vec<Option<CoreRoom>>), StorageError> {
        // Non-canonical UUIDs name nothing, as the former direct reads did.
        let canonical = |id: &String| tmt_core::dispatch::canonical_id(id);
        let mut found_identities = vec![None; identities.len()];
        let mut found_rooms = vec![None; rooms.len()];
        let wanted_identities: Vec<(usize, &String)> = identities
            .iter()
            .enumerate()
            .filter(|(_, id)| canonical(id))
            .collect();
        let wanted_rooms: Vec<(usize, &String)> = rooms
            .iter()
            .enumerate()
            .filter(|(_, id)| canonical(id))
            .collect();
        for chunk in wanted_identities.chunks(RESOLVE_LIMIT) {
            let ids: Vec<&String> = chunk.iter().map(|(_, id)| *id).collect();
            let value = self.api("references.resolve", json!({"identityIds": ids}))?;
            let states = value["identities"]
                .as_array()
                .filter(|states| states.len() == chunk.len())
                .ok_or_else(|| unavailable("Core returned an incomplete reference page."))?;
            for ((index, _), state) in chunk.iter().zip(states) {
                found_identities[*index] = identity_state(state)?;
            }
        }
        for chunk in wanted_rooms.chunks(RESOLVE_LIMIT) {
            let ids: Vec<&String> = chunk.iter().map(|(_, id)| *id).collect();
            let value = self.api("references.resolve", json!({"roomIds": ids}))?;
            let states = value["rooms"]
                .as_array()
                .filter(|states| states.len() == chunk.len())
                .ok_or_else(|| unavailable("Core returned an incomplete reference page."))?;
            for ((index, id), state) in chunk.iter().zip(states) {
                if state["found"] == true {
                    found_rooms[*index] = Some(CoreRoom {
                        id: (*id).clone(),
                        retired: state["retired"].as_bool().unwrap_or(true),
                    });
                }
            }
        }
        Ok((found_identities, found_rooms))
    }
}

#[cfg(any(test, feature = "in-process-core"))]
pub mod in_process;
#[cfg(any(test, feature = "in-process-core"))]
pub use in_process::CoreStore;

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    /// SQL fragments that would read or write core-owned tables.
    const CORE_TABLE_SQL: &[&str] = &[
        "FROM identities",
        "JOIN identities",
        "INTO identities",
        "UPDATE identities",
        "office_meeting_rooms",
        "office_meeting_members",
        "extension_storage_cutovers",
    ];

    /// Only the migration coordinator compares or copies against the core
    /// schema; every other module reaches core through [`super::CoreReferences`].
    /// `migration/switch.rs` alone writes core's cutover receipt, inside its
    /// decision transaction.
    const MIGRATION_MODULES: &[&str] = &[
        "cells.rs",
        "migration.rs",
        "migration/switch.rs",
        "schema.rs",
    ];

    fn visit(root: &Path, directory: &Path, found: &mut Vec<String>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, found);
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let name = path.file_name().unwrap().to_string_lossy();
            if !relative.ends_with(".rs")
                || MIGRATION_MODULES.contains(&relative.as_str())
                || name == "tests.rs"
                || relative.contains("tests/")
                || name.ends_with("_tests.rs")
                || name == "test_support.rs"
            {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            let production = text.split("#[cfg(test)]").next().unwrap_or(&text);
            for pattern in CORE_TABLE_SQL {
                if production.contains(pattern) {
                    found.push(format!("{relative}: {pattern}"));
                }
            }
        }
    }

    /// Office reaches core storage only through the invoking `tmt`; the
    /// migration coordinator is the single, documented exception.
    #[test]
    fn office_code_never_opens_core_storage_in_process() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        let mut pending = vec![root.clone()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    files.push(path);
                }
            }
        }
        let mut found = Vec::new();
        for path in files {
            let relative = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let name = path.file_name().unwrap().to_string_lossy();
            // The in-process port compiles only for tests (see its header).
            if name == "tests.rs"
                || relative.starts_with("tests/")
                || name == "test_support.rs"
                || name == "in_process.rs"
            {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            let production = text.split("#[cfg(test)]").next().unwrap_or(&text);
            for pattern in ["Storage::open", "CoreStore::open"] {
                if production.contains(pattern) {
                    found.push(format!("{relative}: {pattern}"));
                }
            }
        }
        assert_eq!(found, Vec::<String>::new());
    }

    #[test]
    fn office_code_outside_migration_never_runs_sql_on_core_tables() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        visit(&root, &root, &mut found);
        assert_eq!(found, Vec::<String>::new());
    }
}
