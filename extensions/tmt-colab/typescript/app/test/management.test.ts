import { readFileSync } from 'node:fs';
import { beforeEach, expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import {
  ManagementClient,
  managementLog,
  newLink,
  project,
  selectionBytes,
  type Pending,
  type Selection,
} from '../src/management.js';
import type { Registration } from '../src/registration.js';
import type { Bootstrap, PageInfo } from '../src/bootstrap.js';
const records = new Map<string, unknown>();
vi.mock('../src/storage.js', () => ({
  record: async (key: string, ...values: unknown[]) => {
    if (values.length) records.set(key, structuredClone(values[0]));
    else return records.get(key);
  },
}));
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (n) => parseInt(n, 16));
const json = (x: unknown) => c.text(JSON.stringify(x));
const mount = new URL('http://localhost/r/abcd/x/colab/');
const page: PageInfo = {
  pageId: v.page,
  epoch: '1',
  sharing: 'private',
  history: 'shared',
  archived: false,
};
async function fixture() {
  const signer = await crypto.subtle.importKey(
    'pkcs8',
    c.concat(hex('302e020100300506032b657004220420'), hex(v.seed)),
    'Ed25519',
    false,
    ['sign'],
  );
  const issuer = c.statement.Envelope.fromJson(json(v.statement));
  const g = await issuer.verifyNext(v.space, hex(v.public), null);
  const certificate = c.certificate.input({
    space: v.space,
    issuerKind: 'member',
    issuerId: g.head.ownerMember.id,
    deviceId: v.device,
    signingKey: hex(v.public),
    encryptionKey: c.wrap.Envelope.fromJson(json(v.wrap)).header().recipientKey,
    membershipRevision: '1',
    issuedAt: Date.now() - 1000,
    expiresAt: Date.now() + 86400000,
  });
  const chain = c.certificate.Chain.fromJson(
    json({
      version: 1,
      issuerStatement: c.encodeBinary(g.head.hash),
      deviceCertificate: c.encodeBinary(certificate),
      issuerSignature: c.encodeBinary(await c.sign(signer, certificate)),
    }),
  );
  const registration = {
    deviceId: v.device,
    issuer,
    chain,
    keys: { sign: signer, signPublic: hex(v.public) },
  } as Registration;
  const log = [g];
  const raw = [c.encodeBinary(issuer.toJson())];
  async function append(operation: string, value: unknown) {
    const bytes = json(value),
      prev = log[log.length - 1].head;
    const input = c.statement.input({
      space: v.space,
      operation,
      revision: String(prev.revision + 1n),
      previousHash: prev.hash,
      payloadDigest: await c.digest(bytes),
    });
    const envelope = c.statement.Envelope.fromJson(
      json({
        statement: c.encodeBinary(input),
        payload: c.encodeBinary(bytes),
        signature: c.encodeBinary(await c.sign(signer, input)),
      }),
    );
    log.push(await envelope.verifyNext(v.space, hex(v.public), prev));
    raw.push(c.encodeBinary(envelope.toJson()));
  }
  await append('page.share', { pageId: v.page, mode: 'private', epoch: '1' });
  const client = new ManagementClient(mount, registration);
  const boot: Bootstrap = { space: v.space, owner: hex(v.public), revision: '2', pages: [page] };
  const view = project(page, log);
  return { client, registration, log, raw, append, boot, view };
}
beforeEach(() => {
  records.clear();
  vi.restoreAllMocks();
  vi.stubGlobal('navigator', { locks: { request: async (_: string, fn: () => unknown) => fn() } });
});
it('freezes user selections, seed, operation ID and signed bytes across an explicit retry', async () => {
  const f = await fixture(),
    link = newLink('viewer', [v.page]),
    original = { ...link };
  const preparing = f.client.prepare(f.view, { operation: 'link.add', value: link });
  link.seed = c.encodeBinary(new Uint8Array(32));
  const pending = await preparing,
    envelope = JSON.parse(pending.body),
    payload = c.binary(envelope.payload, 16384);
  expect(JSON.parse(c.decodeText(payload))).toEqual(original);
  expect(pending.artifact?.seed).toBe(original.seed);
  const bytes = c.binary(envelope.request, 1024);
  expect(await c.strictVerify(hex(v.public), c.binary(envelope.signature, 64, 64), bytes)).toBe(
    true,
  );
  const fields = c.fields(bytes, 11);
  expect(c.decodeText(fields[4])).toBe(f.view.revision);
  expect(c.equal(fields[7], await c.digest(payload))).toBe(true);
  const fetcher = vi
    .fn()
    .mockRejectedValueOnce(new Error('lost receipt'))
    .mockResolvedValue(
      new Response(
        JSON.stringify({
          operationId: pending.id,
          membershipHead: { revision: '3', statementHash: c.encodeBinary(new Uint8Array(32)) },
        }),
      ),
    );
  vi.stubGlobal('fetch', fetcher);
  await expect(f.client.send(pending)).rejects.toMatchObject({ code: 'UNKNOWN' });
  await f.client.send(pending);
  expect(fetcher.mock.calls.map((args) => args[1].body)).toEqual([pending.body, pending.body]);
  vi.spyOn(Date, 'now').mockReturnValue(pending.expiresAt);
  await expect(f.client.send(pending)).rejects.toMatchObject({ code: 'EXPIRED' });
  expect(fetcher).toHaveBeenCalledTimes(2);
  expect([...records.values()].some((x) => JSON.stringify(x).includes(original.seed))).toBe(false);
});
it('rejects computed fields, wrong page, incomplete assignment and owner-member removal', async () => {
  const { view } = await fixture();
  const member = {
    id: crypto.randomUUID(),
    role: 'editor' as const,
    pages: [v.page, 'ffffffff-ffff-4fff-bfff-ffffffffffff'].sort(),
  };
  view.members.push(member);
  const selections = [
    { operation: 'page.share', value: { pageId: v.page, mode: 'public', publishedKeys: [] } },
    { operation: 'epoch.advance', value: { pageId: member.id } },
    { operation: 'member.remove', value: { memberId: member.id, pages: [v.page] } },
    { operation: 'member.remove', value: { memberId: view.ownerMember, pages: [v.page] } },
    { operation: 'link.add', value: { ...newLink('viewer', [v.page]), seed: 'invalid' } },
  ];
  for (const s of selections) expect(() => selectionBytes(view, s as Selection)).toThrow();
  expect(
    JSON.parse(
      c.decodeText(
        selectionBytes(view, {
          operation: 'member.role',
          value: { memberId: member.id, role: 'viewer', pages: member.pages },
        }),
      ),
    ),
  ).toEqual({ memberId: member.id, role: 'viewer', pages: member.pages });
});
it.each([
  ['INVALID', 400],
  ['DENIED', 403],
  ['EXPIRED', 403],
  ['STALE_HEAD', 409],
  ['CONFLICT', 409],
  ['CAPACITY', 503],
  ['UNAVAILABLE', 503],
])('preserves definite rejection %s without mutation or automatic retry', async (code, status) => {
  const f = await fixture(),
    p = await f.client.prepare(f.view, { operation: 'epoch.advance', value: { pageId: v.page } });
  const fetcher = vi.fn(
    async () => new Response(JSON.stringify({ code }), { status: Number(status) }),
  );
  vi.stubGlobal('fetch', fetcher);
  await expect(f.client.send(p)).rejects.toMatchObject({ code });
  expect(fetcher).toHaveBeenCalledTimes(1);
  expect(records.size).toBe(0);
});
it.each([
  [503, { code: 'UNRECOGNIZED' }],
  [503, { code: 'INVALID' }],
  [500, { code: 'UNAVAILABLE' }],
  [503, { code: 'UNAVAILABLE', detail: 'unexpected field' }],
])('keeps an unrecognized error reply at status %s uncertain', async (status, body) => {
  const f = await fixture(),
    p = await f.client.prepare(f.view, { operation: 'epoch.advance', value: { pageId: v.page } });
  const fetcher = vi.fn(async () => new Response(JSON.stringify(body), { status }));
  vi.stubGlobal('fetch', fetcher);
  await expect(f.client.send(p)).rejects.toMatchObject({ code: 'UNKNOWN' });
  expect(fetcher).toHaveBeenCalledTimes(1);
});
it('does not accept a wrong operation, malformed or oversized acknowledgment', async () => {
  const f = await fixture(),
    p = await f.client.prepare(f.view, { operation: 'epoch.advance', value: { pageId: v.page } });
  for (const body of [
    JSON.stringify({
      operationId: crypto.randomUUID(),
      membershipHead: { revision: '3', statementHash: c.encodeBinary(new Uint8Array(32)) },
    }),
    '{',
    'x'.repeat(8193),
  ]) {
    vi.stubGlobal('fetch', async () => new Response(body));
    await expect(f.client.send(p)).rejects.toMatchObject({ code: 'UNKNOWN' });
  }
});
it('verifies the acknowledged position and requested change even when a later head exists', async () => {
  const f = await fixture(),
    p = await f.client.prepare(f.view, {
      operation: 'page.history',
      value: { pageId: v.page, mode: 'current' },
    });
  await f.append('page.history', { pageId: v.page, mode: 'current' });
  const ack = {
    operationId: p.id,
    membershipHead: { revision: '3', statementHash: c.encodeBinary(f.log[2].head.hash) },
  };
  await f.append('page.history', { pageId: v.page, mode: 'shared' });
  vi.spyOn(f.client, 'snapshot').mockResolvedValue({ boot: f.boot, log: f.log });
  await f.client.read(v.page);
  expect((await f.client.verify(p, ack)).page.history).toBe('shared');
  await expect(
    f.client.verify(p, {
      ...ack,
      membershipHead: { ...ack.membershipHead, statementHash: c.encodeBinary(new Uint8Array(32)) },
    }),
  ).rejects.toThrow();
  await expect(
    f.client.verify(p, {
      ...ack,
      membershipHead: { revision: '2', statementHash: c.encodeBinary(f.log[1].head.hash) },
    }),
  ).rejects.toThrow();
  const other = { ...p, operation: 'page.share' } as Pending;
  await expect(f.client.verify(other, ack)).rejects.toThrow();
});
it('admits paged signed metadata without opening content; forged log, wrong scope and cancellation close the socket', async () => {
  const f = await fixture();
  let mode = 'valid',
    closed = 0,
    sent: string[] = [];
  class Socket {
    protocol = 'colab-sync-v1';
    onopen: (() => void) | null = null;
    onclose: (() => void) | null = null;
    onerror: (() => void) | null = null;
    onmessage: ((e: { data: string }) => void) | null = null;
    constructor() {
      queueMicrotask(() => this.onopen?.());
    }
    close() {
      closed++;
    }
    send(raw: string) {
      sent.push(raw);
      const request = JSON.parse(raw);
      if (mode === 'silent') return;
      const first = request.type === 'hello',
        head = f.log[1].head;
      const frame = {
        version: 1,
        type: 'catchup',
        space: f.boot.space,
        page: mode === 'scope' ? crypto.randomUUID() : v.page,
        epoch: '1',
        streams: [],
        more: true,
        ...(first
          ? {
              baseline: 'unsupported-content-baseline',
              membershipHead: {
                revision: '2',
                statementHash: c.encodeBinary(head.hash),
                ownerKey: c.encodeBinary(f.boot.owner),
                statements: [
                  mode === 'forged'
                    ? c.encodeBinary(
                        json({
                          ...JSON.parse(c.decodeText(c.binary(f.raw[0], 16384))),
                          signature: c.encodeBinary(new Uint8Array(64)),
                        }),
                      )
                    : f.raw[0],
                ],
                more: true,
              },
            }
          : { membership: { statements: [f.raw[1]], more: false } }),
      };
      queueMicrotask(() => this.onmessage?.({ data: JSON.stringify(frame) }));
    }
  }
  vi.stubGlobal('WebSocket', Socket);
  const log = await managementLog(mount, f.boot, page, f.registration);
  expect(log.length).toBe(2);
  expect(closed).toBe(1);
  expect(sent.map((x) => JSON.parse(x).type)).toEqual(['hello', 'ack']);
  expect(records.get(`log:${v.space}`)).toEqual(f.raw);
  for (const invalid of ['forged', 'scope']) {
    records.clear();
    mode = invalid;
    await expect(managementLog(mount, f.boot, page, f.registration)).rejects.toThrow();
  }
  mode = 'silent';
  const controller = new AbortController();
  const loading = managementLog(mount, f.boot, page, f.registration, controller.signal);
  controller.abort();
  await expect(loading).rejects.toMatchObject({ code: 'UNAVAILABLE' });
  expect(closed).toBe(4);
});

