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
use serde_json::Value;
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
        &[
            "page",
            "create",
            "--title",
            "proof",
            "--layout",
            "column",
            "--no-open",
            "--json",
        ],
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
            ("seq".into(), Field::number(1).unwrap()),
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
        &[
            "page",
            "create",
            "--title",
            "proof",
            "--layout",
            "column",
            "--no-open",
            "--json",
        ],
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

/// The expiry the cloud holds for a page, as the provider prints it (sortable text).
fn page_expiry(emulator: &Emulator, page: &str) -> String {
    emulator
        .list("pages")
        .iter()
        .find(|document| document["name"].as_str().unwrap().ends_with(page))
        .and_then(|document| document["fields"]["expiresAt"]["timestampValue"].as_str())
        .unwrap_or_default()
        .to_owned()
}

/// A cloud copy that lapsed while its owner was away: the page row has expired and reads are
/// refused, and reopening the page from local writes it again without losing what was there.
#[test]
#[ignore = "run by the cloud-sync emulator harness"]
fn an_expired_cloud_copy_is_written_again_when_the_page_is_reopened_from_local() {
    let inputs = Inputs::from_env();
    let emulator = Emulator::new(&inputs);
    let world = World::new(&inputs, "exp-a");
    remote_setup(&world, &inputs);
    sign_in(&world, &inputs);
    let created = world.run_json(
        "tmt-colab",
        &[
            "page",
            "create",
            "--title",
            "proof",
            "--layout",
            "column",
            "--no-open",
            "--json",
        ],
    );
    let page = created["pageId"].as_str().unwrap().to_owned();
    let layout = Layout::open(&world.state).unwrap();
    device::register(
        &layout,
        &inputs.bins.join("tmt-colab"),
        &[(DEVICE_A, SEED_A)],
    );
    let mut home = Running {
        facts: Facts::read(&world, &page),
        world,
        serve: None,
        door: None,
    };
    home.start();
    let mut peer = home.peer(DEVICE_A);
    peer.hello(home.facts.revision);
    // The page the real CLI created is published too, before the fixture device writes anything
    // a decoder would refuse. Its content and every browser save are sealed by the root-local
    // writer this machine's keyring names, which no registration backs.
    let key = Keyring::read(&home.facts.layout).unwrap();
    let writer = key.local_writer().unwrap().0;
    until("the content the real CLI created reaches the cloud", || {
        in_cloud(&emulator, &page, "log", &writer) >= 1
    });
    let created = in_cloud(&emulator, &page, "log", &writer);
    let source = home
        .world
        .run_json("tmt-colab", &["page", "read", &page, "--json"])["source"]
        .as_str()
        .unwrap()
        .to_owned();
    let saved = peer.save(
        "50000000-0000-4000-8000-000000000001",
        &source,
        &format!("{source}<p>saved in a browser</p>"),
    );
    assert_eq!(saved["state"], "committed", "{saved}");
    until("the browser save reaches the cloud", || {
        in_cloud(&emulator, &page, "log", &writer) > created
    });
    let (_, last) = append(&mut peer, &home.facts, DEVICE_A, SEED_A, 1, 2, [0; 32]);
    until("the cloud holds the page and its updates", || {
        in_cloud(&emulator, &page, "log", DEVICE_A) == 2
            && in_cloud(&emulator, &page, "pages", &page) == 1
    });
    home.assert_emulator_routing();
    // An owner override reaches the cloud copy without a restart: kept forever locally, the page
    // row takes the 365-day maximum on the next activity.
    let thirty = page_expiry(&emulator, &page);
    home.world
        .run_json("tmt-colab", &["retention", &page, "forever", "--json"]);
    // The signed policy is a new membership revision; a browser opens the page at that one.
    home.facts = Facts::read(&home.world, &page);
    let mut peer = home.peer(DEVICE_A);
    peer.hello(home.facts.revision);
    let (_, last) = append(&mut peer, &home.facts, DEVICE_A, SEED_A, 3, 1, last);
    let beyond = Field::timestamp(device::now() + 300 * 86_400_000).unwrap();
    let beyond = serde_json::to_value(&beyond).unwrap()["timestampValue"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(thirty < beyond, "{thirty} is the 180-day default");
    until("the page row takes the maximum cloud lifetime", || {
        page_expiry(&emulator, &page) > beyond
    });
    // A viewer the owner admitted can read the copy while it lives.
    let viewer = emulator.sign_up();
    assert_eq!(
        emulator.put(&member_row(&home.facts, &page, &viewer.uid, "viewer"), None),
        200
    );
    let entry = emulator
        .ids("log")
        .into_iter()
        .find(|id| id.contains(&page) && id.contains(DEVICE_A))
        .unwrap();
    assert_eq!(emulator.get_as("log", &entry, &viewer), 200);
    // The owner is away past the cloud lifetime: the page row has expired, so every read of the
    // page is refused, though its entries have not.
    let row = emulator
        .ids("pages")
        .into_iter()
        .find(|id| id.contains(&page))
        .unwrap();
    let lapsed = Field::timestamp(device::now() - 86_400_000).unwrap();
    assert_eq!(emulator.set_expiry("pages", &row, &lapsed), 200);
    assert_eq!(emulator.get_as("log", &entry, &viewer), 403);
    // The page is reopened from local: the row is written again, the copy reads as before, and
    // new local work is published on top of it.
    home.restart();
    let mut peer = home.peer(DEVICE_A);
    peer.hello(home.facts.revision);
    until("the page's cloud copy is written again", || {
        emulator.get_as("log", &entry, &viewer) == 200
    });
    let (_, _) = append(&mut peer, &home.facts, DEVICE_A, SEED_A, 4, 1, last);
    until("the update made after reopening is published", || {
        in_cloud(&emulator, &page, "log", DEVICE_A) == 4
    });
    home.stop();
}

/// The cloud documents of a collection for this page, as (ID, fields), by Admin bypass.
fn page_documents(emulator: &Emulator, page: &str, collection: &str) -> Vec<(String, Value)> {
    emulator
        .list(collection)
        .into_iter()
        .filter_map(|document| {
            let id = document["name"].as_str()?.rsplit('/').next()?.to_owned();
            id.contains(page).then(|| (id, document["fields"].clone()))
        })
        .collect()
}
/// The IDs of the authority wraps addressed to `recipient`.
fn wraps_for(emulator: &Emulator, page: &str, recipient: &str) -> Vec<String> {
    page_documents(emulator, page, "authority")
        .into_iter()
        .filter(|(_, fields)| {
            fields["kind"]["stringValue"] == "wrap"
                && fields["recipient"]["stringValue"] == recipient
        })
        .map(|(id, _)| id)
        .collect()
}

/// A share link reaches the cloud as a row and a wrap addressed to the link; a holder with no
/// account of the owner's enrolls by the link's token, reads exactly what the link grants, and
/// loses it all when the link is removed, with no edit to wake the worker.
#[test]
#[ignore = "run by the cloud-sync emulator harness"]
fn a_link_holder_enrolls_by_the_links_token_and_is_removed_with_the_link() {
    let inputs = Inputs::from_env();
    let emulator = Emulator::new(&inputs);
    let world = World::new(&inputs, "lnk-a");
    remote_setup(&world, &inputs);
    sign_in(&world, &inputs);
    let created = world.run_json(
        "tmt-colab",
        &[
            "page",
            "create",
            "--title",
            "proof",
            "--layout",
            "column",
            "--no-open",
            "--json",
        ],
    );
    let page = created["pageId"].as_str().unwrap().to_owned();
    world.run_json(
        "tmt-colab",
        &["share", "mode", &page, "link", "--yes", "--json"],
    );
    let mut home = Running {
        facts: Facts::read(&world, &page),
        world,
        serve: None,
        door: None,
    };
    home.start();

    // The link is added while the page is being served and nothing is edited or opened: the
    // worker is woken by the change itself.
    let added = home.world.run_json(
        "tmt-colab",
        &["share", "link", "add", &page, "--yes", "--json"],
    );
    let link = added["linkId"].as_str().unwrap().to_owned();
    let reader = added["readerPath"].as_str().unwrap();
    let fragment: std::collections::HashMap<_, _> = reader
        .split_once('#')
        .unwrap()
        .1
        .split('&')
        .map(|part| part.split_once('=').unwrap())
        .collect();
    let seed: [u8; 32] = tmt_colab_model::values::binary(fragment["seed"], 32)
        .unwrap()
        .try_into()
        .unwrap();
    let token = tmt_colab_model::link::Keys::derive(&seed, &home.facts.space, &link)
        .unwrap()
        .rules_token();
    let row = format!("{}_{page}_{link}", home.facts.space);
    until("the link's row and wrap are in the cloud", || {
        emulator.ids("links").contains(&row)
            && !wraps_for(&emulator, &page, &format!("link:{link}")).is_empty()
    });
    home.assert_emulator_routing();
    // The link was never opened here, yet the chains of its derived devices, one per transport,
    // are in the cloud, byte for byte what a holder derives from the seed and the statement the
    // link names: whoever follows the link's streams finds the devices there.
    let statement: [u8; 32] = tmt_colab_model::values::binary(fragment["st"], 32)
        .unwrap()
        .try_into()
        .unwrap();
    for transport in tmt_colab_model::link::Transport::ALL {
        let chain = tmt_colab_model::link::device_chain(
            &seed,
            &home.facts.space,
            &link,
            fragment["rev"],
            &statement,
            transport,
        )
        .unwrap();
        let subject =
            tmt_extension_objects::Sha256Hex::from_bytes(Sha256::digest(&chain).into()).to_string();
        until(
            &format!("the link's {transport:?} device chain is in the cloud"),
            || {
                page_documents(&emulator, &page, "authority")
                    .iter()
                    .any(|(_, fields)| {
                        fields["kind"]["stringValue"] == "chain"
                            && fields["subject"]["stringValue"] == subject.as_str()
                    })
            },
        );
    }
    let owner_wrap = page_documents(&emulator, &page, "authority")
        .into_iter()
        .find(|(_, fields)| {
            fields["kind"]["stringValue"] == "wrap"
                && !fields["recipient"]["stringValue"]
                    .as_str()
                    .unwrap()
                    .starts_with("link:")
        })
        .expect("the owner's own wrap")
        .0;
    let link_wrap = wraps_for(&emulator, &page, &format!("link:{link}")).remove(0);
    let links = page_documents(&emulator, &page, "links");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].1["token"]["stringValue"], token.as_str());
    assert_eq!(links[0].1["role"]["stringValue"], "viewer");

    // A holder enrolls with the link's token and nothing else of the owner's.
    let holder = emulator.sign_up();
    let stranger = emulator.sign_up();
    let enroll = |uid: &str, token: &str| {
        encode(
            Resource::Members,
            Fields::from_iter([
                ("space".into(), text(&home.facts.space)),
                ("page".into(), text(&page)),
                ("uid".into(), text(uid)),
                ("role".into(), text("viewer")),
                ("link".into(), text(&link)),
                ("token".into(), text(token)),
                ("seq".into(), Field::number(1).unwrap()),
            ]),
        )
        .unwrap()
    };
    assert_eq!(
        emulator.put(&enroll(&stranger.uid, &"X".repeat(43)), Some(&stranger)),
        403,
        "a wrong token enrolls nobody"
    );
    assert_eq!(
        emulator.put(&enroll(&holder.uid, &token), Some(&holder)),
        200,
        "the link's token enrolls its holder"
    );
    // What a holder of the link may read besides its own wrap: the signed membership records.
    let membership = page_documents(&emulator, &page, "authority")
        .into_iter()
        .find(|(_, fields)| fields["kind"]["stringValue"] == "membership")
        .expect("the signed membership records")
        .0;
    assert_eq!(emulator.get_as("authority", &link_wrap, &holder), 200);
    assert_eq!(
        emulator.get_as("pages", &format!("{}_{page}", home.facts.space), &holder),
        200
    );
    assert_eq!(emulator.get_as("authority", &membership, &holder), 200);
    assert_eq!(
        emulator.get_as("authority", &owner_wrap, &holder),
        403,
        "the owner's own wrap is not the link's"
    );
    assert_eq!(emulator.get_as("authority", &link_wrap, &stranger), 403);
    assert_eq!(emulator.get_as("authority", &membership, &stranger), 403);

    // Removing the link takes the row, the holder's enrollment and every read with it, again
    // with no edit to wake the worker: the row goes first, then the holders it enrolled.
    home.world.run_json(
        "tmt-colab",
        &["share", "link", "remove", &page, &link, "--json"],
    );
    until(
        "the link's row and its holder are gone from the cloud",
        || {
            !emulator.ids("links").contains(&row)
                && !emulator
                    .ids("members")
                    .iter()
                    .any(|id| id.ends_with(&holder.uid))
        },
    );
    assert_eq!(emulator.get_as("authority", &membership, &holder), 403);
    assert_eq!(emulator.get_as("authority", &link_wrap, &holder), 403);
    assert_eq!(
        emulator.get_as("pages", &format!("{}_{page}", home.facts.space), &holder),
        403
    );
    assert_eq!(
        emulator.put(&enroll(&holder.uid, &token), Some(&holder)),
        403,
        "the removed link enrolls nobody again"
    );
    home.stop();
}

