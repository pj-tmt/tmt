//! Serve-owned staging slots for a native attach (#2291). A slot is a private directory the
//! serve names with a random opaque ID and nothing else; a caller never supplies a path. It holds
//! the plaintext copy while the file is being sealed, then only the sealed ciphertext and the
//! frozen upload binding, which is exactly what restart recovery needs. Until the upload has a
//! recorded transfer ID a slot is disposable; after, only an explicit resume or an aged sweep
//! (which discards the original first) removes it. A finished attach keeps its small record for a
//! while, so a reply lost after success is answered again by the same slot, never by a second
//! upload.
use crate::{Result, keyring::Layout, page::Fault};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Condvar, Mutex},
    time::Instant,
};
use tmt_colab_model::attachment::{DESCRIPTOR_BYTES, Descriptor, PLAINTEXT_BYTES};

const DIRECTORY: &str = "attach-slots";
/// The plaintext copy the CLI streams in; removed as soon as the file is sealed.
pub const SOURCE: &str = "source";
const RECORD: &str = "slot.json";
const OBJECT: &str = "object";
const VERSION: u8 = 1;
/// The slot record holds one descriptor plus a few short fields.
const RECORD_BYTES: u64 = DESCRIPTOR_BYTES as u64 + 4096;
const ID_CHARS: usize = 32;
/// Live slots a serve keeps; a caller beyond it is refused, never queued.
pub const MAX_SLOTS: usize = 16;
/// A slot that never recorded a transfer is abandoned after this long.
pub const UNSTARTED_AGE_MS: u64 = 10 * 60 * 1000;
/// A slot with a recorded transfer outlives Remote's one-day staging expiry by this much.
pub const STARTED_AGE_MS: u64 = 25 * 60 * 60 * 1000;
/// A finished slot answers a retry for this long, then goes.
pub const DONE_AGE_MS: u64 = 10 * 60 * 1000;

/// What the serve recorded when it opened the slot, and later when it sealed the file.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Record {
    version: u8,
    pub page: String,
    pub filename: String,
    pub media_type: String,
    pub created_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed: Option<Frozen>,
    /// When the attach finished; the file and ciphertext are gone, the answer remains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_ms: Option<u64>,
}
/// The frozen upload: once recorded, the same ciphertext, descriptor, base and transfer ID are
/// used by every retry, so a lost reply can never turn into a second object.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Frozen {
    pub transfer_id: String,
    pub base: String,
    pub descriptor: Descriptor,
}

pub struct StagingSlots {
    directory: PathBuf,
    /// StagingSlots an attach is running on: one at a time, in this serve.
    busy: Mutex<HashSet<String>>,
    freed: Condvar,
}
/// The right to run an attach on one slot; a later request for the same slot waits for it.
pub struct Claim<'a> {
    slots: &'a StagingSlots,
    id: String,
}
impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.slots
            .busy
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.id);
        self.slots.freed.notify_all();
    }
}
#[derive(Debug)]
pub struct StagingSlot {
    id: String,
    directory: PathBuf,
    pub record: Record,
}
/// A slot past its age, left for the caller to discard its original and dispose. It holds the
/// slot's claim, so no attach can start on it before the caller is done.
pub struct Aged<'a> {
    pub slot: StagingSlot,
    _claim: Claim<'a>,
}

fn private_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(Fault::Denied.into());
    }
    Ok(())
}
fn valid_id(id: &str) -> bool {
    id.len() == ID_CHARS
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn open_flags(options: &mut fs::OpenOptions) -> &mut fs::OpenOptions {
    options.custom_flags(nix::fcntl::OFlag::O_NOFOLLOW.bits())
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = open_flags(fs::OpenOptions::new().read(true)).open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.len() > limit
    {
        return Err(Fault::Invalid.into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Fault::Invalid.into());
    }
    Ok(bytes)
}
/// Write the whole file or nothing: a temporary sibling, then a rename, then the directory.
fn replace_file(directory: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let temporary = directory.join(format!(".{name}.tmp"));
    let mut file = open_flags(
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600),
    )
    .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, directory.join(name))?;
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

