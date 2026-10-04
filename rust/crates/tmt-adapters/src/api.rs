//! Versioned local process protocol. Resource owners retain admission and encoding.

mod changes;
mod consumption;
mod dispatch;
mod identities;
mod identity_hooks;
mod notes;
mod references;
mod requests;
mod rooms;
mod skills;

use crate::{
    config::{ConfigFiles, ConfigPaths},
    skill_installation,
    storage::{Storage, StorageError},
};
use serde::Deserialize;
use serde_json::{json, value::RawValue};
use tmt_core::{
    dispatch::DispatchInput, identity::Identity, identity_hooks::IdentityHook,
    request::history::HistoryQuery, room::RoomWrite,
};

// Preserve the canonical message limit even when every byte is JSON-escaped.
pub const INPUT_LIMIT: usize = crate::dispatch::INPUT_LIMIT + 4096;
pub const OUTPUT_LIMIT: usize = 12 * 1_048_576 + 65_536;
const OPS: &[&str] = &[
    "capabilities",
    "storage.root",
    "changes.cursor",
    "requests.list",
    "requests.show",
    "dispatch.show",
    "dispatch.create",
    "rooms.write",
    "rooms.retire",
    "rooms.roster",
    "notes.read",
    "identityHooks.register",
    "identityHooks.pending",
    "identityHooks.attempt",
    "identityHooks.ack",
    "skills.install",
    "skills.remove",
    "references.resolve",
    "identities.status",
    "consumption.history",
];

