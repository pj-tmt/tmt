//! Storage-only identity policy shared by standalone creation and pane binding.

use crate::names::{NameError, ValidatedName, validate_name};
use std::{error::Error, fmt};
use uuid::{Uuid, Variant, Version};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifetime {
    Temporary,
    Saved,
}

impl Lifetime {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Temporary => "temporary",
            Self::Saved => "saved",
        }
    }
}

/// Non-retired identity. Historical timestamps and canonical keys are not
/// normalized again when read; changing a compiler must not rewrite ownership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub canonical_name: String,
    pub lifetime: Lifetime,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedIdentity {
    pub identity: Identity,
    pub created: bool,
}

/// A storage-safe identifier for identity-owned notes. Constructing this type
/// proves both saved lifetime eligibility and canonical UUID path material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotesIdentityId(String);

impl NotesIdentityId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotesIdentityError {
    SavedIdentityRequired,
    InvalidIdentityId,
}

impl fmt::Display for NotesIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SavedIdentityRequired => "notes require a saved identity",
            Self::InvalidIdentityId => "saved identity has an invalid identifier",
        })
    }
}

impl Error for NotesIdentityError {}

impl TryFrom<&Identity> for NotesIdentityId {
    type Error = NotesIdentityError;

    fn try_from(identity: &Identity) -> Result<Self, Self::Error> {
        if identity.lifetime != Lifetime::Saved {
            return Err(NotesIdentityError::SavedIdentityRequired);
        }
        let id =
            Uuid::parse_str(&identity.id).map_err(|_| NotesIdentityError::InvalidIdentityId)?;
        if id.get_variant() != Variant::RFC4122
            || id.get_version() != Some(Version::Random)
            || id.to_string() != identity.id
        {
            return Err(NotesIdentityError::InvalidIdentityId);
        }
        Ok(Self(identity.id.clone()))
    }
}

/// Read capabilities never infer presence or reconcile bindings.
pub trait IdentityReader {
    type Error;

    fn find_identity(&self, canonical_name: &str) -> Result<Option<Identity>, Self::Error>;
    fn list_identities(&self) -> Result<Vec<Identity>, Self::Error>;
}

/// Available only inside the repository's immediate transaction. Implementations
/// generate UUIDs/timestamps and preserve dependent rows; the service owns when
/// creation or promotion is appropriate.
pub trait IdentityWriter: IdentityReader {
    fn insert_identity(
        &mut self,
        name: &ValidatedName,
        lifetime: Lifetime,
        auto_named: bool,
    ) -> Result<Identity, Self::Error>;
    fn save_identity(&mut self, identity: &Identity) -> Result<Identity, Self::Error>;
}

pub trait IdentityRepository: IdentityReader {
    fn with_identity_transaction<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn IdentityWriter<Error = Self::Error>) -> Result<T, Self::Error>,
    ) -> Result<T, Self::Error>;
}

#[derive(Debug)]
pub enum IdentityError<E> {
    InvalidName(NameError),
    Repository(E),
}

impl<E: fmt::Display> fmt::Display for IdentityError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(error) => error.fmt(formatter),
            Self::Repository(error) => error.fmt(formatter),
        }
    }
}

impl<E: Error + 'static> Error for IdentityError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidName(error) => Some(error),
            Self::Repository(error) => Some(error),
        }
    }
}

/// This commit precedes binding publication. A later publication failure must
/// not undo an identity another invocation could already have observed.
pub fn create_or_resolve<R: IdentityRepository>(
    repository: &mut R,
    name: &str,
    requested_lifetime: Lifetime,
) -> Result<CreatedIdentity, IdentityError<R::Error>> {
    let name = validate_name(name).map_err(IdentityError::InvalidName)?;
    repository
        .with_identity_transaction(|records| {
            if let Some(mut identity) = records.find_identity(name.canonical_name())? {
                if identity.lifetime == Lifetime::Temporary && requested_lifetime == Lifetime::Saved
                {
                    identity = records.save_identity(&identity)?;
                }
                return Ok(CreatedIdentity {
                    identity,
                    created: false,
                });
            }
            Ok(CreatedIdentity {
                identity: records.insert_identity(&name, requested_lifetime, false)?,
                created: true,
            })
        })
        .map_err(IdentityError::Repository)
}

/// Whether explicit text addresses a pane rather than an identity. An
/// existing identity whose name now reads as a target (see
/// [`crate::names::validate_existing_name`]) keeps its name.
pub fn addresses_pane<R: IdentityReader + ?Sized>(
    repository: &R,
    text: &str,
) -> Result<bool, R::Error> {
    if !crate::names::is_pane_target(text) {
        return Ok(false);
    }
    let Ok(name) = crate::names::validate_existing_name(text) else {
        return Ok(true);
    };
    Ok(repository.find_identity(name.canonical_name())?.is_none())
}

/// Target-shaped text is still refused as a name, unless an existing identity
/// already holds it (see [`crate::names::validate_existing_name`]).
pub fn find_by_name<R: IdentityReader + ?Sized>(
    repository: &R,
    name: &str,
) -> Result<Option<Identity>, IdentityError<R::Error>> {
    let refusal = match validate_name(name) {
        Ok(name) => {
            return repository
                .find_identity(name.canonical_name())
                .map_err(IdentityError::Repository);
        }
        Err(error) => error,
    };
    let existing =
        crate::names::validate_existing_name(name).map_err(IdentityError::InvalidName)?;
    match repository
        .find_identity(existing.canonical_name())
        .map_err(IdentityError::Repository)?
    {
        Some(identity) => Ok(Some(identity)),
        None => Err(IdentityError::InvalidName(refusal)),
    }
}

/// A launch-generated name has explicit provenance, independent of lifetime.
/// Insert only: even an improbable collision must never adopt another identity.
pub fn create_auto_named<R: IdentityRepository>(
    repository: &mut R,
    name: &str,
    lifetime: Lifetime,
) -> Result<CreatedIdentity, IdentityError<R::Error>> {
    let name = validate_name(name).map_err(IdentityError::InvalidName)?;
    repository
        .with_identity_transaction(|records| {
            Ok(CreatedIdentity {
                identity: records.insert_identity(&name, lifetime, true)?,
                created: true,
            })
        })
        .map_err(IdentityError::Repository)
}

pub fn automatic_name(harness: &str) -> String {
    format!("{harness}-{}", &Uuid::new_v4().simple().to_string()[..12])
}
