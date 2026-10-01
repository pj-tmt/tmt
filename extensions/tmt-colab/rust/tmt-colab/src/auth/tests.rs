use super::*;
use crate::keyring::{Keyring, Layout, MemberKeys};
use ed25519_dalek::SigningKey;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
struct Fixture {
    root: PathBuf,
    layout: Layout,
    keyring: Keyring,
    member: MemberKeys,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "colab-enroll-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let keyring = Keyring::open(&layout).unwrap();
        let member = MemberKeys::open(&layout).unwrap();
        let store = Store::open(&layout).unwrap();
        Self {
            root,
            layout,
            keyring,
            member,
            store,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn verified_enrollment_certifies_member_device_and_hashes_session() {
    let mut f = Fixture::new();
    let root_before = fs::read(f.layout.directory.join("owner.key")).unwrap();
    let again = MemberKeys::open(&f.layout).unwrap();
    assert_eq!(again.id, f.member.id);
    assert_eq!(again.encryption_public, f.member.encryption_public);
    assert_eq!(
        again.signing.verifying_key(),
        f.member.signing.verifying_key()
    );
    assert_ne!(
        f.keyring.owner_public(),
        f.member.signing.verifying_key().to_bytes()
    );
    let mut code = SignInCode::issue(&mut f.store, &f.keyring, &f.member, 100).unwrap();
    let retained = Store::open(&f.layout)
        .unwrap()
        .membership_head(&code.space)
        .unwrap();
    assert_eq!(retained.revision, 1);
    assert_eq!(retained.hash, code.issuer_statement);
    assert_eq!(retained.owner_member.id, f.member.id);
    assert_eq!(
        retained.owner_member.signing_key,
        f.member.signing.verifying_key().to_bytes()
    );
    assert_eq!(
        retained.owner_member.encryption_key,
        f.member.encryption_public
    );
    let device = SigningKey::from_bytes(&[9; 32]);
    let public = device.verifying_key().to_bytes();
    let code_id = code.id.clone();
    let space = code.space.clone();
    let input = SignIn {
        code_id: &code_id,
        space: &space,
        device: "ee385e9b-7265-45f3-8b26-7f9b8dbdd8ee",
        signing_key: &public,
        encryption_key: &[7; 32],
        nonce: &[8; 16],
    };
    let proof = tmt_colab_model::auth::signin_proof(&code.secret, &input).unwrap();
    let possession = device
        .sign(&tmt_colab_model::auth::signin_possession_input(&input).unwrap())
        .to_bytes();
    assert!(
        code.enroll(&mut f.store, &f.member, &input, &[0; 32], &possession, 101)
            .is_err()
    );
    assert!(
        code.enroll(&mut f.store, &f.member, &input, &proof, &[0; 64], 101)
            .is_err()
    );
    let issued = code
        .enroll(&mut f.store, &f.member, &input, &proof, &possession, 101)
        .unwrap();
    let chain = tmt_colab_model::certificate::Chain::from_json(&issued.chain).unwrap();
    let certificate = chain.certificate().unwrap();
    assert_eq!(certificate.space, space);
    assert_eq!(certificate.device_id, input.device);
    assert_eq!(certificate.membership_revision, "1");
    chain
        .verify(
            &code.issuer_statement,
            &certificate,
            &f.member.signing.verifying_key().to_bytes(),
        )
        .unwrap();
    assert!(
        chain
            .verify(
                &code.issuer_statement,
                &certificate,
                &f.keyring.owner_public()
            )
            .is_err()
    );
    assert_eq!(certificate.expires_at, 101 + 86_400_000);
    let token = URL_SAFE_NO_PAD.decode(&issued.token).unwrap();
    assert_eq!(token.len(), 32);
    let hash = crypto::digest(&token);
    assert_eq!(
        f.store.session(&hash, &space, 102).unwrap(),
        Some(input.device.into())
    );
    assert!(
        code.enroll(&mut f.store, &f.member, &input, &proof, &possession, 102)
            .is_err()
    );
    assert_eq!(code.secret, [0; 16]);
    assert_eq!(
        fs::read(f.layout.directory.join("owner.key")).unwrap(),
        root_before
    );
    // A new process-local code invalidates the outstanding durable code, not live sessions.
    let fresh = SignInCode::issue(&mut f.store, &f.keyring, &f.member, 103).unwrap();
    assert_ne!(fresh.id, code_id);
    assert_eq!(
        f.store.session(&hash, &space, 104).unwrap(),
        Some(input.device.into())
    );
    assert_eq!(f.store.membership_head(&space).unwrap().revision, 1);
}

#[test]
fn owner_index_without_the_exact_signed_genesis_is_rejected() {
    let mut f = Fixture::new();
    SignInCode::issue(&mut f.store, &f.keyring, &f.member, 100).unwrap();
    let db = rusqlite::Connection::open(f.layout.directory.join("space.db")).unwrap();
    db.execute("UPDATE membership SET envelope=x'00'", [])
        .unwrap();
    assert!(SignInCode::issue(&mut f.store, &f.keyring, &f.member, 101).is_err());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
