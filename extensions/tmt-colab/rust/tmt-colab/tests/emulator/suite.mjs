// Firestore Rules conformance for Colab's admission fragment, loaded the way it
// deploys: the composed golden Remote produces from Colab's own vector, not the bare fragment.
// Run only under `firebase emulators:exec --only firestore` (see the tmt-colab skill); there is
// no skip path: without the emulator the suite fails.
//
// The Rules admit ciphertext and routing facts, never authority (clients verify statements), so
// every case here is about who may reach which space, page and stream, and in what order.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const host = process.env.FIRESTORE_EMULATOR_HOST;
assert.ok(host, "FIRESTORE_EMULATOR_HOST is unset: run under `firebase emulators:exec --only firestore`");
const PROJECT = "demo-tmt-colab";
const vectors = new URL("../../../../contracts/vectors/", import.meta.url);
const composed = readFileSync(new URL("deploy-declaration-v1.firestore.rules", vectors), "utf8");
const documents = `http://${host}/v1/projects/${PROJECT}/databases/(default)/documents`;
const ROOT = "x/colab";
const ADMIN = { authorization: "Bearer owner", "content-type": "application/json" };

const b64 = (value) => Buffer.from(JSON.stringify(value)).toString("base64url");
// The emulator accepts unsigned tokens and reads request.auth from the payload.
const as = (uid) => ({
  authorization: `Bearer ${b64({ alg: "none", typ: "JWT" })}.${b64({ user_id: uid, sub: uid, firebase: { sign_in_provider: "anonymous" } })}.`,
  "content-type": "application/json",
});
const NOBODY = { "content-type": "application/json" };
const typed = (value) =>
  value instanceof Date ? { timestampValue: value.toISOString() }
  : typeof value === "boolean" ? { booleanValue: value }
  : typeof value === "number" ? { integerValue: String(value) }
  : { stringValue: String(value) };
const fields = (data) => Object.fromEntries(Object.entries(data).map(([k, v]) => [k, typed(v)]));
const DAY = 86_400_000;
const future = new Date(Date.now() + 30 * DAY);
const sooner = new Date(Date.now() + 10 * DAY);
const past = new Date(Date.now() - DAY);

async function loadRules(content) {
  const response = await fetch(`http://${host}/emulator/v1/projects/${PROJECT}:securityRules`, {
    method: "PUT",
    headers: ADMIN,
    body: JSON.stringify({ rules: { files: [{ name: "firestore.rules", content }] } }),
  });
  assert.equal(response.status, 200, `rules did not load: ${await response.text()}`);
}
async function clear() {
  const response = await fetch(`http://${host}/emulator/v1/projects/${PROJECT}/databases/(default)/documents`, { method: "DELETE" });
  assert.equal(response.status, 200);
}
/** Writes a whole document: a create when the ID is new, an update when it exists. 200 admitted, 403 refused. */
const set = async (path, data, headers) =>
  (await fetch(`${documents}/${ROOT}/${path}`, { method: "PATCH", headers, body: JSON.stringify({ fields: fields(data) }) })).status;
const remove = async (path, headers) => (await fetch(`${documents}/${ROOT}/${path}`, { method: "DELETE", headers })).status;
const get = async (path, headers = NOBODY) => (await fetch(`${documents}/${ROOT}/${path}`, { headers })).status;
const seed = async (path, data) => assert.equal(await set(path, data, ADMIN), 200, `seed ${path}`);
/** A collection query with equality filters; Rules cannot read the query, so these must prove admission themselves. */
async function list(collection, equal, headers) {
  const filter = (field, value) => ({ fieldFilter: { field: { fieldPath: field }, op: "EQUAL", value: typed(value) } });
  const filters = Object.entries(equal).map(([field, value]) => filter(field, value));
  const where = filters.length === 0 ? undefined : filters.length === 1 ? filters[0] : { compositeFilter: { op: "AND", filters } };
  const response = await fetch(`${documents}/${ROOT}:runQuery`, {
    method: "POST",
    headers,
    body: JSON.stringify({ structuredQuery: { from: [{ collectionId: collection }], where } }),
  });
  if (response.status !== 200) return { status: response.status, found: 0 };
  const rows = await response.json();
  return { status: 200, found: rows.filter((row) => row.document).length };
}
/** Atomic commit of several creates, which is how a batched append reaches the Rules. */
async function commit(writes, headers) {
  const response = await fetch(`${documents}:commit`, {
    method: "POST",
    headers,
    body: JSON.stringify({
      writes: writes.map(([path, data]) => ({
        update: { name: `projects/${PROJECT}/databases/(default)/documents/${ROOT}/${path}`, fields: fields(data) },
        currentDocument: { exists: false },
      })),
    }),
  });
  return response.status;
}