/** Discovery is routing evidence; all policies below still require owner signatures. */
function serve(f: Awaited<ReturnType<typeof fixture>>) {
  const contexts: string[] = [];
  let closed = 0;
  vi.stubGlobal('location', { hash: `#space=${f.boot.space}` });
  vi.stubGlobal('history', { replaceState: vi.fn() });
  vi.stubGlobal(
    'fetch',
    async () =>
      new Response(
        JSON.stringify({
          spaceId: f.boot.space,
          ownerKey: c.encodeBinary(f.boot.owner),
          revision: String(f.log.at(-1)!.head.revision),
          pages: f.boot.pages,
        }),
      ),
  );
  class Socket {
    protocol = 'colab-sync-v1';
    onopen: (() => void) | null = null;
    onclose: (() => void) | null = null;
    onerror: (() => void) | null = null;
    onmessage: ((e: { data: string }) => void) | null = null;
    constructor() {
      queueMicrotask(() => this.onopen?.());
    }
    close() {
      closed++;
    }
    send(raw: string) {
      const hello = JSON.parse(raw);
      if (hello.type !== 'hello') return;
      contexts.push(hello.page);
      // Deleted target catchup is denied, so only a different readable context works.
      if (!f.boot.pages.some((p) => p.pageId === hello.page)) {
        queueMicrotask(() => this.onclose?.());
        return;
      }
      const head = f.log.at(-1)!.head;
      queueMicrotask(() =>
        this.onmessage?.({
          data: JSON.stringify({
            version: 1,
            type: 'catchup',
            space: f.boot.space,
            page: hello.page,
            epoch: hello.epoch,
            streams: [],
            baseline: null,
            more: true,
            membershipHead: {
              revision: String(head.revision),
              statementHash: c.encodeBinary(head.hash),
              ownerKey: c.encodeBinary(f.boot.owner),
              statements: f.raw.slice(Number(hello.membershipRevision)),
              more: false,
            },
          }),
        }),
      );
    }
  }
  vi.stubGlobal('WebSocket', Socket);
  return { contexts, closed: () => closed };
}
function acknowledgment(f: Awaited<ReturnType<typeof fixture>>, p: Pending) {
  const head = f.log.at(-1)!.head;
  return {
    operationId: p.id,
    membershipHead: { revision: String(head.revision), statementHash: c.encodeBinary(head.hash) },
  };
}
it('projects signed retention and rejects invalid days, extra fields and wrong page before signing', async () => {
  const f = await fixture();
  expect(f.view.page.retentionDays).toBe(30);
  for (const days of [1, Number.MAX_SAFE_INTEGER, null]) {
    const selected = { operation: 'retention.set' as const, value: { pageId: v.page, days } };
    const pending = await f.client.prepare(f.view, selected);
    expect(JSON.parse(c.decodeText(c.binary(JSON.parse(pending.body).payload, 16384)))).toEqual(
      selected.value,
    );
    await f.append('retention.set', selected.value);
    expect(project(page, f.log).page.retentionDays).toBe(days);
  }
  for (const days of [0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1, NaN, Infinity]) {
    await expect(
      f.client.prepare(f.view, { operation: 'retention.set', value: { pageId: v.page, days } }),
    ).rejects.toThrow();
  }
  for (const value of [
    { pageId: v.page, days: 1, expiresAt: 123 },
    { pageId: crypto.randomUUID(), days: 1 },
  ]) {
    expect(() =>
      selectionBytes(f.view, { operation: 'retention.set', value } as Selection),
    ).toThrow();
  }
});
it('verifies retention at its acknowledged position despite a later retention change', async () => {
  const f = await fixture(),
    p = await f.client.prepare(f.view, {
      operation: 'retention.set',
      value: { pageId: v.page, days: null },
    });
  await f.append('retention.set', { pageId: v.page, days: null });
  const ack = acknowledgment(f, p);
  await f.append('retention.set', { pageId: v.page, days: 14 });
  serve(f);
  expect((await f.client.verify(p, ack)).page.retentionDays).toBe(14);
});
it('reads and verifies archive through the same owner-readable page, without relying on its discovery label', async () => {
  const f = await fixture(),
    other = { ...page, pageId: crypto.randomUUID() };
  f.boot.pages = [other, page].sort((a, b) => a.pageId.localeCompare(b.pageId));
  const transport = serve(f),
    view = await f.client.read(v.page);
  const p = await f.client.prepare(view, { operation: 'page.archive', value: { pageId: v.page } });
  await f.append('page.archive', { pageId: v.page });
  // Discovery deliberately still says active; policy is established by the log.
  const archived = await f.client.verify(p, acknowledgment(f, p));
  expect(archived.page.archived).toBe(true);
  expect(archived.page.deleted).toBe(false);
  expect(transport.contexts).toEqual([v.page, v.page]);
  expect(transport.closed()).toBe(2);
  await expect(
    f.client.prepare(archived, { operation: 'page.archive', value: { pageId: v.page } }),
  ).rejects.toThrow();
});
it('freezes initiating context and verifies deletion using discovery plus another readable signed-log context', async () => {
  const f = await fixture(),
    other = { ...page, pageId: crypto.randomUUID() };
  f.boot.pages = [other, page].sort((a, b) => a.pageId.localeCompare(b.pageId));
  const transport = serve(f),
    preparing = f.client.prepare(f.view, { operation: 'page.delete', value: { pageId: v.page } });
  f.view.page.epoch = '99';
  f.view.revision = '99';
  const p = await preparing;
  expect(p.context.epoch).toBe('1');
  expect(p.expectedRevision).toBe('2');
  expect(Object.isFrozen(p.context)).toBe(true);
  await f.append('page.delete', { pageId: v.page });
  const ack = acknowledgment(f, p);
  f.boot.pages = [other];
  const result = await f.client.verify(p, ack);
  expect(result.page.deleted).toBe(true);
  expect(result.page.pageId).toBe(v.page);
  expect(transport.contexts).toEqual([other.pageId]);
  expect(transport.closed()).toBe(1);
  await expect(
    f.client.prepare(result, { operation: 'retention.set', value: { pageId: v.page, days: null } }),
  ).rejects.toThrow();
  await expect(
    f.client.verify(p, {
      ...ack,
      membershipHead: { ...ack.membershipHead, statementHash: c.encodeBinary(new Uint8Array(32)) },
    }),
  ).rejects.toThrow();
  f.boot.pages = [other, page].sort((a, b) => a.pageId.localeCompare(b.pageId));
  await expect(f.client.verify(p, ack)).rejects.toThrow();
});
it('does not claim deletion from discovery absence without the matching signed change', async () => {
  const f = await fixture(),
    p = await f.client.prepare(f.view, { operation: 'page.delete', value: { pageId: v.page } });
  await f.append('page.archive', { pageId: v.page });
  f.boot.pages = [{ ...page, pageId: crypto.randomUUID() }];
  serve(f);
  await expect(f.client.verify(p, acknowledgment(f, p))).rejects.toThrow();
});
it('keeps last-page deletion acknowledged and awaiting verification without trying the denied target socket or resending', async () => {
  const f = await fixture(),
    p = await f.client.prepare(f.view, { operation: 'page.delete', value: { pageId: v.page } });
  await f.append('page.delete', { pageId: v.page });
  f.boot.pages = [];
  const transport = serve(f),
    ack = acknowledgment(f, p),
    send = vi.spyOn(f.client, 'send');
  for (let attempt = 0; attempt < 2; attempt++) {
    await expect(f.client.verify(p, ack)).rejects.toMatchObject({ code: 'AWAITING_VERIFICATION' });
  }
  expect(transport.contexts).toEqual([]);
  expect(send).not.toHaveBeenCalled();
});

