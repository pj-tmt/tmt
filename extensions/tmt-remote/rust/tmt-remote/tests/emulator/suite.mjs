// Firestore Rules conformance for the Rules Remote composes from extension fragments (#2163),
// and the free-plan append guard of the shipped SDK bundle against the same emulator (#2180).
// Run only under `firebase emulators:exec --only firestore` (see the tmt-remote skill); there
// is no skip path: without the emulator the suite fails.
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";
import { budget } from "../../assets/remote-v1.js";

const host = process.env.FIRESTORE_EMULATOR_HOST;
assert.ok(host, "FIRESTORE_EMULATOR_HOST is unset: run under `firebase emulators:exec --only firestore`");
const fixtures = new URL("../fixtures/rules/", import.meta.url);
const read = (name) => readFileSync(new URL(name, fixtures), "utf8");
const PROJECT = "demo-tmt-remote";
const documents = `http://${host}/v1/projects/${PROJECT}/databases/(default)/documents`;
const OWNER = { authorization: "Bearer owner", "content-type": "application/json" };

const b64 = (value) => Buffer.from(JSON.stringify(value)).toString("base64url");
// The emulator accepts unsigned tokens and reads request.auth from the payload.
const user = (uid) => ({
  authorization: `Bearer ${b64({ alg: "none", typ: "JWT" })}.${b64({ user_id: uid, sub: uid, firebase: { sign_in_provider: "anonymous" } })}.`,
  "content-type": "application/json",
});
const typed = (value) =>
  value instanceof Date ? { timestampValue: value.toISOString() }
  : typeof value === "boolean" ? { booleanValue: value }
  : typeof value === "number" ? { integerValue: String(value) }
  : { stringValue: String(value) };
const body = (fields) => JSON.stringify({ fields: Object.fromEntries(Object.entries(fields).map(([k, v]) => [k, typed(v)])) });
const DAY = 86_400_000;
const future = new Date(Date.now() + 30 * DAY);
const past = new Date(Date.now() - 30 * DAY);

async function loadRules(content) {
  const response = await fetch(`http://${host}/emulator/v1/projects/${PROJECT}:securityRules`, {
    method: "PUT",
    headers: OWNER,
    body: JSON.stringify({ rules: { files: [{ name: "firestore.rules", content }] } }),
  });
  assert.equal(response.status, 200, `rules did not load: ${await response.text()}`);
}
async function seed(path, fields) {
  const response = await fetch(`${documents}/${path}`, { method: "PATCH", headers: OWNER, body: body(fields) });
  assert.equal(response.status, 200, `seed ${path}: ${response.status}`);
}
/** 200 means the rules admitted the request, 403 that they refused it. */
const get = async (path, headers = {}) => (await fetch(`${documents}/${path}`, { headers })).status;
const create = async (collection, id, fields, headers) =>
  (await fetch(`${documents}/${collection}?documentId=${id}`, { method: "POST", headers, body: body(fields) })).status;
async function seedWorld() {
  await seed("secret/s", { ok: true });
  await seed("m/machine/inbox/1", { ok: true });
  await seed("x/other/docs/1", { ok: true });
  await seed("x/colab/members/u1", { ok: true });
  await seed("x/colab/docs/1", { body: "live", expiresAt: future });
  await seed("x/colab/docs/2", { body: "expired", expiresAt: past });
  await seed("x/colab/open/b", { public: false });
  await seed("x/colab/open/pub", { public: true });
  await seed("x/notes/notes/n1", { body: "note" });
}

// The hostile fragments are spliced into the wrapper WITHOUT the composer's admission. Each
// leak below is real, which is what makes the composer's refusal of the same file matter.
const hostile = readdirSync(new URL("hostile/", fixtures)).filter((n) => n.endsWith(".rules")).sort();
const marker = "    match /x/colab {\n";
for (const name of hostile) {
  test(`raw splice leaks: ${name}`, async () => {
    const fragment = read(`hostile/${name}`);
    const probe = /^\/\/ probe: (\S+)/.exec(fragment)?.[1];
    assert.ok(probe, "the hostile file names its probe document");
    const wrapper = read("wrapper-colab-empty.rules");
    assert.ok(wrapper.includes(marker));
    await loadRules(wrapper.replace(marker, marker + fragment));
    await seedWorld();
    assert.equal(await get(probe), 200, `${name} should be readable by an unauthenticated client`);
  });
}

