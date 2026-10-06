import assert from "node:assert/strict";
import { createServer, request } from "node:http";
import { test } from "node:test";
import { allowedRequest, colabEmbedDev, loopbackPeer } from "./colab-embed-dev.ts";
import {
  operationStorageKey,
  restoreSendOperation,
  retainSendOperation,
} from "../src/components/colab-embed/send-operation.ts";

test("send recovery rejects invalid storage and refuses a write failure", () => {
  for (const value of ["broken", "null", "{}", '{"operationId":"invalid"}'])
    assert.equal(restoreSendOperation({ getItem: () => value }), null);
  assert.throws(
    () =>
      retainSendOperation(
        {
          setItem: () => {
            throw new Error("Storage full");
          },
        },
        {},
      ),
    /Storage full/,
  );
});

test("loopback peers include IPv4-mapped sockets but exclude network addresses", () => {
  for (const address of ["127.0.0.1", "127.12.34.56", "::1", "::ffff:127.0.0.1"])
    assert.equal(loopbackPeer(address), true);
  for (const address of [
    undefined,
    "192.168.1.8",
    "::ffff:192.168.1.8",
    "127.0.0.256",
    "127.0.0.1.evil",
    "::",
  ])
    assert.equal(loopbackPeer(address), false);
});

test("a network peer cannot execute CLI work with a spoofed loopback Host", async () => {
  let calls = 0;
  let middleware;
  colabEmbedDev(async () => {
    calls++;
  }).configureServer({
    middlewares: {
      use(fn) {
        middleware = fn;
      },
    },
  });
  const req = {
    url: "/__tmt_embed/status",
    method: "GET",
    socket: { remoteAddress: "192.168.1.8" },
    headers: { host: "localhost:5185", origin: "http://localhost:5185", "x-tmt-embed": "1" },
  };
  let body;
  const res = {
    statusCode: 0,
    setHeader() {},
    end(value) {
      body = JSON.parse(value);
    },
  };
  await middleware(req, res, () => assert.fail("bridge route must be handled"));
  assert.equal(res.statusCode, 403);
  assert.match(body.error, /Local handbook/);
  assert.equal(calls, 0);
});

test("embed rejects non-loopback hosts and foreign origins", () => {
  assert.equal(allowedRequest("127.0.0.1:5185", "http://127.0.0.1:5185"), true);
  assert.equal(allowedRequest("localhost:5185", undefined), true);
  for (const host of ["evil.example:5185", "127.0.0.1.evil:5185", undefined])
    assert.equal(allowedRequest(host, undefined), false);
  assert.equal(allowedRequest("127.0.0.1:5185", "http://evil.example"), false);
});