#[derive(Debug)]
pub struct Fault {
    code: &'static str,
    message: std::borrow::Cow<'static, str>,
    storage_open: Option<StorageError>,
}
impl Fault {
    pub fn new(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message: message.into(),
            storage_open: None,
        }
    }
    /// A fault whose message names the specific skill, path or owner.
    pub fn detailed(code: &'static str, message: String) -> Self {
        Self {
            code,
            message: message.into(),
            storage_open: None,
        }
    }
    pub fn unavailable() -> Self {
        Self::new(
            "API_UNAVAILABLE",
            "Operation could not be confirmed. For a write, inspect its operation receipt or current revision before retrying.",
        )
    }
    pub(super) fn with_storage_open(mut self, error: StorageError) -> Self {
        self.storage_open = Some(error);
        self
    }

    /// Public CLI presentation consumes the typed cause; other API consumers
    /// retain the existing resource-specific fault envelope.
    pub fn take_storage_open(&mut self) -> Option<StorageError> {
        self.storage_open.take()
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut error = json!({"code":self.code,"message":self.message});
        if self.code == "API_VERSION_UNSUPPORTED" {
            error["supported"] = json!({"min":1,"max":1});
        }
        serde_json::to_vec(&json!({"error":error})).expect("bounded error")
    }
}
fn invalid() -> Fault {
    Fault::new(
        "API_INPUT_INVALID",
        "Invalid operation, fields or bounded input.",
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u64,
    operation: String,
    #[serde(default)]
    identity: Option<String>,
    /// `"anonymous"`: no writer identity, as the CLI without `--identity`.
    #[serde(default)]
    originator: Option<String>,
    input: Box<RawValue>,
}
/// Local originator selection. Saved IDs never fall back to display names.
pub enum DispatchIdentity {
    Explicit(String),
    SavedId(String),
}

pub enum Request {
    Capabilities,
    ConsumptionHistory {
        identities: Vec<String>,
        windows: Vec<u64>,
        max_buckets: u64,
    },
    /// Read-only: the selected data directory, without opening storage.
    StorageRoot,
    /// Read-only: the durable change cursor.
    ChangeCursor,
    History(HistoryQuery),
    Detail(String),
    Receipt(String),
    Dispatch {
        /// `None` is the anonymous originator.
        identity: Option<DispatchIdentity>,
        input: DispatchInput,
    },
    Room {
        /// `None` is the anonymous originator.
        identity: Option<String>,
        id: String,
        input: RoomWrite,
    },
    RoomRetire {
        /// `None` is the anonymous originator.
        identity: Option<String>,
        id: String,
        expected_revision: u64,
    },
    Notes(String),
    Roster {
        room: String,
        prefix: Option<String>,
    },
    /// Identity-retirement hooks, always scoped to the named consumer.
    HookRegister(IdentityHook),
    HookPending {
        consumer: String,
        limit: usize,
    },
    HookAttempt(IdentityHook),
    HookAck(IdentityHook),
    /// Extension-owned skills; the caller has obtained the user's consent.
    SkillsInstall {
        owner: String,
        skills: Vec<skill_installation::OwnedSkill>,
        force: bool,
    },
    SkillsRemove {
        owner: String,
        skills: Option<Vec<String>>,
    },
    /// Read-only states of identities and rooms named by UUID.
    References {
        identities: Vec<String>,
        rooms: Vec<String>,
    },
    /// Read-only self-reported statuses of identities named by UUID.
    IdentityStatuses {
        identities: Vec<String>,
    },
}

/// Bound on UUIDs per `references.resolve` call, identities and rooms together.
const REFERENCE_LIMIT: usize = 256;

pub fn decode(body: &str) -> Result<Request, Fault> {
    if body.len() > INPUT_LIMIT {
        return Err(invalid());
    }
    let wire: Envelope = serde_json::from_str(body).map_err(|_| invalid())?;
    if wire.version != 1 {
        return Err(Fault::new(
            "API_VERSION_UNSUPPORTED",
            "Unsupported API major version.",
        ));
    }
    let input = wire.input.get().as_bytes();
    let writing = matches!(
        wire.operation.as_str(),
        "dispatch.create" | "rooms.write" | "rooms.retire"
    );
    // A write names exactly one originator: an identity or `"anonymous"`. Other
    // operations name neither.
    let anonymous = match wire.originator.as_deref() {
        None => false,
        Some("anonymous") => true,
        Some(_) => return Err(invalid()),
    };
    let valid_originator = if writing {
        wire.identity.is_some() != anonymous
    } else {
        wire.identity.is_none() && !anonymous
    };
    if !valid_originator
        || wire
            .identity
            .as_ref()
            .is_some_and(|id| id.trim().is_empty() || id.len() > 256)
    {
        return Err(invalid());
    }
    Ok(match wire.operation.as_str() {
        "capabilities"
            if serde_json::from_slice::<serde_json::Value>(input).is_ok_and(|v| v == json!({})) =>
        {
            Request::Capabilities
        }
        "storage.root"
            if serde_json::from_slice::<serde_json::Value>(input).is_ok_and(|v| v == json!({})) =>
        {
            Request::StorageRoot
        }
        "changes.cursor"
            if serde_json::from_slice::<serde_json::Value>(input).is_ok_and(|v| v == json!({})) =>
        {
            Request::ChangeCursor
        }
        "requests.list" => Request::History(requests::decode_list(input)?),
        "requests.show" => Request::Detail(requests::decode_show(input)?),
        "dispatch.show" => Request::Receipt(dispatch::decode_show(input)?),
        "dispatch.create" => Request::Dispatch {
            identity: wire.identity.map(DispatchIdentity::Explicit),
            input: dispatch::decode_create(input)?,
        },
        "rooms.write" => rooms::decode_write(wire)?,
        "rooms.retire" => rooms::decode_retire(wire)?,
        "rooms.roster" => rooms::decode_roster(input)?,
        "identityHooks.register" => Request::HookRegister(identity_hooks::hook(input)?),
        "identityHooks.attempt" => Request::HookAttempt(identity_hooks::hook(input)?),
        "identityHooks.ack" => Request::HookAck(identity_hooks::hook(input)?),
        "references.resolve" => references::decode(input)?,
        "identities.status" => identities::decode(input)?,
        "consumption.history" => consumption::decode(input)?,
        "identityHooks.pending" => identity_hooks::decode_pending(input)?,
        "skills.install" => skills::decode_install(input)?,
        "skills.remove" => skills::decode_remove(input)?,
        "notes.read" => notes::decode(input)?,
        _ => return Err(invalid()),
    })
}

pub fn capabilities() -> Vec<u8> {
    serde_json::to_vec(&json!({"version":1,"supported":{"min":1,"max":1},"operations":OPS,"limits":{"inputBytes":INPUT_LIMIT,"outputBytes":OUTPUT_LIMIT},"commands":["identity","list","room","x","reply","result","notes"]})).expect("constant capabilities")
}

fn identity(storage: &mut Storage, selector: &str) -> Result<String, Fault> {
    Ok(identity_entry(storage, selector)?.id)
}

fn identity_entry(storage: &mut Storage, selector: &str) -> Result<Identity, Fault> {
    let found = storage
        .resolve_identity(selector)
        .map_err(|_| Fault::unavailable())?;
    found.ok_or_else(|| {
        Fault::new(
            "NAME_NOT_FOUND",
            "Explicit originator identity was not found.",
        )
    })
}

pub fn execute(paths: &ConfigPaths, request: Request) -> Result<Vec<u8>, Fault> {
    if matches!(request, Request::Capabilities) {
        return Ok(capabilities());
    }
    if matches!(request, Request::StorageRoot) {
        let root = crate::config::normalize(
            &std::path::absolute(&paths.global_dir).map_err(|_| Fault::unavailable())?,
        );
        let root = root.to_str().ok_or_else(Fault::unavailable)?;
        return serde_json::to_vec(&json!({"dataRoot": root})).map_err(|_| Fault::unavailable());
    }
    if let Request::SkillsInstall {
        owner,
        skills,
        force,
    } = request
    {
        return skills::install(&paths.global_dir, &owner, &skills, force);
    }
    if let Request::SkillsRemove { owner, skills } = request {
        return skills::remove(&paths.global_dir, &owner, skills);
    }
    if let Request::Notes(id) = request {
        return notes::read(paths, id);
    }
    // Only dispatch uses settings; read operations do not depend on unrelated config.
    let settings = if matches!(request, Request::Dispatch { .. }) {
        Some(
            ConfigFiles {
                paths: paths.clone(),
            }
            .load()
            .map_err(|_| Fault::new("CONFIG_ERROR", "Could not load dispatch settings."))?
            .settings,
        )
    } else {
        None
    };
    let mut storage = Storage::open(&paths.database)
        .map_err(|error| Fault::unavailable().with_storage_open(error))?;
    let pending = match request {
        Request::Capabilities
        | Request::StorageRoot
        | Request::Notes(_)
        | Request::SkillsInstall { .. }
        | Request::SkillsRemove { .. } => unreachable!("handled before storage"),
        Request::ConsumptionHistory {
            identities,
            windows,
            max_buckets,
        } => consumption::history(&mut storage, identities, windows, max_buckets),
        Request::ChangeCursor => changes::cursor(&storage),
        Request::Roster { room, prefix } => rooms::roster(&storage, room, prefix),
        Request::References { identities, rooms } => {
            references::resolve(&mut storage, identities, rooms)
        }
        Request::IdentityStatuses { identities } => identities::status(&storage, identities),
        Request::HookRegister(hook) => identity_hooks::register(&mut storage, hook),
        Request::HookPending { consumer, limit } => {
            identity_hooks::pending(&storage, consumer, limit)
        }
        Request::HookAttempt(hook) => identity_hooks::attempt(&mut storage, hook),
        Request::HookAck(hook) => identity_hooks::ack(&mut storage, hook),
        Request::History(query) => requests::list_history(&mut storage, query),
        Request::Detail(id) => requests::show_request(&mut storage, id),
        Request::Receipt(id) => dispatch::show_receipt(&storage, id),
        Request::Room {
            identity: selector,
            id,
            input,
        } => rooms::write(&mut storage, selector, id, input),
        Request::RoomRetire {
            identity: selector,
            id,
            expected_revision,
        } => rooms::retire(&mut storage, selector, id, expected_revision),
        Request::Dispatch {
            identity: selector,
            input,
        } => dispatch::create_dispatch(&mut storage, selector, input, settings),
    };
    let closed = storage.close();
    let body = pending?;
    closed.map_err(|_| Fault::unavailable())?;
    if body.len() > OUTPUT_LIMIT {
        return Err(Fault::new(
            "API_OUTPUT_TOO_LARGE",
            "Response exceeds the API bound; request a smaller page.",
        ));
    }
    Ok(body)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn storage_root_reports_selected_directory_without_creating_or_opening_state() {
        let directory = crate::test_support::TestDirectory::new();
        let selected = directory.path.join("selected-data");
        let paths = ConfigPaths::resolve(
            &directory.path,
            &directory.path,
            Some(&selected),
            Some(&directory.path.join("ignored-xdg")),
        );
        let request = || decode(r#"{"version":1,"operation":"storage.root","input":{}}"#).unwrap();
        let body = execute(&paths, request()).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"dataRoot": selected})
        );
        assert!(
            !selected.exists(),
            "discovery must not create the data directory"
        );
        assert_eq!(std::fs::read_dir(&directory.path).unwrap().count(), 0);

        // Existing invalid core files cannot affect discovery and remain untouched.
        std::fs::create_dir(&selected).unwrap();
        std::fs::write(&paths.database, b"not a SQLite database").unwrap();
        std::fs::write(&paths.global_config, b"not JSON").unwrap();
        assert_eq!(execute(&paths, request()).unwrap(), body);
        assert_eq!(
            std::fs::read(&paths.database).unwrap(),
            b"not a SQLite database"
        );
        assert_eq!(std::fs::read(&paths.global_config).unwrap(), b"not JSON");
        assert_eq!(std::fs::read_dir(&selected).unwrap().count(), 2);
        assert!(capabilities_list().contains(&"storage.root".to_owned()));
        assert!(
            serde_json::from_slice::<Value>(&capabilities())
                .unwrap()
                .get("dataRoot")
                .is_none()
        );
    }

    #[test]
    fn storage_root_requires_empty_input_and_no_originator() {
        for body in [
            r#"{"version":1,"operation":"storage.root","input":{"path":"/tmp"}}"#,
            r#"{"version":1,"operation":"storage.root","input":null}"#,
            r#"{"version":1,"operation":"storage.root","input":[]}"#,
            r#"{"version":1,"operation":"storage.root","identity":"Ada","input":{}}"#,
            r#"{"version":1,"operation":"storage.root","originator":"anonymous","input":{}}"#,
            r#"{"version":1,"operation":"storage.root","input":{},"extra":true}"#,
        ] {
            assert!(
                matches!(
                    decode(body),
                    Err(Fault {
                        code: "API_INPUT_INVALID",
                        ..
                    })
                ),
                "{body}"
            );
        }
    }

    #[test]
    fn storage_root_makes_a_selected_relative_path_absolute_without_canonicalizing() {
        let directory = crate::test_support::TestDirectory::new();
        let paths = ConfigPaths::resolve(
            &directory.path,
            &directory.path,
            Some(std::path::Path::new("missing-parent/../selected-data")),
            None,
        );
        let body = execute(&paths, Request::StorageRoot).unwrap();
        let result: Value = serde_json::from_slice(&body).unwrap();
        let expected = std::env::current_dir().unwrap().join("selected-data");
        assert_eq!(result, json!({"dataRoot": expected}));
        assert_eq!(std::fs::read_dir(&directory.path).unwrap().count(), 0);
    }

    #[test]
    fn version_and_nested_admission_preserve_strict_resource_decoders() {
        let bad = [
            r#"{"version":1,"operation":"rooms.write","identity":"A","input":{"roomId":"00000000-0000-4000-8000-000000000001","room":{"expectedRevision":0,"name":"A","name":"B","memberIds":[]}}}"#,
            r#"{"version":1,"operation":"capabilities","identity":"A","input":{}}"#,
            r#"{"version":1,"operation":"notes.read","input":{"identityId":"../../notes"}}"#,
            r#"{"version":1,"operation":"requests.list","input":{"limit":1}}"#,
            r#"{"version":1,"operation":"unknown","input":{}}"#,
        ];
        for body in bad {
            assert!(matches!(
                decode(body),
                Err(Fault {
                    code: "API_INPUT_INVALID",
                    ..
                })
            ));
        }
        let error = decode(r#"{"version":2,"operation":"capabilities","input":{}}"#)
            .err()
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&error.encode()).unwrap()["error"]["supported"],
            json!({"min":1,"max":1})
        );
    }

    #[test]
    fn identity_hooks_are_scoped_to_their_consumer_and_keep_lifecycle_semantics() {
        let directory = crate::test_support::TestDirectory::new();
        let paths = ConfigPaths::resolve(
            &directory.path,
            &directory.path,
            Some(&directory.path),
            None,
        );
        let call = |operation: &str,
                    input: serde_json::Value|
         -> Result<serde_json::Value, String> {
            let body = json!({"version": 1, "operation": operation, "input": input}).to_string();
            let request = decode(&body).map_err(|fault| fault.code.to_owned())?;
            execute(&paths, request)
                .map(|bytes| serde_json::from_slice(&bytes).unwrap())
                .map_err(|fault| fault.code.to_owned())
        };
        let active = "11111111-1111-4111-8111-111111111111";
        let retired = "22222222-2222-4222-8222-222222222222";
        {
            Storage::open(&paths.database).unwrap().close().unwrap();
            rusqlite::Connection::open(&paths.database).unwrap().execute_batch(&format!(
                "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES ('{active}', 'Ada', 'ada', 't', 't', 'saved'), ('{retired}', 'Old', 'old', 't', 't', 'saved');
                 UPDATE identities SET retired_at_ms = 5 WHERE id = '{retired}';"
            )).unwrap();
        }
        let hook = |consumer: &str, id: &str| json!({"consumer": consumer, "identityId": id, "reference": "scope-a"});
        assert_eq!(
            call("identityHooks.register", hook("tmt-office", active)).unwrap(),
            json!({"state": "registered"})
        );
        // Registration after retirement is pending at once.
        assert_eq!(
            call("identityHooks.register", hook("tmt-office", retired)).unwrap(),
            json!({"state": "pending"})
        );
        assert_eq!(
            call("identityHooks.register", hook("other", retired)).unwrap(),
            json!({"state": "pending"})
        );
        assert_eq!(
            call(
                "identityHooks.register",
                hook("tmt-office", "33333333-3333-4333-8333-333333333333")
            ),
            Err("IDENTITY_NOT_FOUND".into())
        );

        // Each consumer sees and settles only its own hooks.
        let page = call(
            "identityHooks.pending",
            json!({"consumer": "tmt-office", "limit": 16}),
        )
        .unwrap();
        assert_eq!(
            page,
            json!({"hooks": [{"identityId": retired, "reference": "scope-a", "attemptCount": 0}], "pending": 1})
        );
        assert_eq!(
            call("identityHooks.attempt", hook("tmt-office", retired)).unwrap(),
            json!({"recorded": true})
        );
        assert_eq!(
            call("identityHooks.ack", hook("third", retired)),
            Err("HOOK_NOT_FOUND".into())
        );
        assert_eq!(
            call("identityHooks.attempt", hook("tmt-office", active)),
            Err("HOOK_NOT_PENDING".into())
        );
        assert_eq!(
            call("identityHooks.ack", hook("tmt-office", retired)).unwrap(),
            json!({"acknowledged": true})
        );
        // Delivered is terminal: repeats change nothing, and re-registration keeps it delivered.
        assert_eq!(
            call("identityHooks.ack", hook("tmt-office", retired)).unwrap(),
            json!({"acknowledged": false})
        );
        assert_eq!(
            call("identityHooks.attempt", hook("tmt-office", retired)).unwrap(),
            json!({"recorded": false})
        );
        assert_eq!(
            call("identityHooks.register", hook("tmt-office", retired)).unwrap(),
            json!({"state": "delivered"})
        );
        assert_eq!(
            call(
                "identityHooks.pending",
                json!({"consumer": "tmt-office", "limit": 16})
            )
            .unwrap(),
            json!({"hooks": [], "pending": 0})
        );
        assert_eq!(
            call(
                "identityHooks.pending",
                json!({"consumer": "other", "limit": 16})
            )
            .unwrap()["pending"],
            1
        );

        for (operation, input) in [
            (
                "identityHooks.pending",
                json!({"consumer": "tmt-office", "limit": 0}),
            ),
            (
                "identityHooks.pending",
                json!({"consumer": "tmt-office", "limit": 17}),
            ),
            (
                "identityHooks.pending",
                json!({"consumer": "Office", "limit": 1}),
            ),
            ("identityHooks.pending", json!({"limit": 1})),
            (
                "identityHooks.attempt",
                json!({"consumer": "tmt-office", "identityId": "not-a-uuid", "reference": "r"}),
            ),
            (
                "identityHooks.ack",
                json!({"consumer": "tmt-office", "identityId": active, "reference": "r", "extra": 1}),
            ),
            (
                "identityHooks.register",
                json!({"consumer": "", "identityId": active, "reference": "r"}),
            ),
        ] {
            assert_eq!(
                call(operation, input.clone()),
                Err("API_INPUT_INVALID".into()),
                "{operation} {input}"
            );
        }
        // Hook operations are not identity-attributed writes.
        assert!(decode(&json!({"version": 1, "operation": "identityHooks.ack", "identity": "Ada", "input": hook("tmt-office", active)}).to_string()).is_err());
        let capabilities: serde_json::Value = serde_json::from_slice(&capabilities()).unwrap();
        for operation in [
            "identityHooks.register",
            "identityHooks.pending",
            "identityHooks.attempt",
            "identityHooks.ack",
        ] {
            assert!(
                capabilities["operations"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(operation))
            );
        }
    }

    #[test]
    fn skills_operations_require_consent_and_strict_shapes() {
        let code = |operation: &str, input: Value, identity: Option<&str>| {
            let mut body = json!({"version": 1, "operation": operation, "input": input});
            if let Some(identity) = identity {
                body["identity"] = json!(identity);
            }
            match decode(&body.to_string()) {
                Ok(_) => "OK".to_owned(),
                Err(fault) => fault.code.to_owned(),
            }
        };
        let skills =
            json!([{"name": "tmt-squad", "files": [{"path": "SKILL.md", "content": "x"}]}]);
        assert_eq!(
            code(
                "skills.install",
                json!({"owner": "squad", "consent": true, "skills": skills}),
                None
            ),
            "OK"
        );
        assert_eq!(
            code(
                "skills.install",
                json!({"owner": "squad", "consent": false, "skills": skills}),
                None
            ),
            "API_CONSENT_REQUIRED"
        );
        assert_eq!(
            code(
                "skills.remove",
                json!({"owner": "squad", "consent": false}),
                None
            ),
            "API_CONSENT_REQUIRED"
        );
        assert_eq!(
            code(
                "skills.remove",
                json!({"owner": "squad", "consent": true}),
                None
            ),
            "OK"
        );
        for (input, identity) in [
            (json!({"owner": "squad", "skills": skills}), None),
            (
                json!({"owner": "squad", "consent": true, "skills": skills, "extra": 1}),
                None,
            ),
            (
                json!({"owner": "squad", "consent": true, "skills": skills}),
                Some("Ben"),
            ),
            (
                json!({"owner": "squad", "consent": true, "skills": [{"name": "x", "files": [{"path": "SKILL.md", "bytes": "x"}]}]}),
                None,
            ),
        ] {
            assert_eq!(
                code("skills.install", input.clone(), identity),
                "API_INPUT_INVALID",
                "{input}"
            );
        }
        let listed: Value = serde_json::from_slice(&capabilities()).unwrap();
        let operations = listed["operations"].as_array().unwrap();
        assert!(operations.contains(&json!("skills.install")));
        assert!(operations.contains(&json!("skills.remove")));
    }

    #[test]
    fn references_resolve_reports_states_and_not_found_without_errors() {
        let directory = crate::test_support::TestDirectory::new();
        let paths = ConfigPaths::resolve(
            &directory.path,
            &directory.path,
            Some(&directory.path),
            None,
        );
        let active = "11111111-1111-4111-8111-111111111111";
        let retired = "22222222-2222-4222-8222-222222222222";
        let missing = "33333333-3333-4333-8333-333333333333";
        let room = "44444444-4444-4444-8444-444444444444";
        Storage::open(&paths.database).unwrap().close().unwrap();
        rusqlite::Connection::open(&paths.database).unwrap().execute_batch(&format!(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES ('{active}', 'Ada', 'ada', 't', 't', 'saved'), ('{retired}', 'Old', 'old', 't', 't', 'temporary');
             UPDATE identities SET retired_at_ms = 5 WHERE id = '{retired}';
             INSERT INTO office_meeting_rooms (room_id, name, revision, retired) VALUES ('{room}', 'Review', 2, 1);"
        )).unwrap();
        let call = |input: serde_json::Value| {
            let body = json!({"version": 1, "operation": "references.resolve", "input": input})
                .to_string();
            decode(&body).map(|request| {
                serde_json::from_slice::<serde_json::Value>(&execute(&paths, request).unwrap())
                    .unwrap()
            })
        };
        assert_eq!(
            call(json!({"identityIds": [active, retired, missing], "roomIds": [room, missing]}))
                .unwrap(),
            json!({
                "identities": [
                    {"id": active, "found": true, "name": "Ada", "lifetime": "saved", "retired": false},
                    {"id": retired, "found": true, "name": "Old", "lifetime": "temporary", "retired": true},
                    {"id": missing, "found": false},
                ],
                "rooms": [
                    {"id": room, "found": true, "retired": true},
                    {"id": missing, "found": false},
                ],
            })
        );
        assert_eq!(
            call(json!({})).unwrap(),
            json!({"identities": [], "rooms": []})
        );
        let too_many: Vec<String> = (0..257).map(|_| active.to_owned()).collect();
        for input in [
            json!({"identityIds": too_many}),
            json!({"identityIds": ["not-a-uuid"]}),
            json!({"roomIds": ["44444444444444444444444444444444"]}),
            json!({"identityIds": [active], "extra": true}),
        ] {
            assert!(
                matches!(
                    call(input.clone()),
                    Err(Fault {
                        code: "API_INPUT_INVALID",
                        ..
                    })
                ),
                "{input}"
            );
        }
        let exact: Vec<String> = (0..256).map(|_| active.to_owned()).collect();
        assert!(call(json!({"identityIds": exact})).is_ok());
    }

    #[test]
    fn identities_status_reports_status_expiry_and_not_found_without_errors() {
        let directory = crate::test_support::TestDirectory::new();
        let paths = ConfigPaths::resolve(
            &directory.path,
            &directory.path,
            Some(&directory.path),
            None,
        );
        let fresh = "11111111-1111-4111-8111-111111111111";
        let expired = "22222222-2222-4222-8222-222222222222";
        let plain = "33333333-3333-4333-8333-333333333333";
        let missing = "44444444-4444-4444-8444-444444444444";
        Storage::open(&paths.database).unwrap().close().unwrap();
        let now = crate::request_runtime::wall_time_ms() as i64;
        rusqlite::Connection::open(&paths.database).unwrap().execute_batch(&format!(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES
               ('{fresh}', 'Ada', 'ada', 't', 't', 'saved'),
               ('{expired}', 'Old', 'old', 't', 't', 'saved'),
               ('{plain}', 'Cy', 'cy', 't', 't', 'saved');
             INSERT INTO identity_status (identity_id, activity, mood, updated_at_ms, expires_at_ms) VALUES
               ('{fresh}', 'Reviewing', 'calm', {now}, {}),
               ('{expired}', 'Was busy', NULL, 1000, 5000);",
            now + 3_600_000
        )).unwrap();
        let call = |input: serde_json::Value| {
            let body =
                json!({"version": 1, "operation": "identities.status", "input": input}).to_string();
            decode(&body).map(|request| {
                serde_json::from_slice::<serde_json::Value>(&execute(&paths, request).unwrap())
                    .unwrap()
            })
        };
        let result = call(json!({"identityIds": [fresh, expired, plain, missing]})).unwrap();
        let entries = result["identities"].as_array().unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0]["status"]["activity"], "Reviewing");
        assert_eq!(entries[0]["status"]["stale"], false);
        assert_eq!(entries[1]["status"]["activity"], "Was busy");
        assert_eq!(entries[1]["status"]["stale"], true);
        assert_eq!(
            entries[2],
            json!({"id": plain, "found": true, "status": null})
        );
        assert_eq!(entries[3], json!({"id": missing, "found": false}));
        let too_many: Vec<String> = (0..257).map(|_| fresh.to_owned()).collect();
        for input in [
            json!({"identityIds": too_many}),
            json!({"identityIds": ["not-a-uuid"]}),
            json!({}),
            json!({"identityIds": [fresh], "extra": true}),
        ] {
            assert!(
                matches!(
                    call(input.clone()),
                    Err(Fault {
                        code: "API_INPUT_INVALID",
                        ..
                    })
                ),
                "{input}"
            );
        }
    }

    #[test]
    fn roster_is_a_read_with_a_strict_room_and_key_prefix() {
        let roster = |input: &str| {
            decode(&format!(
                r#"{{"version":1,"operation":"rooms.roster","input":{input}}}"#
            ))
        };
        let Request::Roster { room, prefix } =
            roster(r#"{"room":"Design room","metadataPrefix":"squad.a."}"#).unwrap()
        else {
            panic!("roster");
        };
        assert_eq!(
            (room.as_str(), prefix.as_deref()),
            ("Design room", Some("squad.a."))
        );
        assert!(matches!(
            roster(r#"{"room":"00000000-0000-4000-8000-000000000001"}"#),
            Ok(Request::Roster { prefix: None, .. })
        ));
        let long = format!(r#"{{"room":"{}"}}"#, "r".repeat(257));
        for input in [
            r#"{"room":""}"#,
            long.as_str(),
            r#"{"room":"A","metadataPrefix":""}"#,
            r#"{"room":"A","metadataPrefix":"Squad."}"#,
            r#"{"room":"A","metadataPrefix":"squad a"}"#,
            r#"{"room":"A","presence":true}"#,
            r#"{"room":"A","room":"B"}"#,
            r#"{}"#,
        ] {
            assert!(
                matches!(
                    roster(input),
                    Err(Fault {
                        code: "API_INPUT_INVALID",
                        ..
                    })
                ),
                "{input}"
            );
        }
        assert!(matches!(
            decode(
                r#"{"version":1,"operation":"rooms.roster","identity":"A","input":{"room":"A"}}"#
            ),
            Err(Fault {
                code: "API_INPUT_INVALID",
                ..
            })
        ));
        let capabilities: serde_json::Value = serde_json::from_slice(&capabilities()).unwrap();
        assert!(
            capabilities["operations"]
                .as_array()
                .unwrap()
                .contains(&json!("rooms.roster"))
        );
    }

    #[test]
    fn envelope_allows_canonical_maximum_message_after_json_escaping() {
        let message = "\u{1b}".repeat(tmt_core::exact_text::MAX_EXCHANGE_TEXT_BYTES);
        let body = json!({"version":1,"operation":"dispatch.create","identity":"Sender","input":{
            "operationId":"00000000-0000-4000-8000-000000000001",
            "recipientIds":["00000000-0000-4000-8000-000000000002"],"message":message
        }})
        .to_string();
        assert!(body.len() < INPUT_LIMIT);
        let Request::Dispatch { input, .. } = decode(&body).unwrap() else {
            panic!("dispatch");
        };
        assert_eq!(input.message, message);
    }

    #[test]
    fn unattributed_operations_reject_either_or_both_originator_fields() {
        let id = "11111111-1111-4111-8111-111111111111";
        let hook = json!({"consumer": "extension", "identityId": id, "reference": "scope"});
        for (operation, input) in [
            ("capabilities", json!({})),
            ("storage.root", json!({})),
            ("changes.cursor", json!({})),
            ("requests.list", json!({"recipientId": id})),
            ("requests.show", json!({"requestId": format!("req_{id}")})),
            ("dispatch.show", json!({"operationId": id})),
            ("rooms.roster", json!({"room": id})),
            ("notes.read", json!({"identityId": id})),
            ("references.resolve", json!({"identityIds": [id]})),
            ("identities.status", json!({"identityIds": [id]})),
            ("identityHooks.register", hook.clone()),
            (
                "identityHooks.pending",
                json!({"consumer": "extension", "limit": 1}),
            ),
            ("identityHooks.attempt", hook.clone()),
            ("identityHooks.ack", hook),
            (
                "skills.install",
                json!({"owner": "extension", "consent": true,
                "skills": [{"name": "extension", "files": [{"path": "SKILL.md", "content": "skill"}]}]}),
            ),
            (
                "skills.remove",
                json!({"owner": "extension", "consent": true}),
            ),
        ] {
            let envelope = json!({"version": 1, "operation": operation, "input": input});
            assert!(decode(&envelope.to_string()).is_ok(), "{envelope}");
            for extra in [
                json!({"identity": "Ada"}),
                json!({"originator": "anonymous"}),
                json!({"identity": "Ada", "originator": "anonymous"}),
            ] {
                let mut body = envelope.clone();
                for (key, value) in extra.as_object().unwrap() {
                    body[key] = value.clone();
                }
                let fault = decode(&body.to_string()).err().expect("originator refused");
                assert_eq!(
                    serde_json::from_slice::<Value>(&fault.encode()).unwrap(),
                    json!({"error": {"code": "API_INPUT_INVALID",
                        "message": "Invalid operation, fields or bounded input."}}),
                    "{body}"
                );
            }
        }
    }

    #[test]
    fn writes_name_exactly_one_originator_and_anonymous_stores_no_identity() {
        let directory = crate::test_support::TestDirectory::new();
        let paths = ConfigPaths::resolve(
            &directory.path,
            &directory.path,
            Some(&directory.path),
            None,
        );
        Storage::open(&paths.database).unwrap().close().unwrap();
        let ada = "11111111-1111-4111-8111-111111111111";
        let room = "55555555-5555-4555-8555-555555555555";
        rusqlite::Connection::open(&paths.database)
            .unwrap()
            .execute(
                "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES (?, 'Ada', 'ada', 't', 't', 'saved')",
                [ada],
            )
            .unwrap();
        let dispatch = json!({"operationId": "66666666-6666-4666-8666-666666666666",
            "recipientIds": [ada], "message": "hello"});
        let room_write = json!({"roomId": room,
            "room": {"expectedRevision": 0, "name": "Review", "memberIds": [ada]}});
        let decode_with = |operation: &str, extra: serde_json::Value, input: &serde_json::Value| {
            let mut envelope = json!({"version": 1, "operation": operation, "input": input});
            for (key, value) in extra.as_object().unwrap() {
                envelope[key] = value.clone();
            }
            decode(&envelope.to_string())
        };
        let room_retire = json!({"roomId": room, "expectedRevision": 1});
        for (operation, input) in [
            ("dispatch.create", &dispatch),
            ("rooms.write", &room_write),
            ("rooms.retire", &room_retire),
        ] {
            assert!(decode_with(operation, json!({"originator": "anonymous"}), input).is_ok());
            assert!(decode_with(operation, json!({"identity": "Ada"}), input).is_ok());
            for extra in [
                json!({}),
                json!({"identity": "Ada", "originator": "anonymous"}),
                json!({"originator": "owner"}),
                json!({"originator": ""}),
            ] {
                assert!(
                    matches!(
                        decode_with(operation, extra.clone(), input),
                        Err(Fault {
                            code: "API_INPUT_INVALID",
                            ..
                        })
                    ),
                    "{operation} {extra}"
                );
            }
        }
        // Reads never accept an originator.
        assert!(
            decode_with(
                "capabilities",
                json!({"originator": "anonymous"}),
                &json!({})
            )
            .is_err()
        );

        let run = |operation: &str, input: &serde_json::Value| {
            let request =
                decode_with(operation, json!({"originator": "anonymous"}), input).unwrap();
            serde_json::from_slice::<serde_json::Value>(&execute(&paths, request).unwrap()).unwrap()
        };
        let written = run("rooms.write", &room_write);
        assert_eq!(written["id"], room);
        let receipt = run("dispatch.create", &dispatch);
        assert_eq!(receipt["items"][0]["recipientId"], ada);
        let originators: Vec<Option<String>> = rusqlite::Connection::open(&paths.database)
            .unwrap()
            .prepare("SELECT originator_identity_id FROM request_attempts")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(originators, vec![None]);
    }

    /// The cursor is the same for every read and advances with each write,
    /// whichever operation made it.
    #[test]
    fn changes_cursor_advances_with_writes_and_never_with_reads() {
        let directory = crate::test_support::TestDirectory::new();
        let paths = ConfigPaths::resolve(
            &directory.path,
            &directory.path,
            Some(&directory.path),
            None,
        );
        Storage::open(&paths.database).unwrap().close().unwrap();
        let ada = "11111111-1111-4111-8111-111111111111";
        let room = "55555555-5555-4555-8555-555555555555";
        rusqlite::Connection::open(&paths.database)
            .unwrap()
            .execute(
                "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES (?, 'Ada', 'ada', 't', 't', 'saved')",
                [ada],
            )
            .unwrap();
        let run = |operation: &str, extra: serde_json::Value, input: serde_json::Value| {
            let mut envelope = json!({"version": 1, "operation": operation, "input": input});
            for (key, value) in extra.as_object().unwrap() {
                envelope[key] = value.clone();
            }
            let request = decode(&envelope.to_string()).unwrap();
            serde_json::from_slice::<serde_json::Value>(&execute(&paths, request).unwrap()).unwrap()
        };
        let cursor = || {
            run("changes.cursor", json!({}), json!({}))["cursor"]
                .as_u64()
                .unwrap()
        };
        let reads = || {
            run("capabilities", json!({}), json!({}));
            run("requests.list", json!({}), json!({"recipientId": ada}));
            run("requests.list", json!({}), json!({"roomId": room}));
            run(
                "identities.status",
                json!({}),
                json!({"identityIds": [ada]}),
            );
            run(
                "references.resolve",
                json!({}),
                json!({"identityIds": [ada], "roomIds": [room]}),
            );
        };

        let start = cursor();
        reads();
        assert_eq!(cursor(), start, "reads never advance it");
        run(
            "rooms.write",
            json!({"originator": "anonymous"}),
            json!({"roomId": room, "room": {"expectedRevision": 0, "name": "Review", "memberIds": [ada]}}),
        );
        let written = cursor();
        assert!(written > start, "a room write advances it");
        reads();
        run("rooms.roster", json!({}), json!({"room": "Review"}));
        assert_eq!(cursor(), written);
        run(
            "dispatch.create",
            json!({"originator": "anonymous"}),
            json!({"operationId": "66666666-6666-4666-8666-666666666666",
                "recipientIds": [ada], "message": "hello"}),
        );
        assert!(cursor() > written, "a request advances it");
        assert!(capabilities_list().contains(&"changes.cursor".to_owned()));
        // Strict input, as every other operation.
        assert!(
            decode(
                &json!({"version": 1, "operation": "changes.cursor", "input": {"since": 1}})
                    .to_string()
            )
            .is_err()
        );
    }

    fn capabilities_list() -> Vec<String> {
        serde_json::from_slice::<serde_json::Value>(&capabilities()).unwrap()["operations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|operation| operation.as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn rooms_retire_uses_the_expected_revision_and_the_shared_originator_rule() {
        let directory = crate::test_support::TestDirectory::new();
        let paths = ConfigPaths::resolve(
            &directory.path,
            &directory.path,
            Some(&directory.path),
            None,
        );
        Storage::open(&paths.database).unwrap().close().unwrap();
        let room = "55555555-5555-4555-8555-555555555555";
        let unknown = "66666666-6666-4666-8666-666666666666";
        let call = |operation: &str, extra: serde_json::Value, input: serde_json::Value| {
            let mut envelope = json!({"version": 1, "operation": operation, "input": input});
            for (key, value) in extra.as_object().unwrap() {
                envelope[key] = value.clone();
            }
            decode(&envelope.to_string()).and_then(|request| {
                execute(&paths, request)
                    .map(|body| serde_json::from_slice::<serde_json::Value>(&body).unwrap())
            })
        };
        let anonymous = || json!({"originator": "anonymous"});
        call(
            "rooms.write",
            anonymous(),
            json!({"roomId": room, "room": {"expectedRevision": 0, "name": "Review", "memberIds": []}}),
        )
        .unwrap();
        let retire = |id: &str, revision: u64| {
            call(
                "rooms.retire",
                anonymous(),
                json!({"roomId": id, "expectedRevision": revision}),
            )
        };
        let stale = retire(room, 9).unwrap_err();
        let conflict = stale.code;
        assert_eq!(conflict, "ROOM_REVISION_CONFLICT");
        assert_eq!(retire(unknown, 1).unwrap_err().code, "ROOM_NOT_FOUND");
        let retired = retire(room, 1).unwrap();
        assert_eq!(retired["id"], room);
        assert_eq!(retired["retired"], true);
        // A retry with the pre- or post-retirement revision is idempotent; anything
        // else against a retired room is a conflict.
        assert_eq!(retire(room, 1).unwrap()["retired"], true);
        assert_eq!(retire(room, 2).unwrap()["retired"], true);
        assert_eq!(retire(room, 5).unwrap_err().code, "ROOM_REVISION_CONFLICT");
        for extra in [
            json!({}),
            json!({"identity": "Ada", "originator": "anonymous"}),
        ] {
            assert!(matches!(
                call(
                    "rooms.retire",
                    extra,
                    json!({"roomId": room, "expectedRevision": 1})
                ),
                Err(Fault {
                    code: "API_INPUT_INVALID",
                    ..
                })
            ));
        }
        for input in [
            json!({"roomId": room, "expectedRevision": 0}),
            json!({"roomId": "not-a-uuid", "expectedRevision": 1}),
            json!({"roomId": room}),
        ] {
            assert!(matches!(
                call("rooms.retire", anonymous(), input),
                Err(Fault {
                    code: "API_INPUT_INVALID",
                    ..
                })
            ));
        }
    }
}
