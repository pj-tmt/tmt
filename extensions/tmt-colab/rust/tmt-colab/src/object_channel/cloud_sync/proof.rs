//! The integrated proof of B3: real `tmt-remote serve` (the `test-emulator` build) against the
//! Auth and Firestore emulators loaded with Colab's own Rules, two real Colab serves on two
//! homes, fixture devices that sync over each serve's socket like paired browsers, and
//! Admin-written documents for what Rules or admission must refuse. Every test here is
//! `#[ignore]`d: the harness script builds the executables, starts the emulators and selects them
//! by name; none of it runs in an ordinary `cargo test`.
mod device;
mod emulator;
mod world;

use crate::keyring::{Keyring, Layout};
use crate::object_channel::documents::{
    Field, Fields, Resource, encode,
    originals::{self, Authority, Routing},
};
use crate::store::{Store, StreamScope};
use base64::Engine;
use device::{Member, Peer, Writer};
use emulator::Emulator;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant},
};
use tmt_extension_objects::sync;
use world::{Inputs, World};

const DEVICE_A: &str = "30000000-0000-4000-8000-0000000000a1";
const DEVICE_B: &str = "30000000-0000-4000-8000-0000000000b1";
const SEED_A: u8 = 41;
const SEED_B: u8 = 42;

/// Wait for a condition the cloud has to produce, with a bound and a readable failure.
fn until(what: &str, mut ready: impl FnMut() -> bool) {
    until_with(what, &mut ready, String::new);
}
/// As [`until`], and a timeout also says what the homes and the cloud looked like.
fn until_with(what: &str, ready: &mut dyn FnMut() -> bool, context: impl Fn() -> String) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for: {what}\n{}",
            context()
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// How many documents of a collection are in the cloud for this page and mention `needle`: a
/// device's originals in the log, or the device chains in the authority records.
fn in_cloud(emulator: &Emulator, page: &str, collection: &str, needle: &str) -> usize {
    emulator
        .ids(collection)
        .iter()
        .filter(|id| id.contains(page) && id.contains(needle))
        .count()
}