test("the empty wrapper and the baseline deny everything", async () => {
  await loadRules(read("wrapper-colab-empty.rules"));
  await seedWorld();
  for (const path of ["x/colab/docs/1", "secret/s", "x/other/docs/1", "m/machine/inbox/1", "x/colab"]) {
    assert.equal(await get(path), 403, path);
    assert.equal(await get(path, user("u1")), 403, path);
  }
});

test("composed rules: default deny outside every extension root", async () => {
  await loadRules(read("composed.rules"));
  await seedWorld();
  for (const path of ["secret/s", "m/machine/inbox/1", "x/other/docs/1", "x/colab", "x/notes"]) {
    assert.equal(await get(path), 403, `anonymous ${path}`);
    assert.equal(await get(path, user("u1")), 403, `signed-in ${path}`);
  }
  assert.equal(await create("x/zzz/docs", "1", { body: "a" }, user("u1")), 403);
  assert.equal(await create("secret", "t", { body: "a" }, user("u1")), 403);
});

test("composed rules: a fragment's grants stay inside its own extension root", async () => {
  await loadRules(read("composed.rules"));
  await seedWorld();
  // notes lets any signed-in user read x/notes/notes/*; that must not reach colab or the reverse.
  assert.equal(await get("x/notes/notes/n1", user("u2")), 200);
  assert.equal(await get("x/colab/docs/1", user("u2")), 403, "u2 is not a colab member");
  assert.equal(await get("x/notes/docs/1", user("u1")), 403, "a colab-shaped path under notes");
  assert.equal(await get("x/colab/notes/n1", user("u1")), 403, "a notes-shaped path under colab");
  assert.equal(await get("x/colab/docs/1", user("u1")), 200, "u1 is a member");
});

test("composed rules: macros read the extension's own documents and expiry is enforced", async () => {
  await loadRules(read("composed.rules"));
  await seedWorld();
  assert.equal(await get("x/colab/docs/1"), 403, "anonymous");
  assert.equal(await get("x/colab/docs/1", user("u1")), 200, "member, unexpired");
  assert.equal(await get("x/colab/docs/2", user("u1")), 403, "member, expired by Rules");
  assert.equal(await get("x/colab/members/u1", user("u1")), 200, "own row");
  assert.equal(await get("x/colab/members/u1", user("u2")), 403, "someone else's row");
  assert.equal(await get("x/colab/open/b", user("u1")), 200, "recursive wildcard, listed uid");
  assert.equal(await get("x/colab/open/b", user("u3")), 403, "recursive wildcard, other uid");
  assert.equal(await get("x/colab/open/pub", user("u3")), 200, "recursive wildcard, public data");
});

test("composed rules: create-only grants do not widen to update or delete", async () => {
  await loadRules(read("composed.rules"));
  await seedWorld();
  const u1 = user("u1");
  assert.equal(await create("x/colab/docs", "9", { body: "new", expiresAt: future }, u1), 200);
  assert.equal(await create("x/colab/docs", "10", { body: "new", expiresAt: future, extra: "x" }, u1), 403, "field set");
  assert.equal(await create("x/colab/docs", "11", { body: "new", expiresAt: future }, user("u2")), 403, "non-member");
  const patch = await fetch(`${documents}/x/colab/docs/9?updateMask.fieldPaths=body`, { method: "PATCH", headers: u1, body: body({ body: "changed" }) });
  assert.equal(patch.status, 403, "update");
  assert.equal((await fetch(`${documents}/x/colab/docs/9`, { method: "DELETE", headers: u1 })).status, 403, "delete");
  assert.equal(await create("x/colab/heads", "p1", { seq: 1 }, u1), 200, "getAfter macro");
  assert.equal(await create("x/colab/heads", "p2", { seq: 1 }), 403, "anonymous");
});

