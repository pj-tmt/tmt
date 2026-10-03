//! Extension-owned software keyring and private files; no core/config writes.
use crate::Result;
use ed25519_dalek::SigningKey;
use nix::fcntl::Flock;
use std::{fs::File, ops::Deref, path::Path};
use tmt_extension_state::Error as StateError;

const FILES: &[&str] = &["owner.key", "serve.lock", "keyring.lock", "space.db"];

pub struct Layout {
    shared: tmt_extension_state::Layout,
}
impl Deref for Layout {
    type Target = tmt_extension_state::Layout;

    fn deref(&self) -> &Self::Target {
        &self.shared
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum StateFault {
    RootNotAbsolute,
    UnsafeDirectory,
    UnsafeFile,
    InvalidFileName,
    InvalidOwnerKey,
    AlreadyServing,
    KeyringBusy,
}
impl StateFault {
    pub fn code(&self) -> &'static str {
        match self {
            Self::RootNotAbsolute => "COLAB_ROOT_INVALID",
            Self::UnsafeDirectory | Self::UnsafeFile => "COLAB_STATE_UNSAFE",
            Self::InvalidFileName => "COLAB_STATE_NAME_INVALID",
            Self::InvalidOwnerKey => "COLAB_KEY_INVALID",
            Self::AlreadyServing => "COLAB_ALREADY_SERVING",
            Self::KeyringBusy => "COLAB_KEYRING_BUSY",
        }
    }
}
impl std::fmt::Display for StateFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RootNotAbsolute => "Data root must be absolute.",
            Self::UnsafeDirectory => {
                "Colab directory must be an owned 0700 directory, not a symlink."
            }
            Self::UnsafeFile => "Colab files must be owned regular 0600 files.",
            Self::InvalidFileName => "Invalid private file name.",
            Self::InvalidOwnerKey => "Invalid owner key length; key was not replaced.",
            Self::AlreadyServing => "Local space is already serving.",
            Self::KeyringBusy => "Owner keyring publication is already in progress.",
        })
    }
}
impl std::error::Error for StateFault {}
fn state_error(error: StateError) -> Box<dyn std::error::Error + Send + Sync> {
    match error {
        StateError::RootNotAbsolute => StateFault::RootNotAbsolute.into(),
        StateError::UnsafeDirectory => StateFault::UnsafeDirectory.into(),
        StateError::UnsafeFile => StateFault::UnsafeFile.into(),
        StateError::InvalidFileName => StateFault::InvalidFileName.into(),
        StateError::InvalidLength => StateFault::InvalidOwnerKey.into(),
        StateError::DirectoryMissing(_) => "Colab directory disappeared.".into(),
        StateError::FileOpen(error) | StateError::ReadOpen(error) | StateError::Io(error) => {
            error.into()
        }
        StateError::Lock(error) => error.into(),
        StateError::Busy => nix::errno::Errno::EWOULDBLOCK.into(),
    }
}
fn lock_error(error: StateError, busy: StateFault) -> Box<dyn std::error::Error + Send + Sync> {
    match error {
        StateError::Busy => busy.into(),
        error => state_error(error),
    }
}
impl Layout {
    pub fn open(data_root: &Path) -> Result<Self> {
        tmt_extension_state::Layout::open(data_root, "colab", FILES)
            .map(|shared| Self { shared })
            .map_err(state_error)
    }
    pub fn existing(data_root: &Path) -> Result<Option<Self>> {
        tmt_extension_state::Layout::existing(data_root, "colab", FILES)
            .map(|layout| layout.map(|shared| Self { shared }))
            .map_err(state_error)
    }
    pub fn file(&self, name: &str) -> Result<File> {
        self.shared.file(name).map_err(state_error)
    }
    pub fn serve_lock(&self) -> Result<Flock<File>> {
        self.shared
            .lock("serve.lock")
            .map_err(|error| lock_error(error, StateFault::AlreadyServing))
    }
    pub fn running(&self) -> Result<bool> {
        self.shared.running("serve.lock").map_err(state_error)
    }
}

