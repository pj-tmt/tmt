//! `user.identity`: the person's saved identity, created and marked on first use (#513).

use super::Fault;
use crate::storage::Storage;
use serde_json::{Map, json};
use tmt_core::identity::{Identity, Lifetime, create_or_resolve, find_by_name};
use tmt_core::identity_metadata::{
    MetadataAction, MetadataChange, MetadataChanges, MetadataError, MetadataExpectation,
    MetadataKey, MetadataValue, USER_KEY, USER_VALUE, apply_identity_metadata,
    get_identity_metadata,
};
use tmt_core::settings::Settings;
use tmt_core::user::{self, UserNameError};

/// Resolve the person's identity: the `user.name` setting or the OS login name, reusing the saved
/// identity with that name or creating it, then marking it as a person when no entry says otherwise.
/// A name an agent already holds is never taken over: the identity is `<name>-user` and
/// `renamedFrom` carries the name that was wanted.
pub(super) fn identity(
    storage: &mut Storage,
    settings: &Settings,
    login: Option<&str>,
) -> Result<Vec<u8>, Fault> {
    let wanted = user::wanted_name(settings.user_name.as_ref(), login).map_err(|error| match error {
        UserNameError::Unavailable => Fault::new(
            "USER_NAME_UNAVAILABLE",
            "No user name: the OS login name could not be read. Set one with tmt config set user.name <name>.",
        ),
        UserNameError::Invalid(_) => Fault::new(
            "USER_NAME_INVALID",
            "The OS login name is not a name an identity may take. Set one with tmt config set user.name <name>.",
        ),
    })?;
    let mut name = wanted.clone();
    if holds_an_agent(storage, &name)? {
        name = user::beside_agent(&wanted);
        if holds_an_agent(storage, &name)? {
            return Err(Fault::new(
                "USER_NAME_TAKEN",
                "Agents hold both the user name and its -user variant. Set another name with tmt config set user.name <name>.",
            ));
        }
    }
    let resolved =
        create_or_resolve(storage, &name, Lifetime::Saved).map_err(|_| Fault::unavailable())?;
    mark_as_person(storage, &resolved.identity.id)?;
    let mut value = Map::new();
    value.insert("identityId".into(), json!(resolved.identity.id));
    value.insert("name".into(), json!(resolved.identity.name));
    value.insert("created".into(), json!(resolved.created));
    if name != wanted {
        value.insert("renamedFrom".into(), json!(wanted));
    }
    serde_json::to_vec(&value).map_err(|_| Fault::unavailable())
}

/// An identity with a recorded binding is an agent, unless its own `user` entry says it is a person.
fn holds_an_agent(storage: &Storage, name: &str) -> Result<bool, Fault> {
    let Some(held) = find_by_name(storage, name).map_err(|_| Fault::unavailable())? else {
        return Ok(false);
    };
    if is_person(storage, &held)? {
        return Ok(false);
    }
    storage
        .identity_has_binding(&held.id)
        .map_err(|_| Fault::unavailable())
}

fn is_person(storage: &Storage, identity: &Identity) -> Result<bool, Fault> {
    match get_identity_metadata(storage, &identity.id, USER_KEY) {
        Ok(entry) => Ok(entry.value == USER_VALUE),
        Err(MetadataError::KeyNotFound | MetadataError::IdentityNotFound) => Ok(false),
        Err(_) => Err(Fault::unavailable()),
    }
}