const T = {
  A: { space: "a".repeat(32), page: "10000000-0000-4000-8000-000000000001", stream: "20000000-0000-4000-8000-000000000001", link: "30000000-0000-4000-8000-000000000001", writer: "writerA" },
  B: { space: "b".repeat(32), page: "10000000-0000-4000-8000-000000000002", stream: "20000000-0000-4000-8000-000000000002", link: "30000000-0000-4000-8000-000000000002", writer: "writerB" },
};
const TOKEN_A = "A".repeat(43);
const TOKEN_B = "B".repeat(43);
const HASH = "h".repeat(43);
const sp = (t) => `${t.space}_${t.page}`;
const logId = (t, epoch, stream, seq) => `${sp(t)}_${epoch}_${stream}_${seq}`;
const entry = (t, seq, extra = {}) => ({
  space: t.space, page: t.page, epoch: 1, stream: t.stream, seq, uid: t.writer, hash: HASH, envelope: "ZW52ZWxvcGU", expiresAt: sooner, ...extra,
});
const chunk = (t, index, extra = {}) => ({
  space: t.space, page: t.page, epoch: 1, object: "c".repeat(64), index, count: 513, uid: t.writer, bytes: "Ynl0ZXM", expiresAt: sooner, ...extra,
});
const chunkId = (t, index) => `${sp(t)}_1_${"c".repeat(64)}_${index}`;

/** Two independent tenants under one deployment: each has an owner, two commenters (writer, peer) and a viewer. */
async function seedWorld() {
  await clear();
  for (const [name, t] of Object.entries(T)) {
    await seed(`spaces/${t.space}`, { space: t.space, owner: `owner${name}` });
    await seed(`pages/${sp(t)}`, { space: t.space, page: t.page, epoch: 1, state: "open", expiresAt: future });
    for (const [uid, role] of [[`owner${name}`, "editor"], [`writer${name}`, "commenter"], [`peer${name}`, "commenter"], [`viewer${name}`, "viewer"]]) {
      await seed(`members/${sp(t)}_${uid}`, { space: t.space, page: t.page, uid, role });
    }
    await seed(`links/${sp(t)}_${t.link}`, { space: t.space, page: t.page, link: t.link, role: "viewer", token: name === "A" ? TOKEN_A : TOKEN_B });
    await seed(`log/${logId(t, 1, t.stream, 1)}`, entry(t, 1));
    await seed(`checkpoints/${chunkId(t, 0)}`, chunk(t, 0));
  }
}
const boot = async () => {
  await loadRules(composed);
  await seedWorld();
};

test("every path outside the declared collections and the wrapper is denied", async () => {
  await boot();
  for (const uid of [null, "ownerA", "writerA"]) {
    const headers = uid ? as(uid) : NOBODY;
    for (const path of ["unknown/x", `unknown/${sp(T.A)}`]) {
      assert.equal(await get(path, headers), 403, `${uid} ${path}`);
    }
    assert.equal((await fetch(`${documents}/secret/s`, { headers })).status, 403);
    assert.equal((await fetch(`${documents}/x/other/log/1`, { headers })).status, 403);
  }
  assert.equal(await set("unknown/x", { a: 1 }, as("ownerA")), 403);
});

test("spaces: first writer owns a space ID; no update, delete, transfer or foreign read", async () => {
  await boot();
  const fresh = "c".repeat(32);
  assert.equal(await set(`spaces/${fresh}`, { space: fresh, owner: "someone" }, as("squatter")), 403, "owner must be the writer");
  assert.equal(await set(`spaces/${fresh}`, { space: "d".repeat(32), owner: "squatter" }, as("squatter")), 403, "ID must equal the space field");
  assert.equal(await set("spaces/short", { space: "short", owner: "squatter" }, as("squatter")), 403, "space grammar");
  assert.equal(await set(`spaces/${"A".repeat(32)}`, { space: "A".repeat(32), owner: "squatter" }, as("squatter")), 403, "lowercase base32 only");
  assert.equal(await set(`spaces/${fresh}`, { space: fresh, owner: "squatter", extra: 1 }, as("squatter")), 403, "exact field set");
  assert.equal(await set(`spaces/${fresh}`, { space: fresh, owner: "squatter" }, NOBODY), 403, "sign-in required");
  assert.equal(await set(`spaces/${fresh}`, { space: fresh, owner: "squatter" }, as("squatter")), 200);
  assert.equal(await get(`spaces/${fresh}`, as("squatter")), 200);
  assert.equal(await get(`spaces/${fresh}`, as("ownerA")), 403, "only the owner reads the row");
  // Taking over an existing space is an update, which no one may do.
  assert.equal(await set(`spaces/${T.A.space}`, { space: T.A.space, owner: "ownerB" }, as("ownerB")), 403);
  assert.equal(await set(`spaces/${T.A.space}`, { space: T.A.space, owner: "ownerA" }, as("ownerA")), 403, "no update even by the owner");
  assert.equal(await remove(`spaces/${T.A.space}`, as("ownerA")), 403);
});