impl StagingSlots {
    /// The serve's slot directory under its private root, created on first use.
    pub fn open(layout: &Layout) -> Result<Self> {
        let directory = layout.directory.join(DIRECTORY);
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => fs::File::open(&layout.directory)?.sync_all()?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        private_directory(&directory)?;
        Ok(Self {
            directory,
            busy: Mutex::new(HashSet::new()),
            freed: Condvar::new(),
        })
    }
    fn ids(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let name = entry?.file_name();
            if let Some(name) = name.to_str().filter(|name| valid_id(name)) {
                ids.push(name.to_owned());
            }
        }
        Ok(ids)
    }
    /// Claim a slot only if no attach is on it.
    fn try_claim(&self, id: &str) -> Option<Claim<'_>> {
        let mut busy = self
            .busy
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if busy.contains(id) {
            return None;
        }
        busy.insert(id.to_owned());
        Some(Claim {
            slots: self,
            id: id.to_owned(),
        })
    }
    /// Take the one running attach of a slot. A retry that arrives while the first is still working
    /// waits for it (within `deadline`) and then finds the slot finished, instead of racing it.
    pub fn claim(&self, id: &str, deadline: Instant) -> Result<Claim<'_>> {
        if !valid_id(id) {
            return Err(Fault::Missing.into());
        }
        let mut busy = self
            .busy
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while busy.contains(id) {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return Err(Fault::Unavailable.into());
            };
            busy = self
                .freed
                .wait_timeout(busy, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        busy.insert(id.to_owned());
        Ok(Claim {
            slots: self,
            id: id.to_owned(),
        })
    }
    /// Open a slot for a page. The ID is random; the serve answers with where to stream the
    /// file, and that directory is owner-only like everything under the serve's root.
    pub fn create(
        &self,
        page: &str,
        filename: &str,
        media_type: &str,
        now_ms: u64,
    ) -> Result<StagingSlot> {
        // Finished slots hold only a small record, so only unfinished ones count against the cap.
        let active = self
            .ids()?
            .iter()
            .filter(|id| {
                self.load(id)
                    .is_ok_and(|slot| slot.record.done_ms.is_none())
            })
            .count();
        if active >= MAX_SLOTS {
            return Err(Fault::Capacity.into());
        }
        let mut raw = [0u8; ID_CHARS / 2];
        getrandom::fill(&mut raw)?;
        let id: String = raw.iter().map(|byte| format!("{byte:02x}")).collect();
        let directory = self.directory.join(&id);
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let record = Record {
            version: VERSION,
            page: page.to_owned(),
            filename: filename.to_owned(),
            media_type: media_type.to_owned(),
            created_ms: now_ms,
            sealed: None,
            done_ms: None,
        };
        let slot = StagingSlot {
            id,
            directory,
            record,
        };
        if let Err(error) = slot.write_record() {
            let _ = fs::remove_dir_all(&slot.directory);
            return Err(error);
        }
        fs::File::open(&self.directory)?.sync_all()?;
        Ok(slot)
    }
    /// The slot named by a caller, or `Missing`: an ID that is not canonical or not a slot never
    /// touches the filesystem beyond one lookup under the slot directory.
    pub fn load(&self, id: &str) -> Result<StagingSlot> {
        if !valid_id(id) {
            return Err(Fault::Missing.into());
        }
        let directory = self.directory.join(id);
        match private_directory(&directory) {
            Ok(()) => {}
            Err(_) if !directory.exists() => return Err(Fault::Missing.into()),
            Err(error) => return Err(error),
        }
        let record: Record =
            serde_json::from_slice(&read_bounded(&directory.join(RECORD), RECORD_BYTES)?)
                .map_err(|_| Fault::Invalid)?;
        if record.version != VERSION {
            return Err(Fault::ServerMismatch.into());
        }
        Ok(StagingSlot {
            id: id.to_owned(),
            directory,
            record,
        })
    }
    /// Remove every slot that never recorded a transfer and is past [`UNSTARTED_AGE_MS`] (or has
    /// no readable record at all: a crashed create), and return the started slots past
    /// [`STARTED_AGE_MS`]. Their originals must be discarded by the caller before disposal; a
    /// recent started slot is never touched, because it is what restart recovery resumes.
    pub fn sweep(&self, now_ms: u64) -> Result<Vec<Aged<'_>>> {
        let mut aged = Vec::new();
        for id in self.ids()? {
            // A slot an attach is working on is left alone, and the claim taken here keeps it so
            // until the decision, and any disposal, is done.
            let Some(claim) = self.try_claim(&id) else {
                continue;
            };
            match self.load(&id) {
                Ok(slot) => match &slot.record.sealed {
                    _ if slot
                        .record
                        .done_ms
                        .is_some_and(|done| now_ms.saturating_sub(done) >= DONE_AGE_MS) =>
                    {
                        slot.dispose()?
                    }
                    _ if slot.record.done_ms.is_some() => {}
                    None if now_ms.saturating_sub(slot.record.created_ms) >= UNSTARTED_AGE_MS => {
                        slot.dispose()?
                    }
                    Some(_) if now_ms.saturating_sub(slot.record.created_ms) >= STARTED_AGE_MS => {
                        aged.push(Aged {
                            slot,
                            _claim: claim,
                        })
                    }
                    _ => {}
                },
                Err(_) => {
                    // A directory with no valid record has no transfer to protect, but only a
                    // plain private directory of ours is removed, and only once it is old.
                    let directory = self.directory.join(&id);
                    if private_directory(&directory).is_ok()
                        && fs::symlink_metadata(&directory)
                            .and_then(|m| m.modified())
                            .ok()
                            .and_then(|time| time.elapsed().ok())
                            .is_some_and(|age| age.as_millis() as u64 >= UNSTARTED_AGE_MS)
                    {
                        fs::remove_dir_all(&directory)?;
                    }
                }
            }
        }
        Ok(aged)
    }
}

