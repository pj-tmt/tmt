use super::*;
use crate::{
    registration::{OwnerAdmission, Registration},
    store::owner::{Mutation, Recipient},
    sync::{Access, Admission, Server},
    transitions::{
        Engine, LinkAction, LinkSpec, OwnerAction, OwnerRequest, Publication, ShareMode,
    },
};
use ed25519_dalek::{Signer, SigningKey};
use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
pub(crate) const PAGE: &str = "10000000-0000-4000-8000-000000000001";
pub(crate) const OTHER: &str = "10000000-0000-4000-8000-000000000002";
pub(crate) const LINK: &str = "20000000-0000-4000-8000-000000000001";
pub(crate) const DEVICE: &str = "30000000-0000-4000-8000-000000000001";
pub(crate) const SEED: [u8; 32] = [17; 32];
pub(crate) struct Fixture {
    pub(crate) root: std::path::PathBuf,
    pub(crate) key: Keyring,
    pub(crate) store: Store,
    engine: Engine,
    pub(crate) service: Arc<Mutex<Registration>>,
    pub(crate) server: Server<OwnerAdmission>,
    pub(crate) clock: Arc<AtomicU64>,
    pub(crate) chain: Vec<u8>,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        let root = std::path::PathBuf::from("/tmp").join(format!("tmt-reader-{}", uuid().unwrap()));
        let layout = crate::keyring::Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let mut store = Store::open(&layout).unwrap();
        store.create_page(PAGE).unwrap();
        store.create_page(OTHER).unwrap();
        store.owner_transaction(&key.space_id,&key.owner_public(), Mutation {operation_id:&uuid().unwrap(),expected_revision:0,digest:[1;32]}, |tx| {
            let owner = key.management_member()?;
            tx.append_statement(&key.sign_statement(tx.head(),"member.add",&serde_json::to_vec(&json!({"memberId":owner.id,"role":"editor","signKey":values::encode_binary(&owner.signing_key),"encKey":values::encode_binary(&owner.encryption_key),"pages":[]}))?)?)?;
            tx.put_recipient(&Recipient {kind:"member".into(),id:owner.id,role:Some("editor".into()),signing_key:owner.signing_key,encryption_key:owner.encryption_key,pages:vec![],revoked:false})?;
            for page in [PAGE,OTHER] {tx.put_epoch_secret(page,1,&[11;32])?;}
            Ok(Vec::new())
        }).unwrap();
        let clock = Arc::new(AtomicU64::new(1000));
        let c = clock.clone();
        let service = Arc::new(Mutex::new(
            Registration::new(
                Store::open(&layout).unwrap(),
                Keyring::read(&layout).unwrap(),
                std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("tmt-colab"),
            )
            .unwrap()
            .with_reader_clock(move || Ok(c.load(Ordering::SeqCst))),
        ));
        let server = Server::new(
            Store::open(&layout).unwrap(),
            OwnerAdmission(service.clone()),
        );
        Self {
            root,
            key,
            store,
            engine: Engine::new(
                std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("tmt-colab"),
            )
            .unwrap(),
            service,
            server,
            clock,
            chain: Vec::new(),
        }
    }
    pub(crate) fn apply(&mut self, action: OwnerAction<'_>) {
        self.try_apply(action).unwrap();
    }
    pub(crate) fn try_apply(
        &mut self,
        action: OwnerAction<'_>,
    ) -> std::result::Result<crate::transitions::Applied, crate::transitions::TransitionError> {
        let revision = self
            .store
            .owner_head(&self.key.space_id, &self.key.owner_public())
            .unwrap()
            .unwrap()
            .revision;
        let mut result = None;
        self.server
            .update_admission(|_| {
                result = Some(self.engine.apply(
                    &mut self.store,
                    &self.key,
                    OwnerRequest {
                        operation_id: &uuid().unwrap(),
                        expected_revision: revision,
                        action,
                        transport_digest: None,
                        scope: None,
                    },
                    1000,
                ));
            })
            .unwrap();
        result.unwrap()
    }
    pub(crate) fn share(&mut self, mode: ShareMode) {
        self.apply(OwnerAction::Share {
            page: PAGE,
            mode,
            publication: Publication::Loopback,
        });
    }
    pub(crate) fn add_link(&mut self) {
        self.apply(OwnerAction::Link(LinkAction::Add(LinkSpec {
            id: LINK,
            role: "editor",
            pages: vec![PAGE.into()],
            seed: &SEED,
        })));
        self.chain = self.certify_link(LINK, &SEED, DEVICE);
    }
    /// Used immediately after Add/Reset, whose last statement is link.add.
    pub(crate) fn certify_link(&self, id: &str, seed: &[u8; 32], device: &str) -> Vec<u8> {
        let keys = tmt_colab_model::link::Keys::derive(seed, &self.key.space_id, id).unwrap();
        let head = self
            .store
            .owner_head(&self.key.space_id, &self.key.owner_public())
            .unwrap()
            .unwrap();
        let issuer = self
            .store
            .owner_read(&self.key.space_id, &self.key.owner_public(), |tx| {
                Ok(tx.statement(head.revision)?.unwrap())
            })
            .unwrap();
        let sign = SigningKey::from_bytes(&[19; 32]);
        let enc = tmt_colab_model::wrap::RecipientKey::from_seed(&[20; 32])
            .unwrap()
            .public_key();
        let cert = certificate::input(&certificate::Certificate {
            space: &self.key.space_id,
            issuer_kind: "link",
            issuer_id: id,
            device_id: device,
            signing_key: &sign.verifying_key().to_bytes(),
            encryption_key: &enc,
            membership_revision: &head.revision.to_string(),
            issued_at: 0,
            expires_at: 2_000_000,
        })
        .unwrap();
        serde_json::to_vec(&json!({"version":1,"issuerStatement":values::encode_binary(&issuer.hash().unwrap()),"deviceCertificate":values::encode_binary(&cert),"issuerSignature":values::encode_binary(&keys.certify(&certificate::decode(&cert).unwrap()).unwrap())})).unwrap()
    }
    pub(crate) fn request(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, Code> {
        self.service
            .lock()
            .unwrap()
            .reader_request(path, &serde_json::to_vec(&body).unwrap())
            .map(|b| serde_json::from_slice(&b).unwrap())
    }
    pub(crate) fn challenge(&self, link: bool) -> serde_json::Value {
        let mut body =
            json!({"kind":if link {"link"} else {"public"},"space":self.key.space_id,"page":PAGE});
        if link {
            body["chain"] = json!(values::encode_binary(&self.chain));
        }
        self.request(CHALLENGE_PATH, body).unwrap()
    }
    pub(crate) fn exchange_body(&self, c: &serde_json::Value, link: bool) -> serde_json::Value {
        if !link {
            return json!({"kind":"public","challengeId":c["challengeId"]});
        }
        let nonce = values::binary(c["nonce"].as_str().unwrap(), 32).unwrap();
        let digest = values::binary(c["chainDigest"].as_str().unwrap(), 32).unwrap();
        let input = framing::frame(&[
            b"tmt-colab-reader-session-v1",
            b"1",
            c["challengeId"].as_str().unwrap().as_bytes(),
            &nonce,
            c["space"].as_str().unwrap().as_bytes(),
            c["page"].as_str().unwrap().as_bytes(),
            c["epoch"].as_str().unwrap().as_bytes(),
            &digest,
            c["expiresAt"].as_u64().unwrap().to_string().as_bytes(),
        ])
        .unwrap();
        json!({"kind":"link","challengeId":c["challengeId"],"signature":values::encode_binary(&SigningKey::from_bytes(&[19;32]).sign(&input).to_bytes())})
    }
    pub(crate) fn session(&self, link: bool) -> serde_json::Value {
        let c = self.challenge(link);
        self.request(SESSION_PATH, self.exchange_body(&c, link))
            .unwrap()
    }
    pub(crate) fn upgrade(&self, s: &serde_json::Value) -> Result<String, Code> {
        let token = values::binary(s["token"].as_str().unwrap(), 32)
            .unwrap()
            .try_into()
            .unwrap();
        self.service
            .lock()
            .unwrap()
            .reader_upgrade(&token)
            .map(|r| r.0)
    }
    pub(crate) fn scope(&self) -> SyncScope {
        SyncScope {
            space: self.key.space_id.clone(),
            page: PAGE.into(),
            epoch: self
                .store
                .owner_read(&self.key.space_id, &self.key.owner_public(), |tx| {
                    Ok(tx.page_epoch(PAGE)?.unwrap())
                })
                .unwrap(),
        }
    }
    pub(crate) fn read(&self, id: &str, scope: &SyncScope) -> Result<[u8; 32], crate::sync::Code> {
        OwnerAdmission(self.service.clone()).authorize(id, scope, Access::Read)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
