import { readFileSync } from 'node:fs';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import * as Y from 'yjs';
import { Admission } from '../src/admission.js';
import { Objects } from '../src/objects.js';
import { FrozenAsk } from '../src/ask-intent.js';
import { readThreads } from '../src/thread-records.js';
import { pageAsks } from '../src/live-ask.js';
import type { Registration } from '../src/registration.js';
import type { FoldCommand, FoldResult } from '../src/fold-protocol.js';
import { destination, id, pageLink, selection } from './ask-fixtures.js';
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
const hex = (value: string) =>
  Uint8Array.from(value.match(/../g) ?? [], (part) => parseInt(part, 16));
const json = (value: unknown) => c.text(JSON.stringify(value));
afterEach(() => {
  records.clear();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
async function worker() {
  vi.resetModules();
  let result: FoldResult & { error?: string };
  const host = {
    onmessage: null as unknown as (event: unknown) => Promise<void>,
    postMessage(value: typeof result) {
      result = structuredClone(value);
    },
  };
  vi.stubGlobal('self', host);
  await import('../src/fold.worker.js');
  let request = 0;
  return async (command: FoldCommand) => {
    await host.onmessage({ data: { id: ++request, command } });
    if (result.error) throw new Error(result.error);
    return result;
  };
}

it('retains cut-admitted Ask and discussion history after revocation but never decodes or renders a post-cut record', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_name: string, task: () => unknown) => task() },
  });
  vi.useFakeTimers();
  vi.setSystemTime(50);
  const owner = hex(v.public);
  const a = new Admission(v.space, v.page, '1', owner, { deviceId: v.device } as Registration);
  const shared = c.statement.Envelope.fromJson(json(v.historyCases[0].envelope));
  await a.membership(
    {
      revision: '2',
      statementHash: c.encodeBinary(await shared.hash()),
      ownerKey: c.encodeBinary(owner),
      statements: [c.encodeBinary(json(v.statement)), c.encodeBinary(shared.toJson())],
      more: false,
    },
    true,
  );
  await a.chains([{ deviceId: v.page, chain: c.encodeBinary(json(v.chain)) }]);
  a.root = await crypto.subtle.importKey('raw', hex(v.epochKey), 'HKDF', false, ['deriveBits']);
  const signer = await crypto.subtle.importKey(
    'pkcs8',
    c.concat(hex('302e020100300506032b657004220420'), hex(v.seed)),
    'Ed25519',
    false,
    ['sign'],
  );
  const capture = (operationId: string) =>
    FrozenAsk.capture(
      {
        ...selection(),
        space: v.space,
        page: v.page,
        senderDevice: v.page,
        url: pageLink(v.space, v.page),
      },
      destination(),
      { issuedAt: 50, operationId },
    );
  const preview = capture(id(81)),
    signed = await preview.signed(signer);
  const doc = new Y.Doc();
  doc.getMap('intents').set(signed.operationId, {
    version: 1,
    kind: 'ask',
    signed,
    agentName: 'Historical agent',
    deviceName: 'Revoked browser',
  });
  doc.getMap('messages').set(`${signed.operationId}:1`, {
    version: 1,
    kind: 'ask-state',
    operationId: signed.operationId,
    revision: '1',
    state: 'accepted',
    requestId: `req_${id(8)}`,
    reason: null,
  });
  doc.getMap('replies').set(signed.operationId, {
    version: 1,
    kind: 'ask-reply',
    operationId: signed.operationId,
    requestId: `req_${id(8)}`,
    agentId: destination().agent,
    body: 'Historical final',
  });
  const discussion = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/discussion-v1.json', import.meta.url), 'utf8'),
  );
  const thread = { ...discussion.thread, spaceId: v.space, pageId: v.page, senderDevice: v.page };
  const comment = {
    ...discussion.comment,
    spaceId: v.space,
    pageId: v.page,
    senderDevice: v.page,
    thread: { writer: v.page, id: thread.threadId },
  };
  doc.getMap('threads').set(`${thread.threadId}:1`, thread);
  doc.getMap('messages').set(`${comment.messageId}:1`, comment);
  const scope = { spaceId: v.space, pageId: v.page, epoch: '1' };
  const context: c.Context = {
    space: v.space,
    page: v.page,
    epoch: '1',
    authorDevice: v.page,
    namespace: 'own',
    kind: 'update',
    membershipRevision: '1',
    streamSeq: '1',
    prevHash: new Uint8Array(32),
  };
  const first = await c.Envelope.seal(context, a.root, signer, Y.encodeStateAsUpdate(doc));
  const entry = async (envelope: c.Envelope) => ({
    seq: c.decodeHeader(envelope.header()).context.streamSeq,
    envelopeHash: c.encodeBinary(await envelope.hash()),
    envelope: c.encodeBinary(envelope.toJson()),
  });
  const prior = new Objects(a);
  const admitted = (await prior.admit(v.page, await entry(first)))!;
  const run = await worker();
  const before = await run({ type: 'apply', updates: [], own: [admitted] });
  expect(await pageAsks(before.own, a, (writer) => prior.ownSigningKey(writer))).toMatchObject([
    { agentName: 'Historical agent', reply: 'Historical final' },
  ]);
  const cuts = [
    {
      pageId: v.page,
      epoch: '1',
      namespace: 'own',
      cut: c.encodeBinary(
        c.streamCut.input({
          streamId: v.page,
          namespace: 'own',
          checkpointHash: null,
          checkpointSeq: '0',
          tailHeadSeq: '1',
          tailHeadHash: await first.hash(),
        }),
      ),
    },
  ];
  const body = json({ deviceId: v.page, cuts });
  const input = c.statement.input({
    space: v.space,
    operation: 'device.revoke',
    revision: '3',
    previousHash: a.head!.hash,
    payloadDigest: await c.digest(body),
  });
  const revoke = c.statement.Envelope.fromJson(
    json({
      statement: c.encodeBinary(input),
      payload: c.encodeBinary(body),
      signature: c.encodeBinary(await c.sign(signer, input)),
    }),
  );
  await a.membership(
    {
      revision: '3',
      statementHash: c.encodeBinary(await revoke.hash()),
      ownerKey: c.encodeBinary(owner),
      statements: [c.encodeBinary(revoke.toJson())],
      more: false,
    },
    true,
  );
  expect(() => a.author(v.page, '3')).toThrow();
  // A fresh reader also admits the authenticated historical cut, rather than
  // relying on a key retained before the revocation.
  const after = new Objects(a);
  expect(after.ownSigningKey(v.page)).toBeUndefined();
  const historical = (await after.admit(v.page, await entry(first)))!;
  after.finish();
  const projection = await run({ type: 'apply', updates: [], own: [historical] });
  expect(await pageAsks(projection.own, a, (writer) => after.ownSigningKey(writer))).toMatchObject([
    { operationId: id(81), reply: 'Historical final', canTrack: false },
  ]);
  expect(
    readThreads(projection.own, scope, (writer) => after.ownSigningKey(writer))[0].comments[0].body,
  ).toBe(comment.body);
  const retained = after.ownSigningKey(v.page)!;
  retained.fill(0);
  expect(after.ownSigningKey(v.page)).toEqual(owner);
  const later = await capture(id(82)).signed(signer);
  doc.getMap('intents').set(later.operationId, {
    version: 1,
    kind: 'ask',
    signed: later,
    agentName: 'Late agent',
    deviceName: 'Revoked browser',
  });
  doc
    .getMap('messages')
    .set(`${comment.messageId}:2`, { ...comment, revision: '2', body: 'Post-cut discussion' });
  const tail = await c.Envelope.seal(
    { ...context, streamSeq: '2', prevHash: await first.hash() },
    a.root,
    signer,
    Y.encodeStateAsUpdate(doc),
  );
  await expect(after.admit(v.page, await entry(tail))).rejects.toThrow();
  const unchanged = await run({ type: 'apply', updates: [] });
  expect(unchanged.own).toEqual(projection.own);
  expect(
    (await pageAsks(unchanged.own, a, (writer) => after.ownSigningKey(writer))).map(
      (view) => view.operationId,
    ),
  ).toEqual([id(81)]);
  expect(
    readThreads(unchanged.own, scope, (writer) => after.ownSigningKey(writer))[0].comments[0].body,
  ).toBe(comment.body);
  doc.destroy();
});
