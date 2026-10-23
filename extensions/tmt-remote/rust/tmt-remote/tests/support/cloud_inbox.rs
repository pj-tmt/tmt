//! Shared real Store/journal and message-layer composition; enrollment is fixture-only.
use std::sync::{Arc, Mutex, atomic::AtomicBool};
use tmt_remote::{
    cloud_hpke::{Descriptor, VerifiedDescriptor},
    cloud_inbox::{InboxDriver, Provider, Receiver},
    cloud_keys::CloudKeys,
    core::CloudIdentityQuery,
    message::MessageService,
    session::DoorSessions,
    state::{Layout, MachineKey, Serving},
    store::{Grant, Store, uuid_v4},
};
#[allow(dead_code)]
#[path = "core.rs"]
pub mod core_fixture;
pub const MACHINE: &str = "11111111-1111-4111-8111-111111111111";
pub const PEER: &str = "33333333-3333-4333-8333-333333333333";
pub const TARGET: &str = "66666666-6666-4666-8666-666666666666";
pub struct World {
    pub core: core_fixture::Core,
    pub owner_layout: Layout,
    pub sender_layout: Layout,
    pub owner_store: Arc<Mutex<Store>>,
    pub sender_store: Option<Store>,
    pub sessions: Arc<DoorSessions>,
    pub keys: Arc<CloudKeys>,
    pub service: Arc<MessageService>,
    pub key: MachineKey,
    pub descriptor: VerifiedDescriptor,
    pub owner_lease: Serving,
    pub sender_lease: Serving,
}
impl World {
    pub fn new(now: u64, identity: Arc<dyn CloudIdentityQuery>) -> Self {
        let core = core_fixture::Core::new();
        let owner_root = core.root.join("owner");
        let sender_root = core.root.join("sender");
        let owner_layout = Layout::open(&owner_root).unwrap();
        let sender_layout = Layout::open(&sender_root).unwrap();
        drop(sender_layout.file("machine.key").unwrap());
        std::fs::write(sender_layout.directory.join("machine.key"), [8; 32]).unwrap();
        let key = MachineKey::open(&sender_layout).unwrap();
        let grant = Grant {
            client_id: PEER.into(),
            public_key: key.public(),
            kind: "cli".into(),
            origin: "cli".into(),
            name: "Peer".into(),
            agents: "all".into(),
            scopes: vec!["talk".into()],
            mode: "direct".into(),
            issued_at_ms: 1,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        };
        let owner_lease = owner_layout.serve_lock().unwrap();
        let sender_lease = sender_layout.serve_lock().unwrap();
        let mut owner_store = Store::open(&owner_lease).unwrap();
        owner_store.insert_grant(&grant).unwrap();
        let mut sender_store = Store::open(&sender_lease).unwrap();
        sender_store.insert_grant(&grant).unwrap();
        let owner_store = Arc::new(Mutex::new(owner_store));
        let (sessions, keys, service, descriptor) =
            owner(&core, &owner_layout, owner_store.clone(), now, identity);

        Self {
            core,
            owner_layout,
            sender_layout,
            owner_store,
            sender_store: Some(sender_store),
            sessions,
            keys,
            service,
            key,
            descriptor,
            owner_lease,
            sender_lease,
        }
    }
    /// Caller first cancels/joins and drops its driver. No registration or credential is reset.
    pub fn reopen(self, now: u64, identity: Arc<dyn CloudIdentityQuery>) -> Self {
        let Self {
            core,
            owner_layout,
            sender_layout,
            owner_store,
            sender_store,
            sessions,
            keys,
            service,
            key,
            descriptor: _,
            owner_lease,
            sender_lease,
        } = self;
        drop(service);
        drop(sessions);
        drop(keys);
        drop(owner_store);
        let owner_store = Arc::new(Mutex::new(Store::open(&owner_lease).unwrap()));
        let (sessions, keys, service, descriptor) =
            owner(&core, &owner_layout, owner_store.clone(), now, identity);
        Self {
            core,
            owner_layout,
            sender_layout,
            owner_store,
            sender_store,
            sessions,
            keys,
            service,
            key,
            descriptor,
            owner_lease,
            sender_lease,
        }
    }
    pub fn driver(&self, provider: Provider) -> Arc<InboxDriver> {
        Arc::new(InboxDriver::new(
            Receiver::new(
                self.sessions.clone(),
                self.keys.clone(),
                self.service.clone(),
            ),
            self.sessions.clone(),
            provider,
        ))
    }
}

fn owner(
    core: &core_fixture::Core,
    layout: &Layout,
    store: Arc<Mutex<Store>>,
    now: u64,
    identity: Arc<dyn CloudIdentityQuery>,
) -> (
    Arc<DoorSessions>,
    Arc<CloudKeys>,
    Arc<MessageService>,
    VerifiedDescriptor,
) {
    let sessions = Arc::new(DoorSessions::new(
        MACHINE.into(),
        uuid_v4().unwrap(),
        "http://127.0.0.1:1".into(),
        "/r/test/x/".into(),
        MachineKey::open(layout).unwrap(),
        Arc::clone(&store),
        tmt_remote::session::IDLE,
    ));
    let keys = Arc::new(CloudKeys::new(
        layout.clone(),
        MACHINE.into(),
        MachineKey::open(layout).unwrap(),
    ));
    keys.maintain(now).unwrap();
    let descriptor = keys.advertised(now).unwrap();
    // Re-verify the advertised bytes through the public pin owner, as the sender does.
    let pin = MachineKey::open(layout).unwrap().public();
    let descriptor = Descriptor::verify(
        descriptor.bytes(),
        tmt_remote::cloud_hpke::Pin {
            machine_id: MACHINE,
            authentication_key: &pin,
        },
        now,
        None,
    )
    .unwrap();
    let operations = Arc::new(
        tmt_remote::operations::Operations::new(
            tmt_remote::core::CoreClient::at(core.root.join("tmt")).unwrap(),
            Arc::new(AtomicBool::new(false)),
            tmt_remote::limits::CLOUD_ENVELOPE_BYTES,
        )
        .with_cloud_identity(identity),
    );
    let service = Arc::new(
        MessageService::new(
            Arc::clone(&sessions),
            tmt_remote::limits::CLOUD_ENVELOPE_BYTES,
        )
        .with_operations(operations),
    );

    (sessions, keys, service, descriptor)
}