test("pages: only the space owner admits a page; epoch never decreases; expiry and ID are bound", async () => {
  await boot();
  const fresh = "10000000-0000-4000-8000-0000000000aa";
  const page = (over = {}) => ({ space: T.A.space, page: fresh, epoch: 1, state: "open", expiresAt: future, ...over });
  const id = `${T.A.space}_${fresh}`;
  assert.equal(await set(`pages/${id}`, page(), as("ownerB")), 403, "another tenant's owner");
  assert.equal(await set(`pages/${id}`, page(), as("writerA")), 403, "a member");
  assert.equal(await set(`pages/${id}`, page(), NOBODY), 403);
  assert.equal(await set(`pages/${T.A.space}_${T.B.page}`, page({ page: fresh }), as("ownerA")), 403, "ID must be built from the fields");
  assert.equal(await set(`pages/${id}`, page({ expiresAt: past }), as("ownerA")), 403, "already expired");
  assert.equal(await set(`pages/${id}`, page({ state: "deleted" }), as("ownerA")), 403, "state");
  assert.equal(await set(`pages/${id}`, page({ epoch: 0 }), as("ownerA")), 403, "epoch starts at 1");
  assert.equal(await set(`pages/${id}`, page({ extra: 1 }), as("ownerA")), 403, "exact field set");
  assert.equal(await set(`pages/${id}`, page(), as("ownerA")), 200);
  assert.equal(await set(`pages/${id}`, page({ epoch: 2 }), as("ownerA")), 200, "advance");
  assert.equal(await set(`pages/${id}`, page({ epoch: 1 }), as("ownerA")), 403, "never decrease");
  assert.equal(await set(`pages/${id}`, page({ epoch: 2, state: "archived" }), as("ownerA")), 200);
  assert.equal(await set(`pages/${id}`, page({ epoch: 3 }), as("ownerB")), 403, "another owner cannot advance it");
  assert.equal(await get(`pages/${id}`, as("ownerA")), 200);
  assert.equal(await get(`pages/${id}`, as("writerA")), 403, "members do not read the admission row");
  assert.equal(await remove(`pages/${id}`, as("writerA")), 403);
  assert.equal(await remove(`pages/${id}`, as("ownerA")), 200);
});

test("retention: owners may choose up to 365 days; members and other tenants cannot extend", async () => {
  await boot();
  const t = T.A;
  const page = (expiry) => ({ space: t.space, page: t.page, epoch: 1, state: "open", expiresAt: expiry });
  // The timestamp is taken just before the request; transport time leaves it within the server cap.
  const cap = () => new Date(Date.now() + 365 * DAY);
  assert.equal(await set(`pages/${sp(t)}`, page(cap()), as("writerA")), 403, "only the owner extends");
  assert.equal(await set(`pages/${sp(t)}`, page(cap()), as("ownerB")), 403, "another tenant cannot extend");
  assert.equal(await set(`pages/${sp(t)}`, page(new Date(Date.now() + 366 * DAY)), as("ownerA")), 403, "one day beyond cap");
  assert.equal(await set(`pages/${sp(t)}`, page(cap()), as("ownerA")), 200, "365-day client deadline admitted");
  const fresh = "10000000-0000-4000-8000-0000000000ab";
  assert.equal(await set(`pages/${t.space}_${fresh}`, { ...page(new Date(Date.now() + 366 * DAY)), page: fresh }, as("ownerA")), 403, "create beyond cap");
  assert.equal(await set(`pages/${t.space}_${fresh}`, { ...page(cap()), page: fresh }, as("ownerA")), 200, "create at client cap");
  assert.equal(await set(`pages/${sp(t)}`, page("forever"), as("ownerA")), 403, "finite timestamp required");
  assert.equal(await set(`pages/${sp(t)}`, page(new Date(Date.now() + DAY)), as("ownerA")), 200, "owner may shorten");
});

test("retention: expiry-only refresh preserves ciphertext and every routing field", async () => {
  await boot();
  const t = T.A;
  for (const [path, data, mutations] of [
    [`log/${logId(t, 1, t.stream, 1)}`, entry(t, 1), { envelope: "changed", hash: "z".repeat(43), uid: "ownerA", epoch: 2, space: T.B.space, seq: 2, extra: 1 }],
    [`checkpoints/${chunkId(t, 0)}`, chunk(t, 0), { bytes: "changed", object: "d".repeat(64), uid: "ownerA", epoch: 2, page: T.B.page, count: 2, extra: 1 }],
  ]) {
    const refreshed = { ...data, expiresAt: future };
    assert.equal(await set(path, refreshed, as("writerA")), 403, "writer cannot refresh");
    assert.equal(await set(path, refreshed, as("ownerB")), 403, "foreign owner cannot refresh");
    assert.equal(await set(path, { ...refreshed, expiresAt: new Date(future.getTime() + DAY) }, as("ownerA")), 403, "page bound");
    assert.equal(await set(path, { ...refreshed, expiresAt: past }, as("ownerA")), 403, "expired deadline");
    for (const [field, value] of Object.entries(mutations)) {
      assert.equal(await set(path, { ...refreshed, [field]: value }, as("ownerA")), 403, `immutable ${field}`);
    }
    const missing = { ...refreshed };
    delete missing.uid;
    assert.equal(await set(path, missing, as("ownerA")), 403, "cannot remove a field");
    assert.equal(await set(path, refreshed, as("ownerA")), 200, "owner refresh admitted");
    const response = await fetch(`${documents}/${ROOT}/${path}`, { headers: ADMIN });
    assert.equal(response.status, 200);
    assert.deepEqual((await response.json()).fields, fields(refreshed), "exact stored bytes and fields");
    assert.equal(await get(path, as("viewerA")), 200, "still readable");
  }
});