pub struct Keyring {
    owner: SigningKey,
    pub space_id: String,
}
impl Keyring {
    pub fn open(layout: &Layout) -> Result<Self> {
        let publication = layout
            .shared
            .publication("owner.key", "keyring.lock", ".owner-", 32)
            .map_err(|error| lock_error(error, StateFault::KeyringBusy))?;
        if !publication.exists()? {
            let mut seed = [0; 32];
            getrandom::fill(&mut seed)?;
            let mut name = [0; 16];
            getrandom::fill(&mut name)?;
            // Preserve Colab's failure ordering: staging fails immediately;
            // removal and directory sync precede any write/link error.
            let mut temporary = publication.stage(&name)?;
            let result = temporary.write_and_link(&seed);
            seed.fill(0);
            publication.discard(&name)?;
            layout.shared.sync()?;
            result?;
        }
        Self::read(layout)
    }
    pub fn read(layout: &Layout) -> Result<Self> {
        let mut seed = layout.shared.read("owner.key", 33).map_err(state_error)?;
        let mut seed_array: [u8; 32] = seed
            .as_slice()
            .try_into()
            .map_err(|_| StateFault::InvalidOwnerKey)?;
        let owner = SigningKey::from_bytes(&seed_array);
        seed_array.fill(0);
        seed.fill(0);
        let space_id = tmt_colab_model::crypto::space_id(&owner.verifying_key().to_bytes())
            .map_err(|_| StateFault::InvalidOwnerKey)?;
        Ok(Self { owner, space_id })
    }
    /// Sign exact model-admitted bytes without exporting the root key.
    /// Session, policy and mutation locking belong to the owner-transition caller.
    pub fn sign_statement(
        &self,
        previous: Option<&tmt_colab_model::statement::Head>,
        operation: &str,
        payload: &[u8],
    ) -> tmt_colab_model::Result<tmt_colab_model::statement::Envelope> {
        tmt_colab_model::statement::sign(&self.space_id, previous, operation, payload, &self.owner)
    }
    /// Owner-authenticated wrap; recipient/history admission belongs to the caller.
    pub fn seal_wrap(
        &self,
        header: &tmt_colab_model::wrap::Header,
        secret: &[u8; 32],
    ) -> tmt_colab_model::Result<tmt_colab_model::wrap::Envelope> {
        tmt_colab_model::wrap::seal(header, secret, &self.owner)
    }

    /// Purpose-separated local management keys. Private material never leaves Keyring.
    fn management_seed(&self, label: &[u8]) -> tmt_colab_model::Result<[u8; 32]> {
        let info = tmt_colab_model::framing::frame(&[label, self.space_id.as_bytes()])?;
        let mut root = self.owner.to_bytes();
        let seed = tmt_colab_model::crypto::derive_key(&root, &[], &info);
        root.fill(0);
        Ok(seed)
    }
    pub fn management_member(
        &self,
    ) -> tmt_colab_model::Result<tmt_colab_model::statement::OwnerMember> {
        let mut id = self.management_seed(b"tmt-colab-management-member-id-v1")?;
        id[6] = (id[6] & 15) | 64;
        id[8] = (id[8] & 63) | 128;
        let text = hex(&id[..16]);
        let member_id = format!(
            "{}-{}-{}-{}-{}",
            &text[..8],
            &text[8..12],
            &text[12..16],
            &text[16..20],
            &text[20..]
        );
        id.fill(0);
        let signer = self.management_signer()?;
        let mut seed = self.management_seed(b"tmt-colab-management-encryption-seed-v1")?;
        let recipient = tmt_colab_model::wrap::RecipientKey::from_seed(&seed);
        seed.fill(0);
        Ok(tmt_colab_model::statement::OwnerMember {
            id: member_id,
            signing_key: signer.verifying_key().to_bytes(),
            encryption_key: recipient?.public_key(),
        })
    }
    fn management_signer(&self) -> tmt_colab_model::Result<SigningKey> {
        let mut seed = self.management_seed(b"tmt-colab-management-signing-seed-v1")?;
        let key = SigningKey::from_bytes(&seed);
        seed.fill(0);
        Ok(key)
    }
    /// Caller admits the remote certificate and pinned owner-member binding first.
    pub(crate) fn sign_device_certificate(
        &self,
        cert: &tmt_colab_model::certificate::Certificate<'_>,
    ) -> tmt_colab_model::Result<[u8; 64]> {
        use ed25519_dalek::Signer;
        Ok(self
            .management_signer()?
            .sign(&tmt_colab_model::certificate::input(cert)?)
            .to_bytes())
    }
    /// Epoch baseline objects are authored by the local management identity.
    pub(crate) fn seal_baseline(
        &self,
        context: &tmt_colab_model::object::Context,
        secret: &[u8; 32],
        plaintext: &[u8],
    ) -> tmt_colab_model::Result<tmt_colab_model::object::Envelope> {
        tmt_colab_model::object::seal(context, secret, &self.management_signer()?, plaintext)
    }
    pub fn owner_public(&self) -> [u8; 32] {
        self.owner.verifying_key().to_bytes()
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
