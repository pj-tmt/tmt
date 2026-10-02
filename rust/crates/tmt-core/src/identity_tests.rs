use crate::identity::*;

struct ForbiddenRepository;

impl IdentityReader for ForbiddenRepository {
    type Error = std::convert::Infallible;

    fn find_identity(&self, _: &str) -> Result<Option<Identity>, Self::Error> {
        panic!("invalid input must not read storage")
    }

    fn list_identities(&self) -> Result<Vec<Identity>, Self::Error> {
        panic!("invalid input must not list storage")
    }
}

impl IdentityRepository for ForbiddenRepository {
    fn with_identity_transaction<T>(
        &mut self,
        _: impl FnOnce(&mut dyn IdentityWriter<Error = Self::Error>) -> Result<T, Self::Error>,
    ) -> Result<T, Self::Error> {
        panic!("invalid input must not acquire a transaction")
    }
}

#[test]
fn invalid_names_stop_before_repository_access() {
    for name in ["", " ", "a\0b", "%14", "session:1.2"] {
        for lifetime in [Lifetime::Temporary, Lifetime::Saved] {
            assert!(matches!(
                create_or_resolve(&mut ForbiddenRepository, name, lifetime),
                Err(IdentityError::InvalidName(_))
            ));
        }
        assert!(matches!(
            find_by_name(&ForbiddenRepository, name),
            Err(IdentityError::InvalidName(_))
        ));
    }
}

struct Holder(Identity);

impl IdentityReader for Holder {
    type Error = std::convert::Infallible;

    fn find_identity(&self, canonical_name: &str) -> Result<Option<Identity>, Self::Error> {
        Ok((canonical_name == self.0.canonical_name).then(|| self.0.clone()))
    }

    fn list_identities(&self) -> Result<Vec<Identity>, Self::Error> {
        Ok(vec![self.0.clone()])
    }
}

#[test]
fn a_herdr_shaped_name_is_an_ordinary_name_while_no_driver_claims_it() {
    // Herdr's `wN:pM` targets are its driver's syntax (#1082); in a process
    // with no Herdr driver approved they are names like any other.
    let holder = Holder(Identity {
        name: "W1:p2".into(),
        canonical_name: "w1:p2".into(),
        ..identity("identity-1", Lifetime::Saved)
    });
    let found = find_by_name(&holder, "w1:p2")
        .unwrap()
        .expect("existing identity");
    assert_eq!(found.id, "identity-1");
    assert!(find_by_name(&holder, "w3:p4").unwrap().is_none());
    assert!(!addresses_pane(&holder, "w1:p2").unwrap());
    assert!(!addresses_pane(&holder, "w3:p4").unwrap());
    assert!(!addresses_pane(&holder, "worker").unwrap());
    // tmux targets never name identities and never read storage.
    assert!(addresses_pane(&ForbiddenRepository, "%3").unwrap());
}

fn identity(id: &str, lifetime: Lifetime) -> Identity {
    Identity {
        id: id.into(),
        name: "Researcher".into(),
        canonical_name: "researcher".into(),
        lifetime,
        created_at: "2026-09-13T00:00:00Z".into(),
        updated_at: "2026-09-13T00:00:00Z".into(),
    }
}

#[test]
fn notes_identity_id_requires_saved_canonical_v4_uuid() {
    let canonical = "e27f6cd2-2ce7-4c40-8ef7-f156492e983b";
    assert_eq!(
        NotesIdentityId::try_from(&identity(canonical, Lifetime::Saved))
            .unwrap()
            .as_str(),
        canonical
    );
    assert_eq!(
        NotesIdentityId::try_from(&identity(canonical, Lifetime::Temporary)),
        Err(NotesIdentityError::SavedIdentityRequired)
    );

    for invalid in [
        "../escape",
        "E27F6CD2-2CE7-4C40-8EF7-F156492E983B",
        "e27f6cd22ce74c408ef7f156492e983b",
        "00000000-0000-0000-0000-000000000000",
        "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
    ] {
        assert_eq!(
            NotesIdentityId::try_from(&identity(invalid, Lifetime::Saved)),
            Err(NotesIdentityError::InvalidIdentityId),
            "{invalid}"
        );
    }
}