test("retention: owner cleanup deletes expired entries or entries of expired/removed pages only", async () => {
  await boot();
  const t = T.A;
  const rows = [[`log/${logId(t, 1, t.stream, 1)}`, entry(t, 1)], [`checkpoints/${chunkId(t, 0)}`, chunk(t, 0)]];
  for (const [path, data] of rows) {
    assert.equal(await remove(path, as("ownerA")), 403, "live entry preserved");
    await seed(path, { ...data, expiresAt: past });
    assert.equal(await get(path, as("viewerA")), 403, "expired cloud entry unavailable");
    assert.equal(await remove(path, as("writerA")), 403, "writer cannot clean up");
    assert.equal(await remove(path, as("ownerB")), 403, "another tenant cannot clean up");
    assert.equal(await remove(path, as("ownerA")), 200, "expired entry removed");
    assert.equal(await get(path, ADMIN), 404, "physical deletion");
    await seed(path, data);
  }
  await seed(`pages/${sp(t)}`, { space: t.space, page: t.page, epoch: 1, state: "open", expiresAt: past });
  for (const [path, data] of rows) {
    assert.equal(await get(path, as("ownerA")), 403, "expired page unavailable");
    assert.equal(await remove(path, as("ownerA")), 200, "expired page cleanup");
    await seed(path, data);
  }
  assert.equal(await remove(`pages/${sp(t)}`, as("ownerA")), 200);
  for (const [path] of rows) {
    assert.equal(await remove(path, as("ownerB")), 403, "removed page retains tenant isolation");
    assert.equal(await remove(path, as("ownerA")), 200, "removed page cleanup");
    assert.equal(await get(path, ADMIN), 404);
  }
  assert.equal(await get(`log/${logId(T.B, 1, T.B.stream, 1)}`, as("viewerB")), 200, "other tenant retained");
});

test("links and members: an owner enrolls members; a link holder enrolls only by the link secret and role", async () => {
  await boot();
  const t = T.A;
  const holder = as("holder1");
  const row = (uid, role, extra = {}) => ({ space: t.space, page: t.page, uid, role, ...extra });
  const enrol = (over = {}) => row("holder1", "viewer", { link: t.link, token: TOKEN_A, ...over });
  const id = (uid) => `members/${sp(t)}_${uid}`;

  // The secret never leaves the owner: holders cannot read or write links.
  assert.equal(await get(`links/${sp(t)}_${t.link}`, holder), 403);
  assert.equal(await get(`links/${sp(t)}_${t.link}`, as("writerA")), 403);
  assert.equal(await get(`links/${sp(t)}_${t.link}`, as("ownerA")), 200);
  const newLink = "30000000-0000-4000-8000-0000000000bb";
  const link = (over = {}) => ({ space: t.space, page: t.page, link: newLink, role: "editor", token: "C".repeat(43), ...over });
  assert.equal(await set(`links/${sp(t)}_${newLink}`, link(), holder), 403, "a holder cannot mint a link");
  assert.equal(await set(`links/${sp(t)}_${newLink}`, link(), as("ownerB")), 403, "another tenant's owner");
  assert.equal(await set(`links/${sp(t)}_${newLink}`, link({ role: "bridge" }), as("ownerA")), 403, "a link grants only viewer, commenter or editor");
  assert.equal(await set(`links/${sp(t)}_${newLink}`, link({ token: "short" }), as("ownerA")), 403, "token grammar");
  assert.equal(await set(`links/${sp(t)}_${newLink}`, link({ page: T.B.page }), as("ownerA")), 403, "ID must be built from the fields");
  assert.equal(await set(`links/${sp(t)}_${newLink}`, link(), as("ownerA")), 200);
  assert.equal(await set(`links/${sp(t)}_${newLink}`, link({ token: "D".repeat(43) }), as("ownerA")), 403, "links are immutable");

  // Enrollment.
  assert.equal(await set(id("holder1"), enrol(), NOBODY), 403, "sign-in required");
  assert.equal(await set(id("holder1"), enrol({ token: TOKEN_B }), holder), 403, "another space's secret");
  assert.equal(await set(id("holder1"), enrol({ token: "E".repeat(43) }), holder), 403, "wrong secret");
  assert.equal(await set(id("holder1"), enrol({ role: "editor" }), holder), 403, "a holder cannot pick a stronger role than the link");
  assert.equal(await set(id("holder1"), row("holder1", "viewer"), holder), 403, "no link, no row");
  assert.equal(await set(id("holder1"), enrol({ uid: "holder2" }), holder), 403, "only for itself");
  assert.equal(await set(id("holder2"), enrol(), holder), 403, "the ID must name the writer");
  assert.equal(await set(id("holder1"), enrol({ link: T.B.link }), holder), 403, "a link of another page");
  assert.equal(await set(id("holder1"), enrol({ extra: 1 }), holder), 403, "exact field set");
  assert.equal(await set(id("holder1"), enrol(), holder), 200);
  assert.equal(await get(id("holder1"), holder), 200, "its own row");
  assert.equal(await get(id("holder1"), as("holder2")), 403, "not another holder's");
  assert.equal(await get(id("holder1"), as("ownerA")), 200);
  assert.equal(await set(id("holder1"), enrol({ role: "viewer" }), holder), 403, "no self-update");
  assert.equal(await set(id("holder1"), row("holder1", "editor"), holder), 403, "no self-promotion");
  assert.equal(await remove(id("holder1"), holder), 403, "removal is the owner's");

  // Owner management of rows.
  assert.equal(await set(id("named"), row("named", "editor"), as("ownerA")), 200);
  assert.equal(await set(id("named"), row("named", "viewer"), as("ownerA")), 200, "role change");
  assert.equal(await set(id("bot"), row("bot", "bridge"), as("ownerA")), 200);
  assert.equal(await set(id("root"), row("root", "owner"), as("ownerA")), 403, "no owner role: ownership is the space row");
  assert.equal(await set(id("x1"), row("x1", "editor"), as("ownerB")), 403, "another tenant's owner");
  assert.equal(await set(id("x1"), { ...row("x1", "editor"), space: T.B.space }, as("ownerB")), 403, "ID/fields mismatch and no such page row");
  assert.equal(await set(`members/${t.space}_${"10000000-0000-4000-8000-0000000000cc"}_x1`, { space: t.space, page: "10000000-0000-4000-8000-0000000000cc", uid: "x1", role: "editor" }, as("ownerA")), 403, "no page row, no member");
  assert.equal((await list("members", { space: t.space, page: t.page }, as("ownerA"))).status, 200, "the owner lists its page's rows");
  assert.equal((await list("members", { space: t.space, page: t.page }, as("writerA"))).status, 403, "members do not");
  assert.equal(await remove(id("named"), as("writerA")), 403);
  assert.equal(await remove(id("named"), as("ownerB")), 403);
  assert.equal(await remove(id("named"), as("ownerA")), 200);
});