/// Write the reserved `user` entry only when it is absent; an existing value is kept.
fn mark_as_person(storage: &mut Storage, identity_id: &str) -> Result<(), Fault> {
    let change = MetadataChange {
        key: MetadataKey::parse(USER_KEY).map_err(|_| Fault::unavailable())?,
        expect: MetadataExpectation::Absent,
        then: MetadataAction::Set(
            MetadataValue::parse(USER_VALUE).map_err(|_| Fault::unavailable())?,
        ),
    };
    let changes = MetadataChanges::new(vec![change]).map_err(|_| Fault::unavailable())?;
    match apply_identity_metadata(storage, identity_id, &changes) {
        Ok(_) | Err(MetadataError::Conflict(_)) => Ok(()),
        Err(_) => Err(Fault::unavailable()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use serde_json::Value;
    use tmt_core::{
        binding::BindingRepository,
        endpoint::{PaneObservation, ServerEvidence},
        host::HostKind,
        identity_metadata::{GetMetadataResult, MetadataError},
        settings::UserName,
    };

    fn open() -> (TestDirectory, Storage) {
        let directory = TestDirectory::new();
        let storage = Storage::open(directory.path.join("fixture.db")).unwrap();
        (directory, storage)
    }

    fn call(storage: &mut Storage, settings: &Settings, login: Option<&str>) -> Value {
        serde_json::from_slice(&identity(storage, settings, login).unwrap()).unwrap()
    }

    fn named(name: &str) -> Settings {
        Settings {
            user_name: UserName::parse(name),
            ..Settings::default()
        }
    }

    /// A saved identity with a recorded pane binding: an agent.
    fn agent(storage: &mut Storage, name: &str, pane: &str) -> Identity {
        let identity = create_or_resolve(storage, name, Lifetime::Saved)
            .unwrap()
            .identity;
        let server = ServerEvidence {
            host: HostKind::Tmux,
            server_id: "123e4567-e89b-42d3-a456-426614174000".into(),
            socket_path: "/tmp/tmux-test.sock".into(),
            server_pid: 41,
            server_start_time: "server-start".into(),
        };
        let observed = PaneObservation {
            agent_name: None,
            id: pane.into(),
            target: None,
            cwd: None,
            command: "agent".into(),
            pane_pid: 42,
            pane_incarnation: None,
            suggested_name: None,
            marker: None,
            location: None,
        };
        storage
            .with_binding_transaction(|records| {
                records.insert_binding(&identity, &server, &observed)
            })
            .unwrap();
        identity
    }

    fn marker(storage: &Storage, id: &str) -> Option<String> {
        match get_identity_metadata(storage, id, USER_KEY) {
            Ok(GetMetadataResult { value, .. }) => Some(value),
            Err(MetadataError::KeyNotFound) => None,
            Err(_) => panic!("metadata read failed"),
        }
    }

    #[test]
    fn the_login_name_is_the_default_created_once_and_marked_as_a_person() {
        let (_directory, mut storage) = open();
        let first = call(&mut storage, &Settings::default(), Some("benhsieh"));
        assert_eq!(first["name"], "benhsieh");
        assert_eq!(first["created"], true);
        assert!(first.get("renamedFrom").is_none());
        let id = first["identityId"].as_str().unwrap().to_owned();
        assert_eq!(marker(&storage, &id).as_deref(), Some(USER_VALUE));

        let again = call(&mut storage, &Settings::default(), Some("benhsieh"));
        assert_eq!(again["identityId"], first["identityId"]);
        assert_eq!(again["created"], false);
        storage.close().unwrap();
    }

    #[test]
    fn the_setting_wins_over_the_login_name_and_a_rename_resolves_a_new_identity() {
        let (_directory, mut storage) = open();
        let old = call(&mut storage, &named("Ben"), Some("root"));
        assert_eq!(old["name"], "Ben");
        let new = call(&mut storage, &named("Benjamin"), Some("root"));
        assert_eq!(new["name"], "Benjamin");
        assert_ne!(new["identityId"], old["identityId"]);
        // The old identity keeps its marker: history keeps its meaning (#513 decision a).
        let old_id = old["identityId"].as_str().unwrap();
        assert_eq!(marker(&storage, old_id).as_deref(), Some(USER_VALUE));
        storage.close().unwrap();
    }

    #[test]
    fn a_name_an_agent_holds_is_never_taken_over() {
        let (_directory, mut storage) = open();
        let held = agent(&mut storage, "ben", "%1");
        let resolved = call(&mut storage, &Settings::default(), Some("ben"));
        assert_eq!(resolved["name"], "ben-user");
        assert_eq!(resolved["renamedFrom"], "ben");
        assert_eq!(resolved["created"], true);
        assert_ne!(resolved["identityId"], json!(held.id));
        assert_eq!(marker(&storage, &held.id), None, "the agent stays unmarked");
        let again = call(&mut storage, &Settings::default(), Some("ben"));
        assert_eq!(again["identityId"], resolved["identityId"]);
        assert_eq!(again["renamedFrom"], "ben");
        storage.close().unwrap();
    }

    #[test]
    fn an_identity_that_says_it_is_a_person_is_reused_even_when_bound() {
        let (_directory, mut storage) = open();
        let held = agent(&mut storage, "ben", "%1");
        let change = MetadataChange {
            key: MetadataKey::parse(USER_KEY).unwrap(),
            expect: MetadataExpectation::Absent,
            then: MetadataAction::Set(MetadataValue::parse(USER_VALUE).unwrap()),
        };
        apply_identity_metadata(
            &mut storage,
            &held.id,
            &MetadataChanges::new(vec![change]).unwrap(),
        )
        .unwrap();
        let resolved = call(&mut storage, &Settings::default(), Some("ben"));
        assert_eq!(resolved["identityId"], json!(held.id));
        assert!(resolved.get("renamedFrom").is_none());
        storage.close().unwrap();
    }

    #[test]
    fn an_unbound_identity_is_adopted_and_an_existing_user_entry_is_never_overwritten() {
        let (_directory, mut storage) = open();
        let existing = create_or_resolve(&mut storage, "Ben", Lifetime::Saved)
            .unwrap()
            .identity;
        let adopted = call(&mut storage, &Settings::default(), Some("ben"));
        assert_eq!(adopted["identityId"], json!(existing.id));
        assert_eq!(adopted["created"], false);
        assert_eq!(marker(&storage, &existing.id).as_deref(), Some(USER_VALUE));

        let other = create_or_resolve(&mut storage, "Other", Lifetime::Saved)
            .unwrap()
            .identity;
        let change = MetadataChange {
            key: MetadataKey::parse(USER_KEY).unwrap(),
            expect: MetadataExpectation::Absent,
            then: MetadataAction::Set(MetadataValue::parse("false").unwrap()),
        };
        apply_identity_metadata(
            &mut storage,
            &other.id,
            &MetadataChanges::new(vec![change]).unwrap(),
        )
        .unwrap();
        let resolved = call(&mut storage, &named("Other"), None);
        assert_eq!(resolved["identityId"], json!(other.id));
        assert_eq!(marker(&storage, &other.id).as_deref(), Some("false"));
        storage.close().unwrap();
    }

    #[test]
    fn the_unresolvable_cases_name_the_setting() {
        let (_directory, mut storage) = open();
        for (login, code) in [
            (None, "USER_NAME_UNAVAILABLE"),
            (Some("  "), "USER_NAME_UNAVAILABLE"),
            (Some("bad\nname"), "USER_NAME_INVALID"),
        ] {
            let error = identity(&mut storage, &Settings::default(), login).unwrap_err();
            assert_eq!(error.code, code);
            assert!(error.message.contains("tmt config set user.name"), "{code}");
        }
        agent(&mut storage, "ben", "%1");
        agent(&mut storage, "ben-user", "%2");
        let error = identity(&mut storage, &Settings::default(), Some("ben")).unwrap_err();
        assert_eq!(error.code, "USER_NAME_TAKEN");
        assert!(error.message.contains("tmt config set user.name"));
        storage.close().unwrap();
    }

    #[test]
    fn the_operation_takes_no_input_and_is_advertised() {
        let ok = json!({"version": 1, "operation": "user.identity", "input": {}});
        assert!(matches!(
            crate::api::decode(&ok.to_string()),
            Ok(crate::api::Request::UserIdentity)
        ));
        for input in [json!({"name": "Ben"}), json!(null), json!([])] {
            let body = json!({"version": 1, "operation": "user.identity", "input": input});
            assert!(crate::api::decode(&body.to_string()).is_err(), "{body}");
        }
        let capabilities: Value = serde_json::from_slice(&crate::api::capabilities()).unwrap();
        assert!(
            capabilities["operations"]
                .as_array()
                .unwrap()
                .contains(&json!("user.identity"))
        );
    }
}
