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
  value === null ? { nullValue: null }
  : value instanceof Date ? { timestampValue: value.toISOString().replace(/\.000Z$/, "Z") }
  : typeof value === "boolean" ? { booleanValue: value }
  : typeof value === "number" ? { integerValue: String(value) }
  : { stringValue: String(value) };
const fields = (data) => Object.fromEntries(Object.entries(data).map(([k, v]) => [k, typed(v)]));
const DAY = 86_400_000;
// A whole-second fixture clock exercises Firestore's omitted zero fraction on every run.
const clockMs = Math.floor(Date.now() / 1000) * 1000;
const future = new Date(clockMs + 30 * DAY);
const sooner = new Date(clockMs + 10 * DAY);
const past = new Date(clockMs - DAY);

test("timestamp fixtures omit a zero fraction and preserve nonzero millisecond precision", () => {
  for (const [input, expected] of [
    ["2030-01-02T03:04:05.000Z", "2030-01-02T03:04:05Z"],
    ["2030-01-02T03:04:05.001Z", "2030-01-02T03:04:05.001Z"],
    ["2030-01-02T03:04:05.120Z", "2030-01-02T03:04:05.120Z"],
    ["2030-01-02T03:04:05.123Z", "2030-01-02T03:04:05.123Z"],
    ["2030-01-02T03:04:05.999Z", "2030-01-02T03:04:05.999Z"],
  ]) {
    assert.deepEqual(fields({ expiresAt: new Date(input) }), { expiresAt: { timestampValue: expected } });
  }
  assert.equal(sooner.getUTCMilliseconds(), 0, "read-back regressions always exercise a zero fraction");
});

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
  space: t.space, page: t.page, epoch: 1, object: "c".repeat(64), index, seq: index + 1, count: 513, uid: t.writer, bytes: "Ynl0ZXM", expiresAt: sooner, ...extra,
});
const chunkId = (t, index) => `${sp(t)}_1_${"c".repeat(64)}_${index}`;