test("development bridge fences explicit sends and owned reply receipts", async () => {
  const calls = [];
  const plugin = colabEmbedDev(async (args) => {
    calls.push(args);
    if (args[0] === "ls")
      return { identities: [{ id: "00000000-0000-4000-8000-000000000001", name: "test-agent" }] };
    if (args[0] === "talk" && args[2] === "uncertain") throw new Error("Uncertain dispatch");
    return args[0] === "talk"
      ? { requestId: "req_test", status: "sent" }
      : { status: "completed", response: "Verified reply" };
  });
  let middleware;
  plugin.configureServer({
    middlewares: {
      use(fn) {
        middleware = fn;
      },
    },
  });
  const server = createServer(
    (req, res) =>
      void middleware(req, res, () => {
        res.statusCode = 404;
        res.end();
      }),
  );
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  const call = (path, body, headers = {}) =>
    new Promise((resolve, reject) => {
      const req = request(
        {
          hostname: "127.0.0.1",
          port,
          path: `/__tmt_embed/${path}`,
          method: body ? "POST" : "GET",
          headers: { "X-Tmt-Embed": "1", "Content-Type": "application/json", ...headers },
        },
        (res) => {
          let data = "";
          res.on("data", (chunk) => (data += chunk));
          res.on("end", () => resolve({ code: res.statusCode, data: JSON.parse(data) }));
        },
      );
      req.on("error", reject);
      req.end(body ? JSON.stringify(body) : undefined);
    });
  const previous = process.env.TMT_EMBED_IDENTITY;
  process.env.TMT_EMBED_IDENTITY = "test-sender";
  try {
    assert.equal(
      (
        await call("send", {
          agent: "--force",
          message: "hello",
          operationId: "10000000-0000-4000-8000-000000000001",
        })
      ).code,
      400,
    );
    assert.equal((await call("status", undefined, { Origin: "http://evil.example" })).code, 403);
    assert.equal((await call("status", undefined, { "X-Tmt-Embed": "" })).code, 403);
    assert.equal((await call("result/req_foreign")).code, 404);
    assert.equal(calls.length, 0);
    const storage = new Map();
    const browserStorage = {
      getItem: (key) => storage.get(key) ?? null,
      setItem: (key, value) => storage.set(key, value),
    };
    const operation = {
      key: "thread:agent:hello",
      operationId: "10000000-0000-4000-8000-000000000001",
      message: "hello",
      pending: true,
      done: false,
    };
    retainSendOperation(browserStorage, operation);
    const sent = await call("send", {
      agent: "00000000-0000-4000-8000-000000000001",
      message: operation.message,
      operationId: operation.operationId,
    });
    assert.equal(sent.data.requestId, "req_test");
    assert.deepEqual(calls[1], [
      "talk",
      "test-agent",
      "hello",
      "--detach",
      "--identity",
      "test-sender",
    ]);
    // Lose the acknowledgement, then recreate browser state as reload/HMR does.
    const recovered = restoreSendOperation(browserStorage);
    assert.equal(recovered.pending, false);
    assert.equal(recovered.done, false);
    assert.equal(recovered.operationId, operation.operationId);
    assert.equal(recovered.message, "hello");
    assert.equal(storage.has(operationStorageKey), true);
    const retry = await call("send", {
      agent: "00000000-0000-4000-8000-000000000001",
      message: recovered.message,
      operationId: recovered.operationId,
    });
    assert.equal(retry.data.requestId, sent.data.requestId);
    assert.equal(calls.filter((args) => args[0] === "talk").length, 1);
    const conflict = await call("send", {
      agent: "00000000-0000-4000-8000-000000000001",
      message: "changed",
      operationId: "10000000-0000-4000-8000-000000000001",
    });
    assert.equal(conflict.code, 409);
    const concurrent = {
      agent: "00000000-0000-4000-8000-000000000001",
      message: "concurrent",
      operationId: "10000000-0000-4000-8000-000000000002",
    };
    const results = await Promise.all([
      call("send", concurrent),
      call("send", concurrent),
      call("send", concurrent),
    ]);
    assert.equal(
      results.every((result) => result.data.requestId === "req_test"),
      true,
    );
    assert.equal(calls.filter((args) => args[0] === "talk" && args[2] === "concurrent").length, 1);
    const uncertain = {
      ...concurrent,
      message: "uncertain",
      operationId: "10000000-0000-4000-8000-000000000003",
    };
    retainSendOperation(browserStorage, { ...operation, ...uncertain });
    assert.equal((await call("send", uncertain)).code, 502);
    const recoveredUncertain = restoreSendOperation(browserStorage);
    assert.equal((await call("send", { ...uncertain, ...recoveredUncertain })).code, 502);
    assert.equal(calls.filter((args) => args[0] === "talk" && args[2] === "uncertain").length, 1);
    assert.equal((await call("result/req_test")).data.response, "Verified reply");
  } finally {
    if (previous === undefined) delete process.env.TMT_EMBED_IDENTITY;
    else process.env.TMT_EMBED_IDENTITY = previous;
    await new Promise((resolve) => server.close(resolve));
  }
});