test("log append: create-only, contiguous per stream, current epoch, writer role, bounded expiry", async () => {
  await boot();
  const t = T.A;
  const writer = as("writerA");
  const at = (seq, over, epoch = 1, stream = t.stream) => set(`log/${logId(t, epoch, stream, seq)}`, entry(t, seq, { epoch, stream, ...over }), writer);

  assert.equal(await at(3), 403, "a gap is refused");
  assert.equal(await at(2), 200, "the next sequence");
  assert.equal(await at(2), 403, "a retry is an update and cannot overwrite");
  assert.equal(await at(3), 200);
  assert.equal(await at(5), 403, "still a gap");
  assert.equal(await at(1, undefined, 1, "20000000-0000-4000-8000-0000000000dd"), 200, "a stream starts at 1");
  assert.equal(await at(2, undefined, 1, "20000000-0000-4000-8000-0000000000dd"), 200, "streams advance independently");
  assert.equal(await at(4, { seq: 3 }), 403, "ID must equal the fields it is built from");
  assert.equal(await at(4, { epoch: 2 }), 403, "ID must equal its epoch");
  assert.equal(await at(4, { stream: "20000000-0000-4000-8000-0000000000ee" }), 403, "ID must equal its stream");
  assert.equal(await at(0), 403, "sequence zero is never a log entry");
  assert.equal(await at(9007199254740992, undefined), 403, "beyond the safe-integer bound");
  assert.equal(await at(4, { extra: "x" }), 403, "exact field set");
  assert.equal(await at(4, { hash: "short" }), 403, "hash grammar");
  assert.equal(await at(4, { envelope: "x".repeat(354_305) }), 403, "over the envelope cap");
  assert.equal(await at(4, { expiresAt: past }), 403, "already expired");
  assert.equal(await at(4, { expiresAt: new Date(future.getTime() + DAY) }), 403, "beyond the page's retention");
  assert.equal(await at(4, { expiresAt: future }), 200, "up to the page's retention");
  assert.equal(await at(1, undefined, 2, "20000000-0000-4000-8000-0000000000ab"), 403, "an epoch the page has not reached");

  // Who may start a stream: seq 1 of a stream nobody has used, by an admitted writer for itself.
  const own = "20000000-0000-4000-8000-0000000000a1";
  const first = (uid, headers, over) => set(`log/${logId(t, 1, own, 1)}`, entry(t, 1, { stream: own, uid, ...over }), headers);
  assert.equal(await first("viewerA", as("viewerA")), 403, "viewers read only");
  assert.equal(await first("outsider", as("outsider")), 403, "no member row");
  assert.equal(await first("writerA", NOBODY), 403, "sign-in required");
  assert.equal(await first("writerB", as("writerB")), 403, "another tenant's writer");
  assert.equal(await first("writerA", as("peerA")), 403, "the uid field must be the writer's own");
  assert.equal(await first("ownerA", as("ownerA")), 200, "the owner's own row writes");

  // A stream belongs to the uid that started it: an admitted peer cannot take its next slot.
  const peer = as("peerA");
  assert.equal(await set(`log/${logId(t, 1, t.stream, 5)}`, entry(t, 5, { uid: "peerA" }), peer), 403, "a peer cannot take the next slot of another writer's stream");
  assert.equal(await set(`log/${logId(t, 1, t.stream, 5)}`, entry(t, 5), peer), 403, "nor claim the writer's uid");
  assert.equal(await at(5), 200, "the stream's own writer is not blocked");
  assert.equal(await set(`log/${logId(t, 1, t.stream, 6)}`, entry(t, 6, { uid: "peerA" }), peer), 403, "still refused after the writer advanced");
  assert.equal(await set(`log/${logId(t, 1, "20000000-0000-4000-8000-0000000000a3", 1)}`, entry(t, 1, { uid: "peerA", stream: "20000000-0000-4000-8000-0000000000a2" }), peer), 403, "ID must equal its stream");
  assert.equal(await set(`log/${logId(t, 1, "20000000-0000-4000-8000-0000000000a2", 1)}`, entry(t, 1, { uid: "peerA", stream: "20000000-0000-4000-8000-0000000000a2" }), peer), 200, "a peer starts its own stream");
  assert.equal(await set(`log/${logId(t, 1, "20000000-0000-4000-8000-0000000000a2", 2)}`, entry(t, 2, { uid: "writerA", stream: "20000000-0000-4000-8000-0000000000a2" }), writer), 403, "and the writer cannot take the peer's next slot");
  assert.equal(await set(`log/${logId(t, 1, "20000000-0000-4000-8000-0000000000a2", 2)}`, entry(t, 2, { uid: "peerA", stream: "20000000-0000-4000-8000-0000000000a2" }), peer), 200, "while the peer continues its own stream");
  assert.equal(await set(`log/${logId(t, 1, t.stream, 4)}`, entry(t, 4, { hash: "g".repeat(43) }), writer), 403, "no update of an existing entry");
  assert.equal(await remove(`log/${logId(t, 1, t.stream, 4)}`, writer), 403, "no delete");
  assert.equal(await remove(`log/${logId(t, 1, t.stream, 4)}`, as("ownerA")), 403, "no delete even by the owner (retention is #2454)");

  // A batched append is judged on its final state: 1..n in one commit is contiguous.
  const batchStream = "20000000-0000-4000-8000-0000000000ff";
  const batch = (...seqs) => seqs.map((seq) => [`log/${logId(t, 1, batchStream, seq)}`, entry(t, seq, { stream: batchStream })]);
  assert.equal(await commit(batch(2, 3), writer), 403, "a batch with a gap before it");
  assert.equal(await commit(batch(1, 2, 3), writer), 200, "a contiguous batch");
});