it('reads metadata after a baseline frame and verifies referenced statement chunks before publication', async () => {
  const f = await fixture();
  for (const forged of [false, true]) {
    records.clear();
    const hash = c.encodeBinary(f.log[1].head.hash);
    let closed = 0,
      stage = 0;
    class Socket {
      protocol = 'colab-sync-v1';
      onopen: (() => void) | null = null;
      onclose: (() => void) | null = null;
      onerror: (() => void) | null = null;
      onmessage: ((e: { data: string }) => void) | null = null;
      constructor() {
        queueMicrotask(() => this.onopen?.());
      }
      close() {
        closed++;
      }
      send() {
        const scope = { version: 1, space: v.space, page: v.page, epoch: '1' };
        const frames = [
          {
            ...scope,
            type: 'catchup',
            streams: [],
            more: true,
            baseline: 'AA',
            // Ciphertext is irrelevant to metadata: no opening/decoder capability.
            baselineObject: { envelopeHash: hash, envelope: 'unopened-ciphertext' },
            membershipHead: {
              revision: '2',
              statementHash: hash,
              ownerKey: c.encodeBinary(f.boot.owner),
              statements: [f.raw[0]],
              more: true,
            },
          },
          {
            ...scope,
            type: 'catchup',
            streams: [],
            more: true,
            membership: { statements: [{ statementHash: hash }], more: false },
          },
          {
            ...scope,
            type: 'chunk',
            statementHash: forged ? c.encodeBinary(new Uint8Array(32)) : hash,
            index: 0,
            count: 1,
            bytes: f.raw[1],
          },
        ];
        const frame = frames[stage++];
        if (stage === 3) expect(records.get(`log:${v.space}`)).toEqual([f.raw[0]]);
        queueMicrotask(() => this.onmessage?.({ data: JSON.stringify(frame) }));
      }
    }
    vi.stubGlobal('WebSocket', Socket);
    if (forged) {
      await expect(managementLog(mount, f.boot, page, f.registration)).rejects.toThrow();
      expect(records.get(`log:${v.space}`)).toEqual([f.raw[0]]);
    } else {
      expect((await managementLog(mount, f.boot, page, f.registration)).length).toBe(2);
      expect(records.get(`log:${v.space}`)).toEqual(f.raw);
    }
    expect(closed).toBe(1);
  }
});