/// The public remote reference (`tmt remote add`) for a demo project, with its checksum.
fn remote_link(project: &str) -> String {
    let body = format!(
        r#"{{"kind":"firestore","publicWebConfig":{{"apiKey":"fake-key","appId":"1:12345:web:proof","authDomain":"{project}.firebaseapp.com","projectId":"{project}"}},"version":1}}"#
    );
    let checksum: String = Sha256::digest(body.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let descriptor = format!(r#"{{"checksum":"{checksum}",{}"#, &body[1..]);
    let fragment = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(descriptor);
    format!("https://{project}.web.app/pair#{fragment}")
}
/// Point a home's Remote at the emulator project. The first home signs in; the second reuses
/// that sign-in, which is the owner's second machine under one account.
fn remote_setup(world: &World, inputs: &Inputs) {
    world.remote(&["add", "proof", &remote_link(&inputs.project)]);
    world.remote(&["settings", "default-remote", "proof"]);
}
fn sign_in(world: &World, inputs: &Inputs) -> String {
    let answer = world.run_json(
        "tmt-remote",
        &[
            "--remote",
            "proof",
            "sync",
            "sign-in",
            "--project",
            &inputs.project,
            "--json",
        ],
    );
    answer["uid"].as_str().unwrap().to_owned()
}
fn copy_dir(from: &Path, to: &Path) {
    let status = std::process::Command::new("/bin/cp")
        .arg("-Rp")
        .arg(from)
        .arg(to)
        .status()
        .unwrap();
    assert!(status.success());
}

/// What a home's Store holds about its one page, read the way the serve reads it.
struct Facts {
    layout: Layout,
    space: String,
    page: String,
    epoch: String,
    revision: u64,
    secret: [u8; 32],
}
impl Facts {
    fn read(world: &World, page: &str) -> Self {
        let layout = Layout::open(&world.state).unwrap();
        let key = Keyring::read(&layout).unwrap();
        let store = Store::open(&layout).unwrap();
        let (epoch, secret, revision) = {
            let owner = key.owner_public();
            let head = store.owner_head(&key.space_id, &owner).unwrap().unwrap();
            let (epoch, secret) = store
                .owner_read(&key.space_id, &owner, |tx| {
                    let epoch = tx.page_epoch(page)?.expect("the page has an epoch");
                    let secret = tx.epoch_secret(page, epoch.parse().unwrap())?;
                    Ok((epoch, secret.expect("the owner holds the epoch secret")))
                })
                .unwrap();
            (epoch, secret, head.revision)
        };
        Self {
            space: key.space_id.clone(),
            layout,
            page: page.into(),
            epoch,
            revision,
            secret,
        }
    }
    fn writer(&self) -> Writer<'_> {
        Writer {
            space: &self.space,
            page: &self.page,
            membership_revision: self.revision,
            secret: self.secret,
        }
    }
}

/// The credential of a sign-in, copied to the owner's second machine: one account, two homes.
fn share_sign_in(from: &World, to: &World, project: &str) {
    let source = from
        .state
        .join("remote/firebase-auth")
        .join(format!("{project}.json"));
    let directory = to.state.join("remote/firebase-auth");
    fs::create_dir_all(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let target = directory.join(format!("{project}.json"));
    fs::copy(&source, &target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
}

/// One owner home, running: its door, its serve, and what its Store holds.
struct Running {
    world: World,
    facts: Facts,
    serve: Option<world::Proc>,
    door: Option<world::Proc>,
}
impl Running {
    fn start(&mut self) {
        self.door = Some(self.world.door());
        self.serve = Some(self.world.serve());
    }
    /// Stop the Colab serve and start it again, as a person restarts it. Its first read finds
    /// every device the cloud holds, whichever home published first: a device that appears
    /// later without a sharing change is found by the chain recheck instead, which the worker
    /// and puller tests cover (it waits up to a minute, which a proof should not).
    fn restart(&mut self) {
        if let Some(serve) = self.serve.take() {
            serve.stop(&self.world, "tmt-colab");
        }
        self.serve = Some(self.world.serve());
    }
    fn socket(&self) -> std::path::PathBuf {
        self.serve.as_ref().unwrap().socket()
    }
    /// The Remote door says on stderr that it routes to the emulator only. A proof that never
    /// saw that line would not be about the build it claims to drive.
    fn assert_emulator_routing(&self) {
        let banner = "tmt-remote: test-emulator routing active (loopback, demo project)";
        until("the Remote door announces emulator routing", || {
            self.door.as_ref().unwrap().stderr().contains(banner)
        });
    }
    fn peer(&self, device: &str) -> Peer {
        let facts = &self.facts;
        Peer::connect(
            &self.socket(),
            device,
            &facts.space,
            &facts.page,
            &facts.epoch,
        )
    }
    fn head(&self, stream: &str) -> Option<u64> {
        let store = Store::open(&self.facts.layout).unwrap();
        store
            .stream_head(StreamScope {
                page: &self.facts.page,
                epoch: self.facts.epoch.parse().unwrap(),
                stream,
            })
            .unwrap()
            .map(|(seq, _)| seq)
    }
    fn payload(&self, stream: &str, seq: u64) -> Option<Vec<u8>> {
        let store = Store::open(&self.facts.layout).unwrap();
        store
            .payload(
                StreamScope {
                    page: &self.facts.page,
                    epoch: self.facts.epoch.parse().unwrap(),
                    stream,
                },
                seq,
            )
            .unwrap()
    }
    /// What each process has said, for a failure to explain itself.
    fn said(&self) -> String {
        let log = |name: &str, proc: &Option<world::Proc>| {
            format!(
                "--- {name}\n{}",
                proc.as_ref().map(world::Proc::stderr).unwrap_or_default()
            )
        };
        format!(
            "{}\n{}",
            log("colab serve", &self.serve),
            log("remote door", &self.door)
        )
    }
    fn stop(&mut self) {
        if let Some(serve) = self.serve.take() {
            serve.stop(&self.world, "tmt-colab");
        }
        if let Some(door) = self.door.take() {
            door.stop(&self.world, "tmt-remote");
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Append `count` updates of a device and return their exact bytes.
fn append(
    peer: &mut Peer,
    facts: &Facts,
    device: &str,
    seed: u8,
    from: u64,
    count: u64,
    mut previous: [u8; 32],
) -> (Vec<Vec<u8>>, [u8; 32]) {
    let mut sent = Vec::new();
    for seq in from..from + count {
        let sealed = facts.writer().seal_as(
            device,
            "content",
            seq,
            previous,
            seed,
            &facts.epoch,
            format!("update {seq}").as_bytes(),
        );
        let receipt = peer.append(seq, &sealed);
        assert_eq!(receipt["type"], "receipt", "{receipt}");
        previous = sealed.hash;
        sent.push(sealed.bytes);
    }
    (sent, previous)
}

/// Two homes of one owner (one owner key, one account) each hold a device that writes; every
/// update reaches the other home's Store through Firestore under Colab's real Rules.
#[test]
#[ignore = "run by the cloud-sync emulator harness"]
fn owner_homes_on_one_key_converge_in_both_directions() {
    let inputs = Inputs::from_env();
    let emulator = Emulator::new(&inputs);
    let a = World::new(&inputs, "own-a");
    let b = World::new(&inputs, "own-b");
    for world in [&a, &b] {
        remote_setup(world, &inputs);
    }
    sign_in(&a, &inputs);
    share_sign_in(&a, &b, &inputs.project);
    let created = a.run_json(
        "tmt-colab",
        &["page", "create", "--title", "proof", "--no-open", "--json"],
    );
    let page = created["pageId"].as_str().unwrap().to_owned();
    // The second home starts from the first home's owner state: the same key, membership log,
    // page and epoch secret, and none of its devices.
    copy_dir(&a.state.join("colab"), &b.state.join("colab"));
    for (world, device, seed) in [(&a, DEVICE_A, SEED_A), (&b, DEVICE_B, SEED_B)] {
        let layout = Layout::open(&world.state).unwrap();
        device::register(&layout, &inputs.bins.join("tmt-colab"), &[(device, seed)]);
    }
    let mut first = Running {
        facts: Facts::read(&a, &page),
        world: a,
        serve: None,
        door: None,
    };
    let mut second = Running {
        facts: Facts::read(&b, &page),
        world: b,
        serve: None,
        door: None,
    };
    assert_eq!(first.facts.space, second.facts.space);
    first.start();
    second.start();
    let (mut peer_a, mut peer_b) = (first.peer(DEVICE_A), second.peer(DEVICE_B));
    peer_a.hello(first.facts.revision);
    peer_b.hello(second.facts.revision);
    // Each home writes and publishes before it has read the other's device chain.
    let (sent_a, last_a) = append(&mut peer_a, &first.facts, DEVICE_A, SEED_A, 1, 3, [0; 32]);
    let (sent_b, last_b) = append(&mut peer_b, &second.facts, DEVICE_B, SEED_B, 1, 2, [0; 32]);
    until(
        "the cloud holds both homes' updates and device chains",
        || {
            in_cloud(&emulator, &page, "log", DEVICE_A) == 3
                && in_cloud(&emulator, &page, "log", DEVICE_B) == 2
                && in_cloud(&emulator, &page, "authority", "_chain_") >= 2
        },
    );
    first.assert_emulator_routing();
    second.assert_emulator_routing();
    first.restart();
    second.restart();
    let (mut peer_a, mut peer_b) = (first.peer(DEVICE_A), second.peer(DEVICE_B));
    peer_a.hello(first.facts.revision);
    peer_b.hello(second.facts.revision);
    peer_b.subscribe();
    until_with(
        "the second home holds the first home's updates",
        &mut || second.head(DEVICE_A) == Some(3),
        || {
            let cloud: Vec<_> = ["spaces", "pages", "members", "authority", "log"]
                .iter()
                .map(|c| format!("{c}: {}", emulator.ids(c).len()))
                .collect();
            format!(
                "first home:\n{}\nsecond home:\n{}\ncloud {cloud:?}",
                first.said(),
                second.said()
            )
        },
    );
    for (index, bytes) in sent_a.iter().enumerate() {
        assert_eq!(
            second.payload(DEVICE_A, index as u64 + 1).as_ref(),
            Some(bytes)
        );
    }
    until("the first home holds the second home's updates", || {
        first.head(DEVICE_B) == Some(2)
    });
    for (index, bytes) in sent_b.iter().enumerate() {
        assert_eq!(
            first.payload(DEVICE_B, index as u64 + 1).as_ref(),
            Some(bytes)
        );
    }
    // With every device known, later updates cross while both serves keep running.
    let (more_a, _) = append(&mut peer_a, &first.facts, DEVICE_A, SEED_A, 4, 2, last_a);
    let (more_b, _) = append(&mut peer_b, &second.facts, DEVICE_B, SEED_B, 3, 1, last_b);
    until(
        "the second home holds the first home's later updates",
        || second.head(DEVICE_A) == Some(5),
    );
    until(
        "the first home holds the second home's later update",
        || first.head(DEVICE_B) == Some(3),
    );
    for (offset, bytes) in more_a.iter().enumerate() {
        assert_eq!(
            second.payload(DEVICE_A, offset as u64 + 4).as_ref(),
            Some(bytes)
        );
    }
    // What a home took from the cloud while a browser was connected was announced to it: the
    // updates before were either announced or handed over by the catch-up, whichever the
    // browser's subscription met first.
    let mut announced = Vec::new();
    loop {
        let frame = peer_b.next_update(DEVICE_A);
        let seq: u64 = frame["seq"].as_str().unwrap().parse().unwrap();
        announced.push(seq);
        if seq == 5 {
            break;
        }
    }
    assert!(announced.ends_with(&[4, 5]), "{announced:?}");
    assert_eq!(first.payload(DEVICE_B, 3).as_ref(), Some(&more_b[0]));
    println!("cloud collections: {:?}", emulator.collections());
    first.stop();
    second.stop();
}

const MEMBER: Member = Member {
    id: "40000000-0000-4000-8000-0000000000e1",
    signing_seed: 60,
    encryption_seed: 61,
};
/// A second member, who may comment but not edit.
const COMMENTER: Member = Member {
    id: "40000000-0000-4000-8000-0000000000e2",
    signing_seed: 62,
    encryption_seed: 63,
};
const DEVICE_M: &str = "30000000-0000-4000-8000-0000000000c1";
const SEED_M: u8 = 70;
/// Devices whose originals are wrong in exactly one way each.
const FORGED_SIGNATURE: (&str, u8) = ("30000000-0000-4000-8000-0000000000d1", 71);
const FORGED_EPOCH: (&str, u8) = ("30000000-0000-4000-8000-0000000000d2", 72);
const FORGED_WRITER: (&str, u8) = ("30000000-0000-4000-8000-0000000000d4", 75);
const COMMENTER_DEVICE: (&str, u8) = ("30000000-0000-4000-8000-0000000000d3", 73);

fn routing<'a>(facts: &'a Facts, page: &'a str, uid: &'a str, epoch: u64) -> Routing<'a> {
    Routing {
        space: &facts.space,
        page,
        epoch,
        uid,
        expires_at: Field::timestamp(device::now() + 30 * 86_400_000).unwrap(),
    }
}
fn text(value: &str) -> Field {
    Field::String(value.into())
}
/// Count the lines of a serve's stderr that contain `needle`.
fn lines_with(running: &Running, needle: &str) -> usize {
    running
        .serve
        .as_ref()
        .unwrap()
        .stderr()
        .lines()
        .filter(|line| line.contains(needle))
        .count()
}
/// The owner admits a member through the owner service, as a person would, and the hash of the
/// statement that did so.
fn admit(world: &World, page: &str, member: &Member, role: &str) -> [u8; 32] {
    // A key may begin with `-`, so each is passed as `--option=value`.
    let sign = format!("--sign-key={}", member.signing_key());
    let enc = format!("--enc-key={}", member.encryption_key());
    let added = world.run_json(
        "tmt-colab",
        &[
            "share", "member", "add", page, member.id, role, &sign, &enc, "--yes", "--json",
        ],
    );
    let hash = added["membershipHead"]["statementHash"].as_str().unwrap();
    tmt_colab_model::values::binary(hash, 32)
        .unwrap()
        .try_into()
        .unwrap()
}
/// What the member's row in the cloud says.
fn member_row(facts: &Facts, page: &str, uid: &str, role: &str) -> sync::Document {
    encode(
        Resource::Members,
        Fields::from_iter([
            ("space".into(), text(&facts.space)),
            ("page".into(), text(page)),
            ("uid".into(), text(uid)),
            ("role".into(), text(role)),
            ("link".into(), Field::Null(())),
            ("token".into(), Field::Null(())),
        ]),
    )
    .unwrap()
}

/// A member in the owner's grant writes from its own Firebase identity; the owner's two homes
/// take those updates. What Rules cannot see is refused by the homes: a signature that is not
/// the device's, another epoch, a header that names another author, a commenter's write to
/// the content. A stranger is refused by
/// Rules themselves, and none of it disturbs the owner's own devices.
#[test]
#[ignore = "run by the cloud-sync emulator harness"]
fn a_member_converges_inside_its_grant_and_nothing_forged_or_outside_it_does() {
    let inputs = Inputs::from_env();
    let emulator = Emulator::new(&inputs);
    let a = World::new(&inputs, "mem-a");
    let b = World::new(&inputs, "mem-b");
    for world in [&a, &b] {
        remote_setup(world, &inputs);
    }
    let owner_uid = sign_in(&a, &inputs);
    share_sign_in(&a, &b, &inputs.project);
    let created = a.run_json(
        "tmt-colab",
        &["page", "create", "--title", "proof", "--no-open", "--json"],
    );
    let page = created["pageId"].as_str().unwrap().to_owned();
    let editor_statement = admit(&a, &page, &MEMBER, "editor");
    let commenter_statement = admit(&a, &page, &COMMENTER, "commenter");
    copy_dir(&a.state.join("colab"), &b.state.join("colab"));
    for (world, device, seed) in [(&a, DEVICE_A, SEED_A), (&b, DEVICE_B, SEED_B)] {
        let layout = Layout::open(&world.state).unwrap();
        device::register(&layout, &inputs.bins.join("tmt-colab"), &[(device, seed)]);
    }
    let mut first = Running {
        facts: Facts::read(&a, &page),
        world: a,
        serve: None,
        door: None,
    };
    let mut second = Running {
        facts: Facts::read(&b, &page),
        world: b,
        serve: None,
        door: None,
    };
    first.start();
    second.start();
    // The same Store facts, read apart, so the homes can be restarted while they are used.
    let facts = Facts::read(&first.world, &page);
    let facts = &facts;
    let (mut peer_a, mut peer_b) = (first.peer(DEVICE_A), second.peer(DEVICE_B));
    peer_a.hello(facts.revision);
    peer_b.hello(second.facts.revision);
    until("the owner's page is in the cloud", || {
        emulator.ids("pages").iter().any(|id| id.ends_with(&page))
    });
    first.assert_emulator_routing();
    // The members' rows and device chains are what the owner's client publishes once member
    // enrollment exists; here the fixture seeds them by Admin.
    let editor = emulator.sign_up();
    let commenter = emulator.sign_up();
    assert_eq!(
        emulator.put(&member_row(facts, &page, &editor.uid, "editor"), None),
        200
    );
    assert_eq!(
        emulator.put(&member_row(facts, &page, &commenter.uid, "commenter"), None),
        200
    );
    for (member, statement, revision, device, seed) in [
        (&MEMBER, editor_statement, 3, DEVICE_M, SEED_M),
        (
            &MEMBER,
            editor_statement,
            3,
            FORGED_SIGNATURE.0,
            FORGED_SIGNATURE.1,
        ),
        (&MEMBER, editor_statement, 3, FORGED_EPOCH.0, FORGED_EPOCH.1),
        (
            &MEMBER,
            editor_statement,
            3,
            FORGED_WRITER.0,
            FORGED_WRITER.1,
        ),
        (
            &COMMENTER,
            commenter_statement,
            4,
            COMMENTER_DEVICE.0,
            COMMENTER_DEVICE.1,
        ),
    ] {
        let chain = member.chain(&facts.space, statement, revision, device, seed);
        for document in originals::authority_documents(
            &routing(facts, &page, &owner_uid, 1),
            Authority::Chain,
            "",
            &chain,
        )
        .unwrap()
        {
            assert_eq!(emulator.put(&document, None), 200);
        }
    }
    // The owner's own device writes once, and the cloud holds it with its chain. Then both
    // homes are restarted, and their first read finds every device: the owner's, the members'
    // and the wrong ones.
    let (sent_first, last_first) = append(&mut peer_a, facts, DEVICE_A, SEED_A, 1, 1, [0; 32]);
    until(
        "the cloud holds the owner's update and every device chain",
        || {
            in_cloud(&emulator, &page, "log", DEVICE_A) == 1
                && in_cloud(&emulator, &page, "authority", "_chain_") >= 6
        },
    );
    first.restart();
    second.restart();
    let (mut peer_a, mut peer_b) = (first.peer(DEVICE_A), second.peer(DEVICE_B));
    peer_a.hello(facts.revision);
    peer_b.hello(second.facts.revision);
    until("the second home holds the owner's first update", || {
        second.head(DEVICE_A) == Some(1)
    });
    assert_eq!(second.payload(DEVICE_A, 1).as_ref(), Some(&sent_first[0]));
    // A member's own writes go through Rules as that member. A record is routed to the page's
    // epoch 1 whatever epoch its signed header says.
    let write = |who: &emulator::Identity,
                 device: &str,
                 seq: u64,
                 previous: [u8; 32],
                 seed: u8,
                 epoch: &str| {
        let sealed = facts.writer().seal_as(
            device,
            "content",
            seq,
            previous,
            seed,
            epoch,
            b"member update",
        );
        let signed_epoch: u64 = epoch.parse().unwrap();
        let mut document = originals::log_document(
            &routing(facts, &page, &who.uid, signed_epoch),
            &sealed.bytes,
        )
        .unwrap();
        if signed_epoch != 1 {
            document
                .fields
                .insert("epoch".into(), serde_json::json!({"integerValue": "1"}));
            document.target.id =
                document
                    .target
                    .id
                    .replacen(&format!("_{signed_epoch}_"), "_1_", 1);
        }
        (emulator.put(&document, Some(who)), sealed)
    };
    let (status, first_update) = write(&editor, DEVICE_M, 1, [0; 32], SEED_M, "1");
    assert_eq!(status, 200);
    let (status, second_update) = write(&editor, DEVICE_M, 2, first_update.hash, SEED_M, "1");
    assert_eq!(status, 200);
    // Rules admit these three, which only the homes can tell are wrong.
    assert_eq!(
        write(&editor, FORGED_SIGNATURE.0, 1, [0; 32], 99, "1").0,
        200
    );
    assert_eq!(
        write(&editor, FORGED_EPOCH.0, 1, [0; 32], FORGED_EPOCH.1, "2").0,
        200
    );
    assert_eq!(
        write(
            &commenter,
            COMMENTER_DEVICE.0,
            1,
            [0; 32],
            COMMENTER_DEVICE.1,
            "1"
        )
        .0,
        200
    );
    // A record in one device's stream whose signed header names another device as its author.
    let (_, borrowed) = (
        0,
        facts
            .writer()
            .seal_as(DEVICE_M, "content", 1, [0; 32], SEED_M, "1", b"borrowed"),
    );
    let mut document =
        originals::log_document(&routing(facts, &page, &editor.uid, 1), &borrowed.bytes).unwrap();
    document.fields.insert(
        "stream".into(),
        serde_json::json!({"stringValue": FORGED_WRITER.0}),
    );
    document.target.id = document.target.id.replacen(DEVICE_M, FORGED_WRITER.0, 1);
    assert_eq!(emulator.put(&document, Some(&editor)), 200);
    // A stranger holds no member row: Rules refuse the write itself.
    let stranger = emulator.sign_up();
    assert_eq!(
        write(&stranger, DEVICE_M, 3, second_update.hash, SEED_M, "1").0,
        403
    );

    for home in [&first, &second] {
        until_with(
            "a home holds the member's updates",
            &mut || home.head(DEVICE_M) == Some(2),
            || format!("first:\n{}\nsecond:\n{}", first.said(), second.said()),
        );
        assert_eq!(
            home.payload(DEVICE_M, 1).as_ref(),
            Some(&first_update.bytes)
        );
        assert_eq!(
            home.payload(DEVICE_M, 2).as_ref(),
            Some(&second_update.bytes)
        );
    }
    // Each wrong writer is refused once by each home, and nothing of theirs is stored.
    for home in [&first, &second] {
        until_with(
            "a home refuses each wrong writer",
            &mut || lines_with(home, "one writer") >= 4,
            || format!("first:\n{}\nsecond:\n{}", first.said(), second.said()),
        );
        for (device, _) in [
            FORGED_SIGNATURE,
            FORGED_EPOCH,
            FORGED_WRITER,
            COMMENTER_DEVICE,
        ] {
            assert_eq!(home.head(device), None, "{device} was stored");
        }
    }
    std::thread::sleep(Duration::from_secs(3));
    for home in [&first, &second] {
        assert_eq!(lines_with(home, "one writer"), 4, "{}", home.said());
    }
    // The owner's own devices are not disturbed: a later update still crosses.
    let (sent, _) = append(
        &mut peer_a,
        &first.facts,
        DEVICE_A,
        SEED_A,
        2,
        1,
        last_first,
    );
    until_with(
        "the owner's update crosses after the refusals",
        &mut || second.head(DEVICE_A) == Some(2),
        || {
            let in_cloud = in_cloud(&emulator, &page, "log", DEVICE_A);
            format!(
                "the first home holds {:?} and the cloud {in_cloud} of its originals\nfirst:\n{}\nsecond:\n{}",
                first.head(DEVICE_A),
                first.said(),
                second.said()
            )
        },
    );
    assert_eq!(second.payload(DEVICE_A, 2).as_ref(), Some(&sent[0]));
    let _ = &mut peer_b;
    first.stop();
    second.stop();
}