test("page lifecycle: archive freezes writes, epoch advance fences the old epoch, deletion and revoke deny at once", async () => {
  await boot();
  const t = T.A;
  const writer = as("writerA");
  const page = (over) => ({ space: t.space, page: t.page, epoch: 1, state: "open", expiresAt: future, ...over });
  const next = (seq, epoch = 1) => set(`log/${logId(t, epoch, t.stream, seq)}`, entry(t, seq, { epoch }), writer);

  assert.equal(await next(2), 200);
  assert.equal(await set(`pages/${sp(t)}`, page({ state: "archived" }), as("ownerA")), 200);
  assert.equal(await next(3), 403, "archived: no writes");
  assert.equal(await get(`log/${logId(t, 1, t.stream, 2)}`, writer), 200, "archived: reads continue");
  assert.equal(await set(`checkpoints/${chunkId(t, 1)}`, chunk(t, 1), writer), 403, "archived: no chunk writes");
  assert.equal(await set(`pages/${sp(t)}`, page({ state: "open", epoch: 2 }), as("ownerA")), 200);
  assert.equal(await next(3, 1), 403, "the old epoch is fenced");
  assert.equal(await next(1, 2), 200, "the new epoch starts at 1");
  assert.equal(await get(`log/${logId(t, 1, t.stream, 2)}`, writer), 200, "old-epoch entries stay readable by members");

  // Revocation takes effect on the next request.
  assert.equal(await remove(`members/${sp(t)}_writerA`, as("ownerA")), 200);
  assert.equal(await get(`log/${logId(t, 1, t.stream, 2)}`, writer), 403, "a revoked member cannot read");
  assert.equal(await next(2, 2), 403, "a revoked member cannot write");
  assert.equal(await get(`log/${logId(t, 1, t.stream, 2)}`, as("viewerA")), 200, "others are unaffected");

  // Deleting the admission row denies everyone, and writes too.
  assert.equal(await remove(`pages/${sp(t)}`, as("ownerA")), 200);
  assert.equal(await get(`log/${logId(t, 1, t.stream, 2)}`, as("viewerA")), 403, "deleted page: no reads");
  const fresh = "20000000-0000-4000-8000-0000000000b1";
  assert.equal(await set(`log/${logId(t, 2, fresh, 1)}`, entry(t, 1, { epoch: 2, stream: fresh, uid: "ownerA" }), as("ownerA")), 403, "deleted page: no writes");
  assert.equal(await get(`log/${logId(T.B, 1, T.B.stream, 1)}`, as("viewerB")), 200, "the other tenant is unaffected");
});