/** Two independent tenants under one deployment: each has an owner, two commenters (writer, peer) and a viewer. */
async function seedWorld() {
  await clear();
  for (const [name, t] of Object.entries(T)) {
    await seed(`spaces/${t.space}`, { space: t.space, owner: `owner${name}` });
    await seed(`pages/${sp(t)}`, { space: t.space, page: t.page, epoch: 1, state: "open", expiresAt: future });
    for (const [uid, role] of [[`owner${name}`, "editor"], [`writer${name}`, "commenter"], [`peer${name}`, "commenter"], [`viewer${name}`, "viewer"]]) {
      await seed(`members/${sp(t)}_${uid}`, { space: t.space, page: t.page, uid, role, link: null, token: null, seq: 1 });
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
  // Members read the row (routing and lifetime hints) with an exact get; nobody lists pages.
  assert.equal(await get(`pages/${id}`, as("writerA")), 403, "a page nobody joined has no members to read it");
  await seed(`members/${id}_writerA`, { space: T.A.space, page: fresh, uid: "writerA", role: "viewer", link: null, token: null, seq: 1 });
  assert.equal(await get(`pages/${id}`, as("writerA")), 200, "a member reads the row");
  assert.equal(await get(`pages/${id}`, as("peerA")), 403, "a non-member does not");
  assert.equal(await get(`pages/${id}`, as("writerB")), 403, "another tenant's member does not");
  assert.equal(await get(`pages/${id}`, NOBODY), 403, "sign-in required");
  assert.equal((await list("pages", { space: T.A.space }, as("writerA"))).status, 403, "a member cannot list pages");
  assert.equal((await list("pages", { space: T.A.space }, as("ownerA"))).status, 403, "nor can the owner");
  assert.equal((await list("pages", { space: T.A.space, page: fresh }, as("writerA"))).status, 403, "even narrowed to the page");
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

test("retention: a page refresh is one write and entries have an independent lifetime", async () => {
  await boot();
  const t = T.A;
  const deadline = new Date(Date.now() + 365 * DAY);
  const log = entry(t, 2, { expiresAt: deadline });
  const checkpoint = chunk(t, 1, { expiresAt: deadline });
  const rows = [[`log/${logId(t, 1, t.stream, 2)}`, log], [`checkpoints/${chunkId(t, 1)}`, checkpoint]];
  for (const [path, data] of rows) {
    assert.equal(await set(path, data, as("writerA")), 200, "entry may outlive page within its own cap");
  }
  const before = await Promise.all(rows.map(async ([path]) => (await (await fetch(`${documents}/${ROOT}/${path}`, { headers: ADMIN })).json()).fields));
  assert.equal(await set(`pages/${sp(t)}`, { space: t.space, page: t.page, epoch: 1, state: "open", expiresAt: deadline }, as("ownerA")), 200, "page-only activity refresh");
  for (const [index, [path]] of rows.entries()) {
    assert.equal(await get(path, as("viewerA")), 200, "old entry remains readable");
    assert.deepEqual((await (await fetch(`${documents}/${ROOT}/${path}`, { headers: ADMIN })).json()).fields, before[index], "entry was not rewritten");
  }
  await seed(`pages/${sp(t)}`, { space: t.space, page: t.page, epoch: 1, state: "open", expiresAt: past });
  for (const [path] of rows) {
    assert.equal(await get(path, as("viewerA")), 403, "entry outliving expired page is unreadable");
    assert.equal(await get(path, ADMIN), 200, "ciphertext still physically present");
  }
});

test("retention: expiry-only refresh preserves ciphertext and every routing field", async () => {
  await boot();
  const t = T.A;
  for (const [path, data, mutations] of [
    [`log/${logId(t, 1, t.stream, 1)}`, entry(t, 1), { envelope: "changed", hash: "z".repeat(43), uid: "ownerA", epoch: 2, space: T.B.space, seq: 2, extra: 1 }],
    [`checkpoints/${chunkId(t, 0)}`, chunk(t, 0), { bytes: "changed", object: "d".repeat(64), uid: "ownerA", epoch: 2, page: T.B.page, count: 2, extra: 1 }],
  ]) {
    const refreshed = { ...data, expiresAt: new Date(Date.now() + 365 * DAY) };
    assert.equal(await set(path, refreshed, as("writerA")), 403, "writer cannot refresh");
    assert.equal(await set(path, refreshed, as("ownerB")), 403, "foreign owner cannot refresh");
    assert.equal(await set(path, { ...refreshed, expiresAt: new Date(Date.now() + 366 * DAY) }, as("ownerA")), 403, "entry ceiling");
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
  const row = (uid, role, extra = {}) => ({ space: t.space, page: t.page, uid, role, link: null, token: null, seq: 1, ...extra });
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
  assert.equal(await at(4, { expiresAt: new Date(Date.now() + 366 * DAY) }), 403, "beyond the entry ceiling");
  assert.equal(await at(4, { expiresAt: future }), 200, "within the entry ceiling");
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

/** The link's `hosted` device ID the shared authority vector freezes: UUIDv4-shaped like every stream the Rules admit. */
const HOSTED = JSON.parse(readFileSync(new URL("authority-v1.json", vectors), "utf8")).linkHostedDevice.id;
const OTHER_HOSTED = "20000000-0000-4000-8000-0000000000c2";
const holderRow = (t, uid, link, token, role = "editor") => ({ space: t.space, page: t.page, uid, role, link, token, seq: 1 });
/** The owner mints an editor (or other role) link; each uid then enrolls by its secret with a row of its own. */
async function shareLink(t, linkId, token, uids, role = "editor") {
  assert.equal(await set(`links/${sp(t)}_${linkId}`, { space: t.space, page: t.page, link: linkId, role, token }, as(`owner${t === T.A ? "A" : "B"}`)), 200);
  for (const uid of uids)
    assert.equal(await set(`members/${sp(t)}_${uid}`, holderRow(t, uid, linkId, token, role), as(uid)), 200, `${uid} enrolls`);
}
const hostedAt = (t, stream = HOSTED) => (seq, uid, caller = uid) =>
  set(`log/${logId(t, 1, stream, seq)}`, entry(t, seq, { stream, uid }), as(caller));
const LINK_1 = "30000000-0000-4000-8000-0000000000c1", TOKEN_1 = "F".repeat(43);
const LINK_2 = "30000000-0000-4000-8000-0000000000c2", TOKEN_2 = "G".repeat(43);

test("log append: holders of one editor link share its hosted stream, each with a uid of its own", async () => {
  await boot();
  const t = T.A;
  assert.match(HOSTED, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  await shareLink(t, LINK_1, TOKEN_1, ["holder1", "holder2"]);
  const at = hostedAt(t);

  assert.equal(await at(1, "holder1"), 200, "the first holder starts the hosted stream");
  assert.equal(await at(2, "holder1"), 200, "and continues it");
  assert.equal(await at(3, "holder2"), 200, "a second uid of the same link continues it");
  assert.equal(await at(4, "holder1"), 200, "and the first continues the second's entry");
  assert.equal(await at(5, "holder2"), 200);
  // A race still has one winner: a create on an existing ID is an update, which only the owner may do.
  assert.equal(await at(5, "holder1"), 403, "the loser of a race is refused");
  assert.equal(await at(5, "holder2"), 403, "and so is a second create of the same ID by its writer");
  // The SDK creates with an "absent" precondition, and Firestore evaluates the Rules for that write as a create.
  // A link-mate passes them, so the loser of a race with one sees the precondition fail (409 ALREADY_EXISTS,
  // which the SDK reads back), not a Rules refusal; a writer the Rules refuse sees 403 (see the next case).
  const raced = (uid) => commit([[`log/${logId(t, 1, HOSTED, 5)}`, entry(t, 5, { stream: HOSTED, uid })]], as(uid));
  assert.equal(await raced("holder1"), 409, "the loser of a race with a link-mate: the ID is taken");
  assert.equal(await raced("holder2"), 409, "and a second create of the same ID by its writer");
  assert.equal(await at(7, "holder1"), 403, "a gap is refused");
  assert.equal(await at(6, "holder1"), 200, "the loser continues at the new head");
  assert.equal(await get(`log/${logId(t, 1, HOSTED, 3)}`, as("holder1")), 200, "each holder reads the other's entry");
  assert.equal(await get(`log/${logId(t, 1, HOSTED, 2)}`, as("holder2")), 200);
});

test("log append: only link-mates continue a link's stream; every other writer stays refused", async () => {
  await boot();
  const t = T.A;
  await shareLink(t, LINK_1, TOKEN_1, ["holder1", "holder2"]);
  await shareLink(t, LINK_2, TOKEN_2, ["holder3"]);
  const viewerLink = "30000000-0000-4000-8000-0000000000c3";
  await shareLink(t, viewerLink, "H".repeat(43), ["reader1"], "viewer");
  const at = hostedAt(t);
  assert.equal(await at(1, "holder1"), 200);
  assert.equal(await at(2, "holder1"), 200);

  assert.equal(await at(3, "holder3"), 403, "a holder of another link of the same page");
  // The same writers through the SDK's create (absent precondition) on an ID that exists: Rules first.
  const taken = (uid) => commit([[`log/${logId(t, 1, HOSTED, 2)}`, entry(t, 2, { stream: HOSTED, uid })]], as(uid));
  assert.equal(await taken("holder3"), 403, "a holder of another link is a Rules refusal, not a taken ID");
  assert.equal(await taken("writerA"), 403, "so is a member with no link");
  assert.equal(await taken("holder2"), 409, "while a link-mate passes the Rules and finds the ID taken");
  assert.equal(await at(3, "writerA"), 403, "an owner-enrolled member has no link, so it is no link-mate");
  assert.equal(await at(3, "peerA"), 403);
  assert.equal(await at(3, "viewerA"), 403, "a viewer never writes");
  assert.equal(await at(3, "reader1"), 403, "nor does the holder of a viewer link");
  assert.equal(await at(3, "stranger"), 403, "no member row, no entry");
  assert.equal(await at(3, "holder2", "holder1"), 403, "the entry's uid must be the caller's");
  assert.equal(await at(3, "holder1", "holder2"), 403, "not another holder's uid either");
  assert.equal(await at(3, "holder2", "holder3"), 403, "and a link-mate's uid does not carry another link's caller");

  // The link on a row comes only from the link's own secret, which a holder cannot forge or edit.
  const forged = (uid, link, token) => set(`members/${sp(t)}_${uid}`, holderRow(t, uid, link, token), as(uid));
  assert.equal(await forged("holder4", LINK_1, TOKEN_2), 403, "link 1 with link 2's secret");
  assert.equal(await at(3, "holder4"), 403, "so the forger never became a member");
  assert.equal(await forged("holder3", LINK_1, TOKEN_1), 403, "a holder cannot rewrite its own row to claim another link");
  assert.equal(await at(3, "holder3"), 403);
  // Another tenant's holder is no member of this page at all.
  assert.equal(await set(`members/${sp(T.B)}_holderB`, holderRow(T.B, "holderB", T.B.link, TOKEN_B, "viewer"), as("holderB")), 200);
  assert.equal(await at(3, "holderB"), 403, "another tenant's holder");

  assert.equal(await at(3, "holder2"), 200, "after every refusal the link-mate still continues");
  // The owner may write any stream; its row names no link, so a holder cannot continue the owner's entry.
  assert.equal(await at(4, "ownerA"), 200, "the owner may continue any stream");
  assert.equal(await at(5, "holder1"), 403, "an entry the owner wrote makes no one a link-mate");
});

test("log append: ending a link stops every holder of it, in the order the owner retires it", async () => {
  await boot();
  const t = T.A, owner = as("ownerA");
  await shareLink(t, LINK_1, TOKEN_1, ["holder1", "holder2"]);
  await shareLink(t, LINK_2, TOKEN_2, ["holder3"]);
  const at = hostedAt(t), other = hostedAt(t, OTHER_HOSTED);
  assert.equal(await at(1, "holder1"), 200);
  assert.equal(await at(2, "holder2"), 200);
  assert.equal(await other(1, "holder3"), 200);

  // The owner removes the link row first, so nobody enrolls again, then the rows it enrolled.
  assert.equal(await remove(`links/${sp(t)}_${LINK_1}`, owner), 200);
  assert.equal(await set(`members/${sp(t)}_late`, holderRow(t, "late", LINK_1, TOKEN_1), as("late")), 403, "an ended link enrolls no one");
  const found = await list("members", { space: t.space, page: t.page, link: LINK_1 }, owner);
  assert.deepEqual(found, { status: 200, found: 2 }, "the owner discovers both holders by their link");
  for (const uid of ["holder1", "holder2"]) assert.equal(await remove(`members/${sp(t)}_${uid}`, owner), 200);

  assert.equal(await at(3, "holder1"), 403, "the first holder can no longer continue");
  assert.equal(await at(3, "holder2"), 403, "nor the second");
  assert.equal(await get(`log/${logId(t, 1, HOSTED, 2)}`, as("holder1")), 403, "nor read");
  assert.equal(await get(`log/${logId(t, 1, HOSTED, 2)}`, as("holder2")), 403);
  assert.equal(await other(2, "holder3"), 200, "a holder of another link is unaffected");
  assert.equal(await at(3, "ownerA"), 200, "and the owner still writes");
});

test("log owner bootstrap: only the owner skips a pruned predecessor; other guards remain", async () => {
  await boot();
  const t = T.A;
  const stream = "20000000-0000-4000-8000-0000000000b1";
  const at = (seq, uid, over = {}, headers = as(uid)) =>
    set(`log/${logId(t, over.epoch ?? 1, stream, seq)}`, entry(t, seq, { stream, uid, ...over }), headers);

  // No historical payload is invented: neither seq 1 nor the predecessor 3 exists.
  assert.equal(await get(`log/${logId(t, 1, stream, 3)}`, ADMIN), 404);
  assert.equal(await at(4, "writerA"), 403, "a member cannot bootstrap the missing prefix");
  assert.equal(await at(4, "ownerB"), 403, "another space's owner cannot bootstrap here");
  assert.equal(await at(4, "ownerA", { uid: "writerA" }), 403, "the owner must bind its own uid");
  assert.equal(await at(4, "ownerA", { epoch: 2 }), 403, "the current epoch still binds");
  assert.equal(await at(4, "ownerA", { hash: "short" }), 403, "hash grammar still binds");
  assert.equal(await at(4, "ownerA", { expiresAt: past }), 403, "expiry still binds");
  assert.equal(await at(4, "ownerA", { extra: true }), 403, "the field set still binds");
  assert.equal(await at(4, "ownerA"), 200, "the owner publishes the original retained sequence");
  // PATCH also exercises the pre-existing owner expiry-update rule: an empty diff is legal.
  assert.equal(await at(4, "ownerA"), 200, "an unchanged owner PATCH is a no-op");
  assert.equal(await at(4, "ownerA", { envelope: "Y2hhbmdlZA" }), 403, "an owner cannot replace retained ciphertext");
  assert.equal(await at(5, "writerA"), 403, "a member cannot take the owner's next slot");
  assert.equal(await at(5, "ownerA"), 200, "the owner continues its own bootstrap");

  const member = "20000000-0000-4000-8000-0000000000b2";
  const next = (seq, uid) => set(`log/${logId(t, 1, member, seq)}`, entry(t, seq, { stream: member, uid }), as(uid));
  assert.equal(await next(2, "writerA"), 403, "member gaps remain refused");
  assert.equal(await next(1, "writerA"), 200, "a member starts at one");
  assert.equal(await next(2, "peerA"), 403, "another uid cannot continue the member stream");
  assert.equal(await next(2, "writerA"), 200, "matching member predecessor remains allowed");

  const ownerMember = `members/${sp(t)}_ownerA`;
  const ownerRow = (role) => ({ space: t.space, page: t.page, uid: "ownerA", role, link: null, token: null, seq: 1 });
  await seed(ownerMember, ownerRow("viewer"));
  assert.equal(await at(7, "ownerA"), 403, "the owner's member role still gates writes");
  await seed(ownerMember, ownerRow("editor"));
  assert.equal(await at(7, "ownerA"), 200, "the admitted owner can bootstrap again");
  await seed(`pages/${sp(t)}`, { space: t.space, page: t.page, epoch: 1, state: "archived", expiresAt: future });
  assert.equal(await at(9, "ownerA"), 403, "even the owner cannot bootstrap an archived page");
});

test("log owner bootstrap: the owner-only exception is guard-sensitive", async () => {
  // Mutate only the added clause in copies of the composed golden, not the fragment.
  const guard = " || get(/databases/$(database)/documents/x/colab/spaces/$(request.resource.data.space)).data.owner == request.auth.uid";
  assert.equal(composed.split(guard).length, 2, "exactly one bootstrap exception");
  const t = T.A, stream = "20000000-0000-4000-8000-0000000000b3";
  const write = (uid) => set(`log/${logId(t, 1, stream, 4)}`, entry(t, 4, { stream, uid }), as(uid));
  await loadRules(composed.replace(guard, ""));
  await seedWorld();
  assert.equal(await write("ownerA"), 403, "without the exception the owner gap positive is lost");
  await loadRules(composed.replace(guard, " || true"));
  await seedWorld();
  assert.equal(await write("writerA"), 200, "without owner binding the member gap negative is lost");
  await boot();
  assert.equal(await write("writerA"), 403, "canonical owner binding refuses the member gap");
  assert.equal(await write("ownerA"), 200, "canonical exception admits owner bootstrap");
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
    assert.equal(await set(`members/${sp(theirs)}_${me}`, { space: theirs.space, page: theirs.page, uid: me, role: "viewer", link: mine.link, token: mine === T.A ? TOKEN_A : TOKEN_B, seq: 1 }, as(me)), 403, "my link's secret does not open their page");
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
  assert.equal(await at(6, { expiresAt: new Date(Date.now() + 366 * DAY) }), 403, "beyond the entry ceiling");
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

// ASCII payloads keep the retained per-document character limits equal to byte limits.
test("payload admission refuses multibyte ciphertext", async () => {
  await boot();
  const t = T.A, writer = as(t.writer);
  const log = `log/${logId(t, 1, t.stream, 2)}`;
  const checkpoint = `checkpoints/${chunkId(t, 1)}`;
  assert.equal(await set(log, entry(t, 2, { envelope: "é" }), writer), 403);
  assert.equal(await set(log, entry(t, 2), writer), 200);
  assert.equal(await set(checkpoint, chunk(t, 1, { bytes: "é" }), writer), 403);
  assert.equal(await set(checkpoint, chunk(t, 1), writer), 200);
});

test("members: one seven-field shape distinguishes owner nulls from link enrollment", async () => {
  await boot();
  const t = T.A, path = `members/${sp(t)}_newMember`;
  const row = { space: t.space, page: t.page, uid: "newMember", role: "viewer", link: null, token: null, seq: 1 };
  const legacy = { ...row };
  delete legacy.link;
  delete legacy.token;
  assert.equal(await set(path, legacy, as("ownerA")), 403, "no four-field shape");
  const unsequenced = { ...row };
  delete unsequenced.seq;
  assert.equal(await set(path, unsequenced, as("ownerA")), 403, "seq is required");
  assert.equal(await set(path, { ...row, seq: 2 }, as("ownerA")), 403, "seq is pinned to 1 for the owner");
  assert.equal(await set(path, { ...row, seq: "1" }, as("ownerA")), 403, "seq is an integer");
  assert.equal(await set(path, { ...row, token: TOKEN_A }, as("ownerA")), 403, "owner requires the null pair");
  assert.equal(await set(path, row, as("ownerA")), 200);
  const enrolled = { ...row, uid: "newHolder", link: t.link, token: TOKEN_A };
  const enrolPath = `members/${sp(t)}_newHolder`;
  assert.equal(await set(enrolPath, { ...enrolled, token: null }, as("newHolder")), 403, "null is not an enrollment secret");
  assert.equal(await set(enrolPath, { ...enrolled, link: null }, as("newHolder")), 403, "non-null link required");
  assert.equal(await set(enrolPath, { ...enrolled, seq: 2 }, as("newHolder")), 403, "seq is pinned to 1 for a holder");
  assert.equal(await set(enrolPath, { ...unsequenced, uid: "newHolder", link: t.link, token: TOKEN_A }, as("newHolder")), 403, "a holder's seq is required");
  assert.equal(await set(enrolPath, enrolled, as("newHolder")), 200);
  assert.equal(await get(`log/${logId(t, 1, t.stream, 1)}`, as("newHolder")), 200, "viewer reads");
  assert.equal(await set(`log/${logId(t, 1, t.stream, 2)}`, entry(t, 2, { uid: "newHolder" }), as("newHolder")), 403, "enrollment never grants viewer writes");
});

test("checkpoints: positive chunk sequence is exactly zero-based index plus one", async () => {
  await boot();
  const t = T.A, path = `checkpoints/${chunkId(t, 1)}`;
  for (const seq of [0, 1, 3, "2"]) {
    assert.equal(await set(path, chunk(t, 1, { seq }), as(t.writer)), 403, `wrong seq ${seq}`);
  }
  const missing = chunk(t, 1);
  delete missing.seq;
  assert.equal(await set(path, missing, as(t.writer)), 403, "seq is required");
  assert.equal(await set(path, chunk(t, 1), as(t.writer)), 200);
  assert.equal(await get(path, as("viewerA")), 200);
});

// These are opaque signed-record transport stand-ins. Rules do not verify signatures;
// the existing Colab verifier remains responsible for complete bytes and authority.
const authority = (t, kind, extra = {}) => ({
  space: t.space, page: t.page, epoch: 1, kind, subject: "a".repeat(64),
  seq: 1, count: 1, uid: t === T.A ? "ownerA" : "ownerB", recipient: kind === "wrap" ? t.writer : "",
  bytes: "c2lnbmVk", expiresAt: sooner, ...extra,
});
const authorityId = (row) => `${row.space}_${row.page}_${row.epoch}_${row.kind}_${row.subject}_${row.seq}`;
const authorityPath = (row) => `authority/${authorityId(row)}`;
const authorityScope = (row) => Object.fromEntries(["space", "page", "epoch", "kind", "recipient"].map((key) => [key, row[key]]));

test("authority: owner-only immutable records bind page, epoch, kind and subject", async () => {
  await boot();
  const t = T.A, owner = as("ownerA");
  for (const kind of ["membership", "chain", "wrap"]) {
    const row = authority(t, kind), path = authorityPath(row);
    for (const uid of [null, "writerA", "viewerA", "ownerB"]) {
      assert.equal(await set(path, { ...row, uid: uid ?? row.uid }, uid ? as(uid) : NOBODY), 403, `owner-only ${kind}`);
    }
    for (const [key, value] of [
      ["epoch", 2], ["kind", "baseline"], ["subject", "short"], ["uid", "writerA"],
      ["seq", 0], ["count", 0], ["expiresAt", past], ["bytes", "é"],
      ["bytes", "A".repeat(43692)], ["extra", 1],
    ]) {
      assert.equal(await set(path, { ...row, [key]: value }, owner), 403, `${kind} invalid ${key}`);
    }
    assert.equal(await set(`${path}x`, row, owner), 403, "ID binds fields");
    assert.equal(await set(path, row, owner), 200);
    const response = await fetch(`${documents}/${ROOT}/${path}`, { headers: ADMIN });
    assert.deepEqual((await response.json()).fields, fields(row), "original record and routing preserved");
    assert.equal(await set(path, { ...row, bytes: "Y2hhbmdlZA" }, owner), 403, "no ciphertext replacement");
    assert.equal(await set(path, { ...row, expiresAt: future }, owner), 403, "no mutable signed record");
    assert.equal(await remove(path, owner), 403, "live record preserved");
  }
  const row = authority(t, "membership", { subject: "b".repeat(64) });
  assert.equal(await set(authorityPath(row), { ...row, recipient: "writerA" }, owner), 403, "membership is not recipient-specific");
  const chain = authority(t, "chain", { subject: "b".repeat(64) });
  assert.equal(await set(authorityPath(chain), { ...chain, recipient: "writerA" }, owner), 403, "chain uses the empty recipient");
  const wrap = authority(t, "wrap", { subject: "b".repeat(64), recipient: "" });
  assert.equal(await set(authorityPath(wrap), wrap, owner), 403, "wrap must name its recipient");
  await seed(`pages/${sp(t)}`, { space: t.space, page: t.page, epoch: 1, state: "archived", expiresAt: future });
  assert.equal(await set(authorityPath(row), row, owner), 403, "archive refuses new records");
});

test("authority: discovery is page-scoped and wraps disclose only to their recipient", async () => {
  await boot();
  for (const t of [T.A, T.B]) {
    for (const kind of ["membership", "chain", "wrap"]) {
      const row = authority(t, kind);
      assert.equal(await set(authorityPath(row), row, as(row.uid)), 200);
    }
  }
  for (const kind of ["membership", "chain"]) {
    const row = authority(T.A, kind), path = authorityPath(row), scope = authorityScope(row);
    for (const uid of ["ownerA", "writerA", "viewerA"]) {
      assert.equal(await get(path, as(uid)), 200);
      assert.deepEqual(await list("authority", scope, as(uid)), { status: 200, found: 1 });
    }
    assert.equal(await get(path, as("ownerB")), 403, "other tenant");
    assert.equal((await list("authority", { ...scope, page: T.B.page }, as("writerA"))).status, 403);
    assert.equal((await list("authority", {}, as("writerA"))).status, 403, "scope omitted");
  }
  const wrap = authority(T.A, "wrap"), path = authorityPath(wrap), scope = authorityScope(wrap);
  assert.equal(await get(path, as("writerA")), 200);
  assert.deepEqual(await list("authority", scope, as("writerA")), { status: 200, found: 1 });
  for (const uid of ["ownerA", "peerA", "viewerA", "ownerB"]) {
    assert.equal(await get(path, as(uid)), 403, `${uid} is not the recipient`);
    assert.equal((await list("authority", scope, as(uid))).status, 403);
  }
  assert.equal((await list("authority", { ...scope, recipient: "peerA" }, as("writerA"))).status, 403, "another recipient query");
  assert.equal((await list("authority", { ...scope, recipient: "" }, as("writerA"))).status, 403, "empty recipient query");
  const { recipient: _recipient, ...omitted } = scope;
  assert.equal((await list("authority", omitted, as("writerA"))).status, 403, "recipient omitted");
  const own = authority(T.A, "wrap", { subject: "b".repeat(64), recipient: "ownerA" });
  assert.equal(await set(authorityPath(own), own, as("ownerA")), 200);
  assert.equal(await get(authorityPath(own), as("ownerA")), 200, "owner is this wrap's recipient");
  await remove(`members/${sp(T.A)}_writerA`, as("ownerA"));
  assert.equal(await get(path, as("writerA")), 403, "recipient must still be admitted");
});

/** A link holder enrolled by the link's secret, as the hosted reader does it. */
async function enrol(t, uid, link = t.link, token = TOKEN_A) {
  assert.equal(
    await set(`members/${sp(t)}_${uid}`, { space: t.space, page: t.page, uid, role: "viewer", link, token, seq: 1 }, as(uid)),
    200,
    `${uid} enrols by link ${link}`,
  );
}

test("authority: a link holder reads the wraps addressed to its own link and no one else's", async () => {
  await boot();
  const t = T.A, owner = as("ownerA"), otherLink = "30000000-0000-4000-8000-0000000000aa";
  await seed(`links/${sp(t)}_${otherLink}`, { space: t.space, page: t.page, link: otherLink, role: "viewer", token: "F".repeat(43) });
  const linkWrap = (link, subject) => authority(t, "wrap", { subject, recipient: `link:${link}` });
  const mine = linkWrap(t.link, "d".repeat(64)), theirs = linkWrap(otherLink, "e".repeat(64));
  for (const row of [mine, theirs]) assert.equal(await set(authorityPath(row), row, owner), 200, "the owner publishes a link wrap");
  await enrol(t, "holder1");
  await enrol(t, "holder2", otherLink, "F".repeat(43));

  // The holder reads exactly its own link's wrap, by get and by the page-scoped list.
  assert.equal(await get(authorityPath(mine), as("holder1")), 200);
  assert.deepEqual(await list("authority", authorityScope(mine), as("holder1")), { status: 200, found: 1 });
  assert.equal(await get(authorityPath(theirs), as("holder1")), 403, "another link's wrap");
  assert.equal((await list("authority", authorityScope(theirs), as("holder1"))).status, 403, "another link's query");
  assert.equal(await get(authorityPath(theirs), as("holder2")), 200);
  assert.equal(await get(authorityPath(mine), as("holder2")), 403);
  // The prefix is the guard: a bare link ID, a uid-looking value and an unenrolled caller read nothing.
  const bare = authority(t, "wrap", { subject: "f".repeat(64), recipient: t.link });
  assert.equal(await set(authorityPath(bare), bare, owner), 200);
  assert.equal(await get(authorityPath(bare), as("holder1")), 403, "the recipient must carry the link: prefix");
  assert.equal(await get(authorityPath(mine), as("stranger")), 403, "not enrolled");
  assert.equal(await get(authorityPath(mine), NOBODY), 403);
  assert.equal((await list("authority", authorityScope(mine), as("stranger"))).status, 403);
  // Owner and ordinary members hold no link, so a link wrap is not theirs to read.
  for (const uid of ["ownerA", "writerA", "viewerA"]) {
    assert.equal(await get(authorityPath(mine), as(uid)), 403, `${uid} has no link`);
    assert.equal((await list("authority", authorityScope(mine), as(uid))).status, 403);
  }
  // A uid wrap is still its recipient's alone, and a holder is not its uid's recipient by link.
  assert.equal(await get(authorityPath(authority(t, "wrap")), as("holder1")), 403);
  // The holder's own uid-addressed wraps keep working beside the link's.
  const own = authority(t, "wrap", { subject: "9".repeat(64), recipient: "holder1" });
  assert.equal(await set(authorityPath(own), own, owner), 200);
  assert.equal(await get(authorityPath(own), as("holder1")), 200);
  // Another tenant's holder never reads this space's link wrap, whatever its link.
  await enrol(T.B, "holderB", T.B.link, TOKEN_B);
  assert.equal(await get(authorityPath(mine), as("holderB")), 403, "other tenant");
  // Removing the holder's row closes every read at once.
  assert.equal(await remove(`members/${sp(t)}_holder1`, owner), 200);
  assert.equal(await get(authorityPath(mine), as("holder1")), 403, "removed holder");
  assert.equal(await get(`pages/${sp(t)}`, as("holder1")), 403, "removed holder reads no page row");
  assert.equal(await get(`log/${logId(t, 1, t.stream, 1)}`, as("holder1")), 403, "nor the log");
});

test("members: the owner discovers a link's holders; no one else lists members", async () => {
  await boot();
  const t = T.A, otherLink = "30000000-0000-4000-8000-0000000000aa";
  await seed(`links/${sp(t)}_${otherLink}`, { space: t.space, page: t.page, link: otherLink, role: "viewer", token: "F".repeat(43) });
  await enrol(t, "holder1");
  await enrol(t, "holder2");
  await enrol(t, "holder3", otherLink, "F".repeat(43));
  const holders = (link, extra = {}) => ({ space: t.space, page: t.page, link, ...extra });
  assert.deepEqual(await list("members", holders(t.link), as("ownerA")), { status: 200, found: 2 }, "the link's two holders");
  assert.deepEqual(await list("members", holders(otherLink), as("ownerA")), { status: 200, found: 1 });
  assert.deepEqual(await list("members", holders("30000000-0000-4000-8000-0000000000bb"), as("ownerA")), { status: 200, found: 0 }, "an unused link has none");
  // Not the owner: not another tenant's owner, not a member or holder, not a stranger.
  for (const uid of ["ownerB", "writerA", "viewerA", "holder1", "stranger"]) {
    assert.equal((await list("members", holders(t.link), as(uid))).status, 403, `${uid} cannot list members`);
  }
  assert.equal((await list("members", holders(t.link), NOBODY)).status, 403);
  assert.deepEqual(await list("members", holders(t.link, { page: T.B.page }), as("ownerA")), { status: 200, found: 0 }, "another page's scope finds nothing");
  assert.equal((await list("members", { ...holders(t.link), space: T.B.space }, as("ownerA"))).status, 403, "another tenant's space");
  // The owner removes the holders it discovered; deletes are the owner's alone.
  assert.equal(await remove(`members/${sp(t)}_holder1`, as("holder1")), 403, "a holder cannot remove itself");
  assert.equal(await remove(`members/${sp(t)}_holder1`, as("ownerB")), 403);
  assert.equal(await remove(`members/${sp(t)}_holder1`, as("ownerA")), 200);
  assert.equal(await remove(`members/${sp(t)}_holder2`, as("ownerA")), 200);
  assert.deepEqual(await list("members", holders(t.link), as("ownerA")), { status: 200, found: 0 });
  assert.deepEqual(await list("members", holders(otherLink), as("ownerA")), { status: 200, found: 1 }, "the other link's holder stays");
});

test("authority: chunks retain one original count and recipient; only owner cleans expired records", async () => {
  await boot();
  const first = authority(T.A, "membership", { count: 2 }), second = { ...first, seq: 2 };
  assert.equal(await set(authorityPath(second), second, as("ownerA")), 403, "missing first chunk");
  assert.equal(await set(authorityPath(first), first, as("ownerA")), 200);
  assert.equal(await set(authorityPath(second), { ...second, count: 3 }, as("ownerA")), 403, "cannot change count mid-record");
  assert.equal(await set(authorityPath(second), second, as("ownerA")), 200);
  assert.deepEqual(await list("authority", authorityScope(first), as("viewerA")), { status: 200, found: 2 });
  const tooMany = authority(T.A, "membership", { subject: "b".repeat(64), count: 12 });
  assert.equal(await set(authorityPath(tooMany), tooMany, as("ownerA")), 403, "statement chunk bound");
  for (const kind of ["chain", "wrap"]) {
    const row = authority(T.A, kind, { count: 2 });
    assert.equal(await set(authorityPath(row), row, as("ownerA")), 403, "small records stay within one chunk");
  }
  await seed(authorityPath(first), { ...first, expiresAt: past });
  assert.equal(await get(authorityPath(first), as("viewerA")), 403, "expired");
  assert.equal(await remove(authorityPath(first), as("writerA")), 403, "member cannot clean up");
  assert.equal(await remove(authorityPath(first), as("ownerB")), 403, "other tenant cannot clean up");
  assert.equal(await remove(authorityPath(first), as("ownerA")), 200);
  await remove(`pages/${sp(T.A)}`, as("ownerA"));
  assert.equal(await remove(authorityPath(second), as("ownerA")), 200, "deleted page cleanup");
});
