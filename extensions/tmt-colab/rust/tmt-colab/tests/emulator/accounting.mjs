// Test-only atomic accounting fixture. Administrative reads build candidate writes;
// admission is always exercised by the supplied authenticated caller, never ADMIN.
// This is not the future #2452 transport or a client transaction implementation.
import assert from "node:assert/strict";

export function accountingFixture({ documents, root, admin, fields }) {
  const name = (path) => `${documents.slice(documents.indexOf("/v1/") + 4)}/${root}/${path}`;
  const pathUrl = (path) => `${documents}/${root}/${path}`;
  const charge = (data) => (data.envelope ?? data.bytes ?? "").length + 256;
  const decode = (value) =>
    value.timestampValue
      ? new Date(value.timestampValue)
      : value.integerValue !== undefined
        ? Number(value.integerValue)
        : value.booleanValue !== undefined
          ? value.booleanValue
          : value.stringValue;
  async function read(path) {
    const response = await fetch(pathUrl(path), { headers: admin });
    if (response.status === 404) return undefined;
    assert.equal(response.status, 200, `fixture read ${path}`);
    return Object.fromEntries(
      Object.entries((await response.json()).fields).map(([key, value]) => [key, decode(value)]),
    );
  }
  async function rawSet(path, data, headers = admin) {
    return (
      await fetch(pathUrl(path), {
        method: "PATCH",
        headers,
        body: JSON.stringify({ fields: fields(data) }),
      })
    ).status;
  }
  const write = (path, data) =>
    data === undefined
      ? { delete: name(path) }
      : { update: { name: name(path), fields: fields(data) } };
  async function submit(writes, headers) {
    return (
      await fetch(`${documents}:commit`, {
        method: "POST",
        headers,
        body: JSON.stringify({ writes }),
      })
    ).status;
  }
  const caller = (headers) => {
    try {
      return JSON.parse(Buffer.from(headers.authorization.split(".")[1], "base64url").toString())
        .user_id;
    } catch {
      return undefined;
    }
  };
  const uidPath = (space, uid) => `uid-budgets/${space}_${uid}`;
  const spacePath = (space) => `space-budgets/${space}`;
  const initial = (space, uid) => ({
    space,
    ...(uid === undefined ? {} : { uid }),
    chargedBytes: 256,
    entries: 1,
    windowStart: new Date(Date.now() - 61_000),
    mutations: 1,
    targetKind: "spaces",
    targetId: space,
    action: "create",
    actor: uid ?? "owner",
  });
  // ADMIN seeds model existing resident rows, including budget-row overhead. Normal
  // mutations below still have to prove all deltas to Rules. Tests may rawSet a
  // counter deliberately to place a boundary or invalid state.
  async function seed(path, data) {
    if (path.startsWith("uid-budgets/") || path.startsWith("space-budgets/")) {
      assert.equal(await rawSet(path, data), 200);
      return;
    }
    const old = await read(path);
    const owner = data.owner ?? (await read(`spaces/${data.space}`))?.owner;
    const allocation =
      data.quotaUid ??
      old?.quotaUid ??
      (path.startsWith("log/") || path.startsWith("checkpoints/") ? data.uid : owner);
    const next = { ...data, quotaUid: allocation, quotaBytes: data.quotaBytes ?? charge(data) };
    if (old) {
      assert.equal(old.quotaUid, allocation, "seed does not silently transfer an allocation");
      assert.equal(await rawSet(path, next), 200);
      return;
    }
    const uPath = uidPath(data.space, allocation),
      sPath = spacePath(data.space);
    const uBefore = await read(uPath),
      sBefore = await read(sPath);
    const u = uBefore ?? initial(data.space, allocation),
      s = sBefore ?? initial(data.space);
    u.chargedBytes += next.quotaBytes + (sBefore ? 0 : 256);
    u.entries += 1 + (sBefore ? 0 : 1);
    s.chargedBytes += next.quotaBytes + (uBefore ? 0 : 256);
    s.entries += 1 + (uBefore ? 0 : 1);
    assert.equal(
      await submit([write(path, next), write(uPath, u), write(sPath, s)], admin),
      200,
      `seed ${path}`,
    );
  }
  function rateWrite(path, row) {
    const reset = Date.now() >= row.windowStart.getTime() + 60_000;
    const result = write(path, { ...row, mutations: reset ? 1 : row.mutations + 1 });
    if (reset)
      result.updateTransforms = [{ fieldPath: "windowStart", setToServerValue: "REQUEST_TIME" }];
    return result;
  }
  async function prepare(path, data, headers) {
    const actor = caller(headers);
    const before = await read(path);
    // An unsigned/anonymous candidate still reaches the Rules and must refuse.
    if (!actor) return [write(path, data)];
    const space = data?.space ?? before?.space;
    if (!space) return [write(path, data)];
    const [kind, id] = path.split("/");
    const action = data === undefined ? "delete" : before ? "update" : "create";
    const binding = { targetKind: kind, targetId: id, action, actor };
    const uPath = uidPath(space, actor),
      sPath = spacePath(space);
    const uBefore = await read(uPath),
      sBefore = await read(sPath);
    const u = { ...(uBefore ?? initial(space, actor)), ...binding };
    const s = { ...(sBefore ?? initial(space)), ...binding };
    let next =
      data === undefined
        ? undefined
        : {
            ...data,
            quotaUid: data.quotaUid ?? before?.quotaUid ?? actor,
            quotaBytes: data.quotaBytes ?? charge(data),
          };
    const creditWrites = [];
    if (action === "create") {
      u.chargedBytes += next.quotaBytes + (sBefore ? 0 : 256);
      u.entries += 1 + (sBefore ? 0 : 1);
      s.chargedBytes += next.quotaBytes + (uBefore ? 0 : 256);
      s.entries += 1 + (uBefore ? 0 : 1);
    } else if (action === "delete" && before) {
      const retired = kind === "uid-budgets";
      const credit = retired ? 256 : before.quotaBytes;
      s.chargedBytes -= credit;
      s.entries -= 1;
      if (!retired && before.quotaUid === actor) {
        u.chargedBytes -= credit;
        u.entries -= 1;
      } else if (!retired) {
        const originalPath = uidPath(space, before.quotaUid);
        const original = await read(originalPath);
        creditWrites.push(
          write(originalPath, {
            ...original,
            ...binding,
            chargedBytes: original.chargedBytes - credit,
            entries: original.entries - 1,
          }),
        );
      }
    }
    return [write(path, next), rateWrite(uPath, u), rateWrite(sPath, s), ...creditWrites];
  }
  async function set(path, data, headers) {
    return submit(await prepare(path, data, headers), headers);
  }
  async function remove(path, headers) {
    return submit(await prepare(path, undefined, headers), headers);
  }
  return { seed, read, rawSet, write, submit, prepare, set, remove, charge, uidPath, spacePath };
}