test("reads: members only, one space and page per query; an expired entry is unreadable by get and an expired page by every read", async () => {
  await boot();
  const t = T.A;
  await seed(`log/${logId(t, 1, t.stream, 2)}`, entry(t, 2, { expiresAt: past }));
  const key = { space: t.space, page: t.page };

  for (const uid of ["viewerA", "writerA", "ownerA"]) {
    assert.equal(await get(`log/${logId(t, 1, t.stream, 1)}`, as(uid)), 200, `${uid} get`);
    assert.deepEqual(await list("log", key, as(uid)), { status: 200, found: 2 }, `${uid} list: queries cannot filter an entry's own expiry; clients drop it`);
    assert.deepEqual(await list("checkpoints", key, as(uid)), { status: 200, found: 1 }, `${uid} chunks`);
  }
  assert.equal(await get(`log/${logId(t, 1, t.stream, 2)}`, as("writerA")), 403, "an expired entry is unreadable");
  assert.equal(await get(`log/${logId(t, 1, t.stream, 1)}`, as("outsider")), 403, "no member row: anonymous uid without enrollment");
  assert.equal(await get(`log/${logId(t, 1, t.stream, 1)}`, NOBODY), 403);
  assert.equal((await list("log", key, as("outsider"))).status, 403);
  assert.equal((await list("log", key, NOBODY)).status, 403);
  assert.equal((await list("log", { space: t.space }, as("writerA"))).status, 403, "a query without the page filter");
  assert.equal((await list("log", { page: t.page }, as("writerA"))).status, 403, "a query without the space filter");
  assert.equal((await list("log", {}, as("writerA"))).status, 403, "an unfiltered query");
  assert.equal((await list("log", { space: t.space, page: t.page, epoch: 1 }, as("writerA"))).status, 200, "narrower queries are fine");
  assert.equal((await list("log", { space: t.space, page: T.B.page }, as("writerA"))).status, 403, "a page the member was never admitted to");

  // An expired page is gone for every reader, even when its entries are not expired.
  await seed(`pages/${sp(t)}`, { space: t.space, page: t.page, epoch: 1, state: "open", expiresAt: past });
  await seed(`log/${logId(t, 1, t.stream, 3)}`, entry(t, 3, { expiresAt: future }));
  assert.equal(await get(`log/${logId(t, 1, t.stream, 3)}`, as("ownerA")), 403, "expired page");
  assert.equal((await list("log", key, as("ownerA"))).status, 403);
});

test("tenants: owner A can neither read, list nor write owner B's space, and the reverse", async () => {
  await boot();
  for (const [mine, theirs, me, ally] of [[T.A, T.B, "ownerA", "writerA"], [T.B, T.A, "ownerB", "writerB"]]) {
    const theirKey = { space: theirs.space, page: theirs.page };
    for (const uid of [me, ally]) {
      const headers = as(uid);
      assert.equal(await get(`log/${logId(theirs, 1, theirs.stream, 1)}`, headers), 403, `${uid} get`);
      assert.equal((await list("log", theirKey, headers)).status, 403, `${uid} list`);
      assert.equal((await list("checkpoints", theirKey, headers)).status, 403, `${uid} chunk list`);
      assert.equal(await get(`checkpoints/${chunkId(theirs, 0)}`, headers), 403, `${uid} chunk`);
      assert.equal(await get(`spaces/${theirs.space}`, headers), 403, `${uid} space row`);
      assert.equal(await get(`pages/${sp(theirs)}`, headers), 403, `${uid} page row`);
      assert.equal(await get(`links/${sp(theirs)}_${theirs.link}`, headers), 403, `${uid} link`);
      assert.equal(await get(`members/${sp(theirs)}_${theirs === T.A ? "ownerA" : "ownerB"}`, headers), 403, `${uid} member`);
      assert.equal(await set(`log/${logId(theirs, 1, theirs.stream, 2)}`, entry(theirs, 2), headers), 403, `${uid} append`);
      assert.equal(await set(`checkpoints/${chunkId(theirs, 1)}`, chunk(theirs, 1), headers), 403, `${uid} chunk write`);
    }
    // Crafted attempts: write into their ID space with fields or IDs that point at one's own.
    assert.equal(await set(`log/${logId(theirs, 1, theirs.stream, 2)}`, entry(mine, 2), as(ally)), 403, "their ID, my fields");
    assert.equal(await set(`log/${logId(mine, 1, theirs.stream, 2)}`, entry(theirs, 2), as(ally)), 403, "my ID, their fields");
    assert.equal(await set(`log/${logId(mine, 1, mine.stream, 2)}`, entry(mine, 2, { space: theirs.space }), as(ally)), 403, "a space field that disagrees with the ID");
    assert.equal(await set(`pages/${sp(theirs)}`, { space: theirs.space, page: theirs.page, epoch: 9, state: "open", expiresAt: future }, as(me)), 403, "page row");
    assert.equal(await set(`members/${sp(theirs)}_${me}`, { space: theirs.space, page: theirs.page, uid: me, role: "editor" }, as(me)), 403, "self-enrollment");
    assert.equal(await set(`members/${sp(theirs)}_${me}`, { space: theirs.space, page: theirs.page, uid: me, role: "viewer", link: mine.link, token: mine === T.A ? TOKEN_A : TOKEN_B }, as(me)), 403, "my link's secret does not open their page");
  }
  // Each side still works inside its own space.
  assert.equal(await set(`log/${logId(T.A, 1, T.A.stream, 2)}`, entry(T.A, 2), as("writerA")), 200);
  assert.equal(await set(`log/${logId(T.B, 1, T.B.stream, 2)}`, entry(T.B, 2), as("writerB")), 200);
  assert.deepEqual(await list("log", { space: T.A.space, page: T.A.page }, as("viewerA")), { status: 200, found: 2 });
});