// Native emulator_artifact_is_the_exact_verified_deployment_output binds this checked file
// to the bytes captured from an authorized run, including its deployment marker.
test('deployed artifact: member grants, expiry, isolation and create-only refusal', async () => {
  await loadRules(read('deployed.rules'));
  await seedWorld();
  const u1 = user('u1');
  assert.equal(await get('x/colab/docs/1', u1), 200);
  assert.equal(await get('x/colab/docs/1', user('u2')), 403);
  assert.equal(await get('x/colab/docs/2', u1), 403, 'expired');
  for (const path of [
    'secret/s',
    'm/machine/inbox/1',
    'x/other/docs/1',
    'x/notes/docs/1',
    'x/colab/notes/n1',
  ]) {
    assert.equal(await get(path, u1), 403, `outside grant ${path}`);
  }
  assert.equal(await get('x/notes/notes/n1', user('u2')), 200);
  assert.equal(
    await create('x/colab/docs', 'deployed-member', { body: 'new', expiresAt: future }, u1),
    200
  );
  assert.equal(
    await create(
      'x/colab/docs',
      'deployed-outsider',
      { body: 'new', expiresAt: future },
      user('u2')
    ),
    403
  );
  const document = `${documents}/x/colab/docs/deployed-member`;
  assert.equal(
    (
      await fetch(`${document}?updateMask.fieldPaths=body`, {
        method: 'PATCH',
        headers: u1,
        body: body({ body: 'changed' }),
      })
    ).status,
    403,
    'update'
  );
  assert.equal((await fetch(document, { method: 'DELETE', headers: u1 })).status, 403, 'delete');
});

// The guard judges every append before it is sent. The emulator does not enforce quotas, so this
// proves the shipped client stops its own requests below the modeled daily allowance, not
// Google's counter: 1000 members and one writer model 16 appends a day (warn 11, refuse 14).
test("budget guard: refuses before the modeled limit and sends nothing more until the reset", async () => {
  await loadRules(read("deployed.rules"));
  await seedWorld();
  const u1 = user("u1");
  const model = budget.BudgetModel.create(1000, 1);
  assert.deepEqual([model.dailyAppendLimit(), model.warnAt(), model.refuseAt()], [16, 11, 14]);
  const now = Date.UTC(2026, 9, 9, 20, 0, 0);
  const resetAt = Date.UTC(2026, 9, 10, 7, 0, 0);
  const run = Date.now().toString(36);
  let usage = budget.newUsage(now);
  let sent = 0;
  const append = async (id, at) => {
    const decision = budget.decide(model, usage, at);
    if (decision.decision === "refuse") return decision;
    assert.equal(await create("x/colab/docs", `guard-${run}-${id}`, { body: id, expiresAt: future }, u1), 200, id);
    sent += 1;
    usage = budget.recordedUsage(usage, at);
    return decision;
  };
  const stored = async () => {
    const response = await fetch(`${documents}/x/colab/docs?pageSize=300`, { headers: OWNER });
    assert.equal(response.status, 200);
    const { documents: found = [] } = await response.json();
    return found.map((d) => d.name.split("/").pop()).filter((name) => name.startsWith(`guard-${run}-`)).sort();
  };

  const verdicts = [];
  for (let i = 0; i < model.refuseAt(); i += 1) verdicts.push((await append(`a${i}`, now)).decision);
  assert.deepEqual(verdicts, [...Array(model.warnAt()).fill("allow"), ...Array(3).fill("warn")]);
  assert.equal(sent, model.refuseAt());
  assert.ok(model.refuseAt() < model.dailyAppendLimit(), "refuses below the modeled allowance");

  const refused = await append("held", now);
  assert.deepEqual(refused, { decision: "refuse", resetAtMs: resetAt, retryAfterMs: resetAt - now });
  for (let i = 0; i < 5; i += 1) assert.equal((await append("held", now + i)).decision, "refuse");
  const before = await stored();
  assert.equal(before.length, model.refuseAt(), "the emulator saw exactly the allowed appends");
  assert.ok(!before.includes(`guard-${run}-held`), "the refused update was never sent");
  assert.equal(sent, model.refuseAt());

  // The same update goes out unchanged once the Pacific day rolls over.
  assert.equal((await append("held", resetAt)).decision, "allow");
  const after = await stored();
  assert.equal(after.length, model.refuseAt() + 1);
  assert.ok(after.includes(`guard-${run}-held`));
});
