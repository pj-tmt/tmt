//! Validated, identity-owned descriptive metadata and shared operations.

use crate::identity::Identity;
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

pub const MAX_METADATA_ENTRIES: usize = 64;
pub const MAX_METADATA_KEY_BYTES: usize = 64;
pub const MAX_METADATA_VALUE_BYTES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MetadataKey(String);

impl MetadataKey {
    pub fn parse(value: &str) -> Result<Self, MetadataValidationError> {
        let bytes = value.as_bytes();
        if bytes.is_empty()
            || bytes.len() > MAX_METADATA_KEY_BYTES
            || !bytes[0].is_ascii_lowercase()
            || !bytes[1..].iter().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'_' | b'.' | b'-')
            })
        {
            return Err(MetadataValidationError::InvalidKey);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataValue(String);

impl MetadataValue {
    pub fn parse(value: &str) -> Result<Self, MetadataValidationError> {
        if value.is_empty()
            || value.len() > MAX_METADATA_VALUE_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(MetadataValidationError::InvalidValue);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataFilter {
    Equals {
        key: MetadataKey,
        value: MetadataValue,
    },
    Has(MetadataKey),
}

impl MetadataFilter {
    pub fn equals(key: &str, value: &str) -> Result<Self, MetadataValidationError> {
        Ok(Self::Equals {
            key: MetadataKey::parse(key)?,
            value: MetadataValue::parse(value)?,
        })
    }

    pub fn has(key: &str) -> Result<Self, MetadataValidationError> {
        Ok(Self::Has(MetadataKey::parse(key)?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataValidationError {
    InvalidKey,
    InvalidValue,
    EntryLimit,
    InvalidChanges,
}

impl fmt::Display for MetadataValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidKey => {
                "Metadata keys must be 1-64 ASCII characters and match [a-z][a-z0-9_.-]*."
            }
            Self::InvalidValue => {
                "Metadata values must be 1-1024 UTF-8 bytes and contain no control characters."
            }
            Self::EntryLimit => "An identity may have at most 64 metadata entries.",
            Self::InvalidChanges => "Metadata apply requires 1-64 changes with distinct keys.",
        })
    }
}

impl Error for MetadataValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataLookup {
    IdentityNotFound,
    KeyNotFound,
    Found(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataCollection {
    IdentityNotFound,
    Found(BTreeMap<String, String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataMutation {
    IdentityNotFound,
    Set { changed: bool },
    Removed { removed: bool },
    EntryLimit,
}

/// Exact expectations never reserve string values for sentinel spellings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataExpectation {
    Value(MetadataValue),
    Absent,
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataAction {
    Set(MetadataValue),
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataChange {
    pub key: MetadataKey,
    pub expect: MetadataExpectation,
    pub then: MetadataAction,
}

/// Admitted changes contain distinct keys, so input order cannot affect the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataChanges(Vec<MetadataChange>);

pub type MetadataConflicts = BTreeMap<String, Option<String>>;

impl MetadataChanges {
    pub fn new(changes: Vec<MetadataChange>) -> Result<Self, MetadataValidationError> {
        let mut keys = BTreeSet::new();
        if changes.is_empty()
            || changes.len() > MAX_METADATA_ENTRIES
            || changes
                .iter()
                .any(|change| !keys.insert(change.key.clone()))
        {
            return Err(MetadataValidationError::InvalidChanges);
        }
        Ok(Self(changes))
    }

    pub fn changes(&self) -> &[MetadataChange] {
        &self.0
    }

    /// Evaluate every expectation against one snapshot before projecting any effect.
    pub fn evaluate(
        &self,
        current: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>, MetadataApply> {
        let conflicts: MetadataConflicts = self
            .0
            .iter()
            .filter_map(|change| {
                let value = current.get(change.key.as_str());
                let matches = match &change.expect {
                    MetadataExpectation::Value(expected) => {
                        value.is_some_and(|value| value == expected.as_str())
                    }
                    MetadataExpectation::Absent => value.is_none(),
                    MetadataExpectation::Any => true,
                };
                (!matches).then(|| (change.key.as_str().to_owned(), value.cloned()))
            })
            .collect();
        if !conflicts.is_empty() {
            return Err(MetadataApply::Conflict(conflicts));
        }
        let mut next = current.clone();
        for change in &self.0 {
            match &change.then {
                MetadataAction::Set(value) => {
                    next.insert(change.key.as_str().to_owned(), value.as_str().to_owned());
                }
                MetadataAction::Remove => {
                    next.remove(change.key.as_str());
                }
            }
        }
        if next.len() > MAX_METADATA_ENTRIES {
            return Err(MetadataApply::EntryLimit);
        }
        Ok(next)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataApply {
    IdentityNotFound,
    Conflict(MetadataConflicts),
    Applied { changed: bool },
    EntryLimit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyMetadataResult {
    pub identity_id: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetMetadataResult {
    pub identity_id: String,
    pub key: String,
    pub value: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetMetadataResult {
    pub identity_id: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListMetadataResult {
    pub identity_id: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveMetadataResult {
    pub identity_id: String,
    pub key: String,
    pub removed: bool,
}

/// Storage operations validate the exact active UUID inside their transaction.
pub trait IdentityMetadataRepository {
    type Error;

    fn get_metadata(
        &self,
        identity_id: &str,
        key: &MetadataKey,
    ) -> Result<MetadataLookup, Self::Error>;
    fn list_metadata(&self, identity_id: &str) -> Result<MetadataCollection, Self::Error>;
    fn set_metadata(
        &mut self,
        identity_id: &str,
        key: &MetadataKey,
        value: &MetadataValue,
    ) -> Result<MetadataMutation, Self::Error>;
    fn remove_metadata(
        &mut self,
        identity_id: &str,
        key: &MetadataKey,
    ) -> Result<MetadataMutation, Self::Error>;
    /// Recheck the exact active UUID, evaluate one snapshot and commit every effect
    /// under the same writer transaction. Conflicts and limits perform no writes.
    fn apply_metadata(
        &mut self,
        identity_id: &str,
        changes: &MetadataChanges,
    ) -> Result<MetadataApply, Self::Error>;

    fn list_identities_matching(
        &self,
        filters: &[MetadataFilter],
    ) -> Result<Vec<Identity>, Self::Error>;
}

#[derive(Debug)]
pub enum MetadataError<E> {
    Invalid(MetadataValidationError),
    IdentityNotFound,
    KeyNotFound,
    Conflict(MetadataConflicts),
    Repository(E),
}

impl<E: fmt::Display> fmt::Display for MetadataError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) => error.fmt(formatter),
            Self::IdentityNotFound => formatter.write_str("The selected identity was not found."),
            Self::KeyNotFound => formatter.write_str("The metadata key was not found."),
            Self::Conflict(_) => {
                formatter.write_str("Metadata expectations did not match current values.")
            }
            Self::Repository(error) => error.fmt(formatter),
        }
    }
}

impl<E: Error + 'static> Error for MetadataError<E> {}

pub fn set_identity_metadata<R: IdentityMetadataRepository>(
    repository: &mut R,
    identity_id: &str,
    key: &str,
    value: &str,
) -> Result<SetMetadataResult, MetadataError<R::Error>> {
    let key = MetadataKey::parse(key).map_err(MetadataError::Invalid)?;
    let value = MetadataValue::parse(value).map_err(MetadataError::Invalid)?;
    match repository
        .set_metadata(identity_id, &key, &value)
        .map_err(MetadataError::Repository)?
    {
        MetadataMutation::IdentityNotFound => Err(MetadataError::IdentityNotFound),
        MetadataMutation::Set { changed } => Ok(SetMetadataResult {
            identity_id: identity_id.to_owned(),
            key: key.as_str().to_owned(),
            value: value.as_str().to_owned(),
            changed,
        }),
        MetadataMutation::EntryLimit => {
            Err(MetadataError::Invalid(MetadataValidationError::EntryLimit))
        }
        MetadataMutation::Removed { .. } => unreachable!("set returned remove result"),
    }
}

pub fn get_identity_metadata<R: IdentityMetadataRepository>(
    repository: &R,
    identity_id: &str,
    key: &str,
) -> Result<GetMetadataResult, MetadataError<R::Error>> {
    let key = MetadataKey::parse(key).map_err(MetadataError::Invalid)?;
    match repository
        .get_metadata(identity_id, &key)
        .map_err(MetadataError::Repository)?
    {
        MetadataLookup::IdentityNotFound => Err(MetadataError::IdentityNotFound),
        MetadataLookup::KeyNotFound => Err(MetadataError::KeyNotFound),
        MetadataLookup::Found(value) => Ok(GetMetadataResult {
            identity_id: identity_id.to_owned(),
            key: key.as_str().to_owned(),
            value,
        }),
    }
}

pub fn list_identity_metadata<R: IdentityMetadataRepository>(
    repository: &R,
    identity_id: &str,
) -> Result<ListMetadataResult, MetadataError<R::Error>> {
    match repository
        .list_metadata(identity_id)
        .map_err(MetadataError::Repository)?
    {
        MetadataCollection::IdentityNotFound => Err(MetadataError::IdentityNotFound),
        MetadataCollection::Found(metadata) => Ok(ListMetadataResult {
            identity_id: identity_id.to_owned(),
            metadata,
        }),
    }
}

pub fn remove_identity_metadata<R: IdentityMetadataRepository>(
    repository: &mut R,
    identity_id: &str,
    key: &str,
) -> Result<RemoveMetadataResult, MetadataError<R::Error>> {
    let key = MetadataKey::parse(key).map_err(MetadataError::Invalid)?;
    match repository
        .remove_metadata(identity_id, &key)
        .map_err(MetadataError::Repository)?
    {
        MetadataMutation::IdentityNotFound => Err(MetadataError::IdentityNotFound),
        MetadataMutation::Removed { removed } => Ok(RemoveMetadataResult {
            identity_id: identity_id.to_owned(),
            key: key.as_str().to_owned(),
            removed,
        }),
        MetadataMutation::Set { .. } | MetadataMutation::EntryLimit => {
            unreachable!("remove returned set result")
        }
    }
}

pub fn apply_identity_metadata<R: IdentityMetadataRepository>(
    repository: &mut R,
    identity_id: &str,
    changes: &MetadataChanges,
) -> Result<ApplyMetadataResult, MetadataError<R::Error>> {
    match repository
        .apply_metadata(identity_id, changes)
        .map_err(MetadataError::Repository)?
    {
        MetadataApply::IdentityNotFound => Err(MetadataError::IdentityNotFound),
        MetadataApply::Conflict(current) => Err(MetadataError::Conflict(current)),
        MetadataApply::EntryLimit => {
            Err(MetadataError::Invalid(MetadataValidationError::EntryLimit))
        }
        MetadataApply::Applied { changed } => Ok(ApplyMetadataResult {
            identity_id: identity_id.to_owned(),
            changed,
        }),
    }
}

pub fn search_identities_by_metadata<R: IdentityMetadataRepository>(
    repository: &R,
    filters: &[MetadataFilter],
) -> Result<Vec<Identity>, MetadataError<R::Error>> {
    repository
        .list_identities_matching(filters)
        .map_err(MetadataError::Repository)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Lifetime;
    use std::convert::Infallible;

    struct Stub {
        lookup: MetadataLookup,
        collection: MetadataCollection,
        mutation: MetadataMutation,
        identities: Vec<Identity>,
        set_calls: usize,
        apply: MetadataApply,
    }

    impl IdentityMetadataRepository for Stub {
        type Error = Infallible;

        fn get_metadata(
            &self,
            _identity_id: &str,
            _key: &MetadataKey,
        ) -> Result<MetadataLookup, Self::Error> {
            Ok(self.lookup.clone())
        }

        fn list_metadata(&self, _identity_id: &str) -> Result<MetadataCollection, Self::Error> {
            Ok(self.collection.clone())
        }

        fn set_metadata(
            &mut self,
            _identity_id: &str,
            _key: &MetadataKey,
            _value: &MetadataValue,
        ) -> Result<MetadataMutation, Self::Error> {
            self.set_calls += 1;
            Ok(self.mutation)
        }

        fn remove_metadata(
            &mut self,
            _identity_id: &str,
            _key: &MetadataKey,
        ) -> Result<MetadataMutation, Self::Error> {
            Ok(self.mutation)
        }

        fn apply_metadata(
            &mut self,
            _identity_id: &str,
            _changes: &MetadataChanges,
        ) -> Result<MetadataApply, Self::Error> {
            Ok(self.apply.clone())
        }

        fn list_identities_matching(
            &self,
            _filters: &[MetadataFilter],
        ) -> Result<Vec<Identity>, Self::Error> {
            Ok(self.identities.clone())
        }
    }

    fn stub() -> Stub {
        Stub {
            lookup: MetadataLookup::Found("value".into()),
            collection: MetadataCollection::Found(
                [("key".into(), "value".into())].into_iter().collect(),
            ),
            mutation: MetadataMutation::Set { changed: true },
            identities: vec![Identity {
                id: "id".into(),
                name: "Alice".into(),
                canonical_name: "alice".into(),
                lifetime: Lifetime::Saved,
                created_at: "created".into(),
                updated_at: "updated".into(),
            }],
            set_calls: 0,
            apply: MetadataApply::Applied { changed: true },
        }
    }

    #[test]
    fn keys_and_values_preserve_exact_string_semantics() {
        assert_eq!(
            MetadataKey::parse("capability.review").unwrap().as_str(),
            "capability.review"
        );
        assert!(MetadataKey::parse("A").is_err());
        assert!(MetadataKey::parse("a/b").is_err());
        assert!(MetadataKey::parse(&format!("a{}", "b".repeat(64))).is_err());
        assert_eq!(MetadataValue::parse(" true ").unwrap().as_str(), " true ");
        assert!(MetadataValue::parse("").is_err());
        assert!(MetadataValue::parse("line\nfeed").is_err());
        assert!(MetadataValue::parse(&"é".repeat(513)).is_err());
    }

    #[test]
    fn key_and_utf8_value_byte_boundaries_are_exact() {
        let key_at_limit = format!("a{}", "b".repeat(63));
        assert_eq!(key_at_limit.len(), MAX_METADATA_KEY_BYTES);
        assert_eq!(
            MetadataKey::parse(&key_at_limit).unwrap().as_str(),
            key_at_limit
        );
        let key_over_limit = format!("a{}", "b".repeat(64));
        assert_eq!(key_over_limit.len(), MAX_METADATA_KEY_BYTES + 1);
        assert!(matches!(
            MetadataKey::parse(&key_over_limit),
            Err(MetadataValidationError::InvalidKey)
        ));

        let value_at_limit = "é".repeat(MAX_METADATA_VALUE_BYTES / 2);
        assert_eq!(value_at_limit.len(), MAX_METADATA_VALUE_BYTES);
        assert_eq!(
            MetadataValue::parse(&value_at_limit).unwrap().as_str(),
            value_at_limit
        );
        let value_over_limit = format!("{value_at_limit}a");
        assert_eq!(value_over_limit.len(), MAX_METADATA_VALUE_BYTES + 1);
        assert!(matches!(
            MetadataValue::parse(&value_over_limit),
            Err(MetadataValidationError::InvalidValue)
        ));
    }

    #[test]
    fn services_expose_typed_results_without_cli_or_storage_dependencies() {
        let mut repository = stub();
        assert_eq!(
            set_identity_metadata(&mut repository, "id", "key", "value").unwrap(),
            SetMetadataResult {
                identity_id: "id".into(),
                key: "key".into(),
                value: "value".into(),
                changed: true,
            }
        );
        assert_eq!(
            get_identity_metadata(&repository, "id", "key")
                .unwrap()
                .value,
            "value"
        );
        assert_eq!(
            list_identity_metadata(&repository, "id").unwrap().metadata["key"],
            "value"
        );
        repository.mutation = MetadataMutation::Removed { removed: false };
        assert!(
            !remove_identity_metadata(&mut repository, "id", "key")
                .unwrap()
                .removed
        );
        assert_eq!(
            search_identities_by_metadata(
                &repository,
                &[MetadataFilter::equals("key", "value").unwrap()]
            )
            .unwrap(),
            repository.identities
        );

        repository.lookup = MetadataLookup::KeyNotFound;
        assert!(matches!(
            get_identity_metadata(&repository, "id", "key"),
            Err(MetadataError::KeyNotFound)
        ));
        repository.mutation = MetadataMutation::EntryLimit;
        assert!(matches!(
            set_identity_metadata(&mut repository, "id", "new", "value"),
            Err(MetadataError::Invalid(MetadataValidationError::EntryLimit))
        ));

        repository.mutation = MetadataMutation::Set { changed: true };
        repository.lookup = MetadataLookup::Found("value".into());
        let calls_before_invalid_replacement = repository.set_calls;
        assert!(matches!(
            set_identity_metadata(&mut repository, "id", "key", "line\nfeed"),
            Err(MetadataError::Invalid(
                MetadataValidationError::InvalidValue
            ))
        ));
        assert_eq!(repository.set_calls, calls_before_invalid_replacement);
        assert_eq!(
            get_identity_metadata(&repository, "id", "key")
                .unwrap()
                .value,
            "value"
        );
    }
    fn change(key: &str, expect: MetadataExpectation, then: MetadataAction) -> MetadataChange {
        MetadataChange {
            key: MetadataKey::parse(key).unwrap(),
            expect,
            then,
        }
    }

    #[test]
    fn conditional_changes_evaluate_exact_absent_and_any_against_the_original_snapshot() {
        use MetadataAction::{Remove, Set};
        use MetadataExpectation::{Absent, Any, Value};
        let value = |s: &str| MetadataValue::parse(s).unwrap();
        let current = [
            ("state".into(), " old ".into()),
            ("pending".into(), "yes".into()),
        ]
        .into_iter()
        .collect();
        let matching = MetadataChanges::new(vec![
            change("state", Value(value(" old ")), Set(value("new"))),
            change("pending", Any, Remove),
            change("created", Absent, Set(value("any"))),
            change("missing", Any, Remove),
        ])
        .unwrap();
        assert_eq!(
            matching.evaluate(&current).unwrap(),
            [
                ("state".into(), "new".into()),
                ("created".into(), "any".into())
            ]
            .into_iter()
            .collect()
        );
        let conflicting = MetadataChanges::new(vec![
            change("pending", Any, Remove),
            change("state", Value(value("old")), Set(value("new"))),
            change("missing", Value(value("old")), Remove),
            change("pending2", Absent, Set(value("new"))),
        ])
        .unwrap();
        let expected = [
            ("state".into(), Some(" old ".into())),
            ("missing".into(), None),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            conflicting.evaluate(&current),
            Err(MetadataApply::Conflict(expected))
        );
        assert_eq!(current["pending"], "yes");
        assert!(MetadataChanges::new(vec![]).is_err());
        assert!(
            MetadataChanges::new(vec![
                change("key", Any, Remove),
                change("key", Absent, Remove)
            ])
            .is_err()
        );
    }

    #[test]
    fn conditional_service_preserves_typed_repository_outcomes() {
        let changes = MetadataChanges::new(vec![change(
            "key",
            MetadataExpectation::Any,
            MetadataAction::Remove,
        )])
        .unwrap();
        let mut repository = stub();
        assert_eq!(
            apply_identity_metadata(&mut repository, "id", &changes).unwrap(),
            ApplyMetadataResult {
                identity_id: "id".into(),
                changed: true
            }
        );
        repository.apply = MetadataApply::Applied { changed: false };
        assert!(
            !apply_identity_metadata(&mut repository, "id", &changes)
                .unwrap()
                .changed
        );
        repository.apply = MetadataApply::Conflict([("key".into(), None)].into_iter().collect());
        assert!(
            matches!(apply_identity_metadata(&mut repository, "id", &changes), Err(MetadataError::Conflict(current)) if current["key"].is_none())
        );
        repository.apply = MetadataApply::IdentityNotFound;
        assert!(matches!(
            apply_identity_metadata(&mut repository, "id", &changes),
            Err(MetadataError::IdentityNotFound)
        ));
        repository.apply = MetadataApply::EntryLimit;
        assert!(matches!(
            apply_identity_metadata(&mut repository, "id", &changes),
            Err(MetadataError::Invalid(MetadataValidationError::EntryLimit))
        ));
    }
}