test("checkpoint chunks: create-only, bounded, indexed, current epoch and writer role", async () => {
  await boot();
  const t = T.A;
  const writer = as("writerA");
  const at = (index, over, headers = writer) => set(`checkpoints/${chunkId(t, index)}`, chunk(t, index, over), headers);

  const peer = as("peerA");
  // An object belongs to the uid that wrote its chunk 0: a peer cannot add to or complete it.
  assert.equal(await at(1, { uid: "peerA" }, peer), 403, "a peer cannot extend another writer's object");
  assert.equal(await at(1, undefined, peer), 403, "nor claim the writer's uid");
  assert.equal(await at(1), 200, "the object's own writer extends it");
  assert.equal(await at(2, { uid: "peerA" }, peer), 403, "still refused after the writer extended it");
  const mine = "d".repeat(64);
  const peerChunk = (index, uid = "peerA", headers = peer) => set(`checkpoints/${sp(t)}_1_${mine}_${index}`, chunk(t, index, { object: mine, uid }), headers);
  assert.equal(await peerChunk(1), 403, "an object starts at index 0 (no predecessor to bind to)");
  assert.equal(await peerChunk(0, "writerA"), 403, "the uid field must be the writer's own, so an object cannot be started in another uid's name");
  assert.equal(await peerChunk(0), 200, "a peer starts its own object");
  assert.equal(await peerChunk(1), 200, "and extends it");
  assert.equal(await peerChunk(2, "writerA", writer), 403, "while the writer cannot add to the peer's object");
  assert.equal(await at(1), 403, "a chunk cannot be overwritten");
  assert.equal(await at(2), 200, "chunks need not arrive in order beyond the writer's own chain");
  assert.equal(await at(3, { count: 3 }), 403, "index must be below count");
  assert.equal(await at(4, { count: 514 }), 403, "over the largest object's chunk count");
  assert.equal(await at(4), 200);
  assert.equal(await at(5, { bytes: "x".repeat(43_693) }), 403, "over the chunk bound");
  assert.equal(await at(5, { bytes: "x".repeat(43_692) }), 200);
  assert.equal(await at(6, { epoch: 2 }), 403, "ID must equal its epoch");
  assert.equal(await at(6, { object: "c".repeat(63) }), 403, "object grammar");
  assert.equal(await at(6, { expiresAt: new Date(future.getTime() + DAY) }), 403, "beyond the page's retention");
  assert.equal(await at(6, { extra: 1 }), 403, "exact field set");
  assert.equal(await at(6, undefined, as("viewerA")), 403, "viewers read only");
  assert.equal(await at(6, undefined, as("outsider")), 403);
  assert.equal(await remove(`checkpoints/${chunkId(t, 1)}`, writer), 403, "no delete");
  assert.equal(await remove(`checkpoints/${chunkId(t, 1)}`, as("ownerA")), 403, "no delete even by the owner");
  assert.equal(await get(`checkpoints/${chunkId(t, 1)}`, as("viewerA")), 200);
});

test("the declaration lists exactly the collections the Rules read or write, and no blob resource", async () => {
  const declaration = JSON.parse(JSON.parse(readFileSync(new URL("deploy-declaration-v1.json", vectors), "utf8")).declaration);
  const declared = declaration.resources.map((resource) => resource.path).sort();
  const matched = [...readFileSync(new URL("../../firestore/admission.rules", vectors), "utf8").matchAll(/^match \/([a-z]+)\/\{/gm)].map((m) => m[1]).sort();
  assert.deepEqual(declared, matched);
  assert.ok(!declaration.resources.some((resource) => resource.kind === "blob"));
  const read = [...readFileSync(new URL("../../firestore/admission.rules", vectors), "utf8").matchAll(/ext\.(?:get|exists|existsAfter|getAfter)\(\/([a-z]+)\//g)].map((m) => m[1]);
  for (const collection of read) assert.ok(declared.includes(collection), `${collection} is read but not declared`);
});