/// The chunks of the checkpoint objects published for `epoch` of this page, as
/// (object ID, chunk IDs in order, reassembled bytes).
fn epoch_objects(
    emulator: &Emulator,
    page: &str,
    epoch: u64,
) -> Vec<(String, Vec<String>, Vec<u8>)> {
    let mut chunks: Vec<(String, u64, String, Vec<u8>)> =
        page_documents(emulator, page, "checkpoints")
            .into_iter()
            .filter(|(_, fields)| fields["epoch"]["integerValue"] == epoch.to_string().as_str())
            .map(|(id, fields)| {
                let index = fields["index"]["integerValue"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap();
                let bytes = tmt_colab_model::values::binary(
                    fields["bytes"]["stringValue"].as_str().unwrap(),
                    32_768,
                )
                .unwrap();
                let object = fields["object"]["stringValue"].as_str().unwrap().to_owned();
                (object, index, id, bytes)
            })
            .collect();
    chunks.sort();
    let mut objects: Vec<(String, Vec<String>, Vec<u8>)> = Vec::new();
    for (object, _, id, bytes) in chunks {
        match objects.last_mut() {
            Some((last, ids, whole)) if *last == object => {
                ids.push(id);
                whole.extend_from_slice(&bytes);
            }
            _ => objects.push((object, vec![id], bytes)),
        }
    }
    objects
}

/// Rotating the epoch publishes that epoch's exact signed baseline, under the hash the signed
/// `epoch.advance` names, before the wraps of the new epoch; the first epoch has none; a holder of
/// the replacement link reads it whole.
#[test]
#[ignore = "run by the cloud-sync emulator harness"]
fn a_new_epochs_baseline_reaches_the_cloud_before_its_wraps_and_a_link_holder_reads_it() {
    let inputs = Inputs::from_env();
    let emulator = Emulator::new(&inputs);
    let world = World::new(&inputs, "bsl-a");
    remote_setup(&world, &inputs);
    sign_in(&world, &inputs);
    let created = world.run_json(
        "tmt-colab",
        &[
            "page",
            "create",
            "--title",
            "proof",
            "--layout",
            "column",
            "--no-open",
            "--json",
        ],
    );
    let page = created["pageId"].as_str().unwrap().to_owned();
    world.run_json(
        "tmt-colab",
        &["share", "mode", &page, "link", "--yes", "--json"],
    );
    let mut home = Running {
        facts: Facts::read(&world, &page),
        world,
        serve: None,
        door: None,
    };
    home.start();
    let added = home.world.run_json(
        "tmt-colab",
        &["share", "link", "add", &page, "--yes", "--json"],
    );
    let old = added["linkId"].as_str().unwrap().to_owned();
    until("the first epoch's link wrap is in the cloud", || {
        !wraps_for(&emulator, &page, &format!("link:{old}")).is_empty()
    });
    assert!(
        page_documents(&emulator, &page, "checkpoints").is_empty(),
        "the first epoch has no baseline"
    );

    // Resetting the link revokes the old one and rotates the page to epoch 2.
    let reset = home.world.run_json(
        "tmt-colab",
        &["share", "link", "reset", &page, &old, "--yes", "--json"],
    );
    let link = reset["linkId"].as_str().unwrap().to_owned();
    assert_ne!(link, old);
    let reader = reset["readerPath"].as_str().unwrap();
    let seed: [u8; 32] = tmt_colab_model::values::binary(
        reader
            .split_once('#')
            .unwrap()
            .1
            .split('&')
            .find_map(|part| part.strip_prefix("seed="))
            .unwrap(),
        32,
    )
    .unwrap()
    .try_into()
    .unwrap();
    let token = tmt_colab_model::link::Keys::derive(&seed, &home.facts.space, &link)
        .unwrap()
        .rules_token();
    let recipient = format!("link:{link}");
    until("the new epoch's link wrap is in the cloud", || {
        !wraps_for(&emulator, &page, &recipient).is_empty()
    });
    home.assert_emulator_routing();

    // The wrap is there, so the whole baseline already is: it was confirmed first.
    let objects = epoch_objects(&emulator, &page, 2);
    assert_eq!(objects.len(), 1, "one baseline object for the new epoch");
    let (object, ids, bytes) = &objects[0];
    let facts = Facts::read(&home.world, &page);
    assert_eq!(facts.epoch, "2");
    let key = Keyring::read(&facts.layout).unwrap();
    let store = Store::open(&facts.layout).unwrap();
    let stored = store
        .owner_read(&key.space_id, &key.owner_public(), |tx| {
            tx.baseline(&page, 2)
        })
        .unwrap()
        .expect("the owner holds the epoch's baseline");
    assert_eq!(
        *bytes, stored.envelope,
        "the cloud holds the owner's exact sealed baseline"
    );
    let named = serde_json::from_slice::<Value>(&stored.descriptor).unwrap()["objectEnvelopeHash"]
        .as_str()
        .unwrap()
        .to_owned();
    let hash = tmt_colab_model::values::binary(&named, 32).unwrap();
    assert_eq!(
        *object,
        hash.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "published under the hash the signed epoch.advance names"
    );
    assert!(
        ids.iter()
            .enumerate()
            .all(|(index, id)| { *id == format!("{}_{page}_2_{object}_{index}", facts.space) })
    );

    // A holder of the replacement link enrolls with its token and reads the chunks whole.
    let holder = emulator.sign_up();
    let member = encode(
        Resource::Members,
        Fields::from_iter([
            ("space".into(), text(&facts.space)),
            ("page".into(), text(&page)),
            ("uid".into(), text(&holder.uid)),
            ("role".into(), text("viewer")),
            ("link".into(), text(&link)),
            ("token".into(), text(&token)),
            ("seq".into(), Field::number(1).unwrap()),
        ]),
    )
    .unwrap();
    assert_eq!(emulator.put(&member, Some(&holder)), 200);
    for id in ids {
        assert_eq!(emulator.get_as("checkpoints", id, &holder), 200);
    }
    let stranger = emulator.sign_up();
    assert_eq!(emulator.get_as("checkpoints", &ids[0], &stranger), 403);
    home.stop();
}