impl StagingSlot {
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Where the caller streams the plaintext copy: a file the serve will open itself.
    pub fn source_path(&self) -> PathBuf {
        self.directory.join(SOURCE)
    }
    fn write_record(&self) -> Result<()> {
        let bytes = serde_json::to_vec(&self.record)?;
        if bytes.len() as u64 > RECORD_BYTES {
            return Err(Fault::Capacity.into());
        }
        replace_file(&self.directory, RECORD, &bytes)
    }
    /// The staged plaintext, bounded while it is read, never followed through a link, and only a
    /// regular file of this user: the serve trusts the directory it made, not the file's size.
    pub fn read_source(&self) -> Result<Vec<u8>> {
        read_bounded(&self.source_path(), PLAINTEXT_BYTES as u64)
    }
    /// Freeze the upload: the ciphertext and the binding reach disk, in that order, before the
    /// first request leaves, and the plaintext copy is removed once they are durable.
    pub fn seal(&mut self, ciphertext: &[u8], sealed: Frozen) -> Result<()> {
        replace_file(&self.directory, OBJECT, ciphertext)?;
        self.record.sealed = Some(sealed);
        self.write_record()?;
        match fs::remove_file(self.source_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
    /// The frozen ciphertext of a sealed slot, exactly as it was sealed.
    pub fn read_object(&self, limit: u64) -> Result<Vec<u8>> {
        if self.record.sealed.is_none() {
            return Err(Fault::Invalid.into());
        }
        read_bounded(&self.directory.join(OBJECT), limit)
    }
    /// Record the finished attach and drop the plaintext and the ciphertext: what stays is the
    /// descriptor, enough to answer a retry whose reply was lost.
    pub fn finish(&mut self, now_ms: u64) -> Result<()> {
        self.record.done_ms = Some(now_ms);
        self.write_record()?;
        for name in [SOURCE, OBJECT] {
            match fs::remove_file(self.directory.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
    /// Remove the slot with everything in it. A slot holds only files this serve wrote, so the
    /// removal never follows a link out of it.
    pub fn dispose(self) -> Result<()> {
        private_directory(&self.directory)?;
        for entry in fs::read_dir(&self.directory)? {
            fs::remove_file(entry?.path())?;
        }
        fs::remove_dir(&self.directory)?;
        if let Some(parent) = self.directory.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Root(PathBuf);
    impl Root {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "tmt-colab-slots-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
        fn slots(&self) -> StagingSlots {
            let directory = self.0.join(DIRECTORY);
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .unwrap();
            StagingSlots {
                directory,
                busy: Mutex::new(HashSet::new()),
                freed: Condvar::new(),
            }
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    const PAGE: &str = "20000000-0000-4000-8000-000000000091";

    fn sealed(slot: &mut StagingSlot) {
        let descriptor: Descriptor = serde_json::from_value(serde_json::json!({
            "version": 1,
            "attachmentId": "20000000-0000-4000-8000-000000000092",
            "space": "a".repeat(32),
            "page": PAGE,
            "epoch": "1",
            "namespace": "content",
            "objectId": "a".repeat(64),
            "authorDevice": "20000000-0000-4000-8000-000000000093",
            "membershipRevision": "1",
            "source": {"kind": "document", "sourceDigest": "b".repeat(64)},
            "envelopeHash": "c".repeat(64),
            "signature": tmt_colab_model::values::encode_binary(&[1u8; 64]),
            "payloadSha256": "d".repeat(64),
            "payloadBytes": "40",
            "plaintextBytes": "5",
            "filename": "a.txt",
            "mediaType": "text/plain",
        }))
        .unwrap();
        slot.seal(
            b"ciphertext",
            Frozen {
                transfer_id: "20000000-0000-4000-8000-000000000094".into(),
                base: format!("v1:{}", "e".repeat(64)),
                descriptor,
            },
        )
        .unwrap();
    }

    #[test]
    fn a_slot_is_private_named_only_by_its_random_id_and_survives_a_reopen() {
        let root = Root::new("create");
        let slots = root.slots();
        let slot = slots.create(PAGE, "a.txt", "text/plain", 10).unwrap();
        assert_eq!(slot.id().len(), ID_CHARS);
        assert!(valid_id(slot.id()));
        let directory = root.0.join(DIRECTORY).join(slot.id());
        assert_eq!(fs::metadata(&directory).unwrap().mode() & 0o777, 0o700);
        let again = slots.load(slot.id()).unwrap();
        assert_eq!(again.record.page, PAGE);
        assert!(again.record.sealed.is_none());
        let other = slots.create(PAGE, "a.txt", "text/plain", 10).unwrap();
        assert_ne!(other.id(), slot.id());
    }
    #[test]
    fn only_a_canonical_id_names_a_slot_and_never_a_path() {
        let root = Root::new("ids");
        let slots = root.slots();
        for id in [
            "",
            "..",
            "../slot",
            "/etc/passwd",
            &"A".repeat(ID_CHARS),
            &"a".repeat(ID_CHARS + 1),
            &"g".repeat(ID_CHARS),
        ] {
            assert_eq!(
                slots.load(id).unwrap_err().downcast_ref::<Fault>(),
                Some(&Fault::Missing),
                "{id}"
            );
        }
        let absent = "f".repeat(ID_CHARS);
        assert_eq!(
            slots.load(&absent).unwrap_err().downcast_ref::<Fault>(),
            Some(&Fault::Missing)
        );
    }
    #[test]
    fn the_staged_copy_is_bounded_and_never_followed_through_a_link() {
        let root = Root::new("source");
        let slots = root.slots();
        let slot = slots.create(PAGE, "a.txt", "text/plain", 10).unwrap();
        fs::write(slot.source_path(), b"hello").unwrap();
        assert_eq!(slot.read_source().unwrap(), b"hello");
        fs::remove_file(slot.source_path()).unwrap();
        let target = root.0.join("outside");
        fs::write(&target, b"secret").unwrap();
        symlink(&target, slot.source_path()).unwrap();
        assert!(slot.read_source().is_err());
        fs::remove_file(slot.source_path()).unwrap();
        fs::write(slot.source_path(), vec![0u8; PLAINTEXT_BYTES + 1]).unwrap();
        assert!(slot.read_source().is_err());
    }
    #[test]
    fn sealing_freezes_the_ciphertext_and_drops_the_plaintext() {
        let root = Root::new("seal");
        let slots = root.slots();
        let mut slot = slots.create(PAGE, "a.txt", "text/plain", 10).unwrap();
        fs::write(slot.source_path(), b"hello").unwrap();
        sealed(&mut slot);
        assert!(!slot.source_path().exists());
        let reopened = slots.load(slot.id()).unwrap();
        assert_eq!(
            reopened.record.sealed.as_ref().unwrap().transfer_id,
            "20000000-0000-4000-8000-000000000094"
        );
        assert_eq!(reopened.read_object(1024).unwrap(), b"ciphertext");
        assert!(reopened.read_object(3).is_err());
    }
    #[test]
    fn the_sweep_removes_only_unstarted_or_aged_slots_and_returns_aged_started_ones() {
        let root = Root::new("sweep");
        let slots = root.slots();
        let fresh = slots.create(PAGE, "a", "text/plain", 1_000).unwrap();
        let abandoned = slots.create(PAGE, "a", "text/plain", 1_000).unwrap();
        let mut started = slots.create(PAGE, "a", "text/plain", 1_000).unwrap();
        sealed(&mut started);
        let mut old = slots.create(PAGE, "a", "text/plain", 1_000).unwrap();
        sealed(&mut old);
        let (fresh_id, abandoned_id) = (fresh.id().to_owned(), abandoned.id().to_owned());
        let (started_id, old_id) = (started.id().to_owned(), old.id().to_owned());
        // Just before the unstarted age: nothing goes.
        assert!(
            slots
                .sweep(1_000 + UNSTARTED_AGE_MS - 1)
                .unwrap()
                .is_empty()
        );
        assert!(slots.load(&abandoned_id).is_ok());
        // An unstarted slot goes at its age; a started slot, which restart recovery needs, stays
        // and is not offered for discard until it has outlived the backend's staging expiry.
        assert!(slots.sweep(1_000 + UNSTARTED_AGE_MS).unwrap().is_empty());
        for id in [&fresh_id, &abandoned_id] {
            assert!(slots.load(id).is_err(), "unstarted slot {id} was swept");
        }
        assert!(slots.load(&started_id).is_ok() && slots.load(&old_id).is_ok());
        let aged = slots.sweep(1_000 + STARTED_AGE_MS).unwrap();
        let mut names: Vec<_> = aged.iter().map(|a| a.slot.id().to_owned()).collect();
        names.sort();
        let mut expected = vec![started_id.clone(), old_id.clone()];
        expected.sort();
        assert_eq!(names, expected);
        // Offering a slot for discard does not dispose it: the caller discards first.
        assert!(slots.load(&started_id).is_ok());
        for Aged { slot, .. } in aged {
            slot.dispose().unwrap();
        }
        assert!(slots.load(&started_id).is_err() && slots.load(&old_id).is_err());
    }
    #[test]
    fn slots_are_capped_and_disposal_frees_the_directory() {
        let root = Root::new("cap");
        let slots = root.slots();
        let mut made = Vec::new();
        for _ in 0..MAX_SLOTS {
            made.push(slots.create(PAGE, "a", "text/plain", 10).unwrap());
        }
        assert_eq!(
            slots
                .create(PAGE, "a", "text/plain", 10)
                .err()
                .unwrap()
                .downcast_ref::<Fault>(),
            Some(&Fault::Capacity)
        );
        let id = made[0].id().to_owned();
        fs::write(made[0].source_path(), b"x").unwrap();
        made.remove(0).dispose().unwrap();
        assert!(!root.0.join(DIRECTORY).join(id).exists());
        slots.create(PAGE, "a", "text/plain", 10).unwrap();
    }
    #[test]
    fn a_finished_slot_keeps_only_its_answer_for_a_while_and_never_blocks_a_new_one() {
        let root = Root::new("done");
        let slots = root.slots();
        let mut finished = Vec::new();
        for _ in 0..MAX_SLOTS {
            let mut slot = slots.create(PAGE, "a", "text/plain", 1_000).unwrap();
            sealed(&mut slot);
            slot.finish(2_000).unwrap();
            finished.push(slot.id().to_owned());
        }
        // Finished slots do not count against the cap.
        let open = slots.create(PAGE, "a", "text/plain", 3_000).unwrap();
        let again = slots.load(&finished[0]).unwrap();
        assert_eq!(again.record.done_ms, Some(2_000));
        assert!(again.record.sealed.is_some());
        assert!(again.read_object(1024).is_err(), "the ciphertext is gone");
        assert!(!again.source_path().exists());
        // Their answer outlives their file for the retry window, then they go; an open one stays.
        assert!(slots.sweep(2_000 + DONE_AGE_MS - 1).unwrap().is_empty());
        assert!(finished.iter().all(|id| slots.load(id).is_ok()));
        assert!(slots.sweep(2_000 + DONE_AGE_MS).unwrap().is_empty());
        assert!(finished.iter().all(|id| slots.load(id).is_err()));
        assert!(slots.load(open.id()).is_ok());
    }
    #[test]
    fn a_second_attach_on_a_slot_waits_for_the_first_and_gives_up_at_its_deadline() {
        let root = Root::new("claim");
        let slots = std::sync::Arc::new(root.slots());
        let id = "a".repeat(ID_CHARS);
        let first = slots
            .claim(&id, Instant::now() + std::time::Duration::from_secs(1))
            .unwrap();
        // Another slot is not held up.
        slots
            .claim(
                &"b".repeat(ID_CHARS),
                Instant::now() + std::time::Duration::from_secs(1),
            )
            .unwrap();
        // The same slot waits, and gives up at its own deadline.
        let early = slots.claim(&id, Instant::now() + std::time::Duration::from_millis(50));
        assert_eq!(
            early.err().unwrap().downcast_ref::<Fault>(),
            Some(&Fault::Unavailable)
        );
        // Released by the first, a waiting second proceeds.
        let waiter = {
            let slots = slots.clone();
            let id = id.clone();
            std::thread::spawn(move || {
                slots
                    .claim(&id, Instant::now() + std::time::Duration::from_secs(5))
                    .is_ok()
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(first);
        assert!(waiter.join().unwrap());
        assert!(slots.claim("not-a-slot", Instant::now()).is_err());
    }
}
