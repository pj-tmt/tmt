import { readFileSync } from 'node:fs';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import * as Y from 'yjs';
import { Admission } from '../src/admission.js';
import { Objects } from '../src/objects.js';
import { attachmentHistory } from '../src/attachment-history.js';
import { AttachmentObjectChannel, FrozenAttachmentUpload } from '../src/attachment-channel.js';
import {
  AdmittedAttachmentRead,
  prepareAttachmentPublication,
  type AttachmentReadOwner,
} from '../src/attachments.js';
import type { Registration } from '../src/registration.js';
import type { PageView } from '../src/transport.js';
import type { FoldCommand } from '../src/fold-protocol.js';
const records = new Map<string, unknown>();
vi.mock('../src/storage.js', () => ({
  record: async (key: string, ...values: unknown[]) =>
    values.length ? records.set(key, values[0]) : records.get(key),
}));
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const hex = (value: Uint8Array) => [...value].map((b) => b.toString(16).padStart(2, '0')).join('');
const bytes = (s: string) => Uint8Array.from(s.match(/../g)!, (b) => parseInt(b, 16));
const encoded = (value: unknown) => c.encodeBinary(c.text(JSON.stringify(value)));
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
async function worker() {
  const surface = {
    onmessage: null as unknown as (event: {
      data: { id: number; command: FoldCommand };
    }) => Promise<void>,
    postMessage: vi.fn(),
  };
  vi.stubGlobal('self', surface);
  vi.resetModules();
  await import('../src/fold.worker.js');
  return async (command: FoldCommand) => {
    surface.postMessage.mockClear();
    await surface.onmessage({ data: { id: 1, command } });
    const result = surface.postMessage.mock.calls[0][0];
    if (result.error) throw new Error(result.error);
    return result;
  };
}
async function fixture(expiredCreator = false) {
  const creator = expiredCreator ? '00000000-0000-4000-8000-000000000094' : v.device;
  records.clear();
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, fn: () => unknown) => fn() },
  });
  vi.spyOn(Date, 'now').mockReturnValue(50);
  const signer = await crypto.subtle.importKey(
    'pkcs8',
    c.concat(bytes('302e020100300506032b657004220420'), bytes(v.seed)),
    'Ed25519',
    false,
    ['sign'],
  );
  const encKey = await crypto.subtle.importKey(
    'pkcs8',
    c.concat(bytes('302e020100300506032b656e04220420'), bytes(v.recipientSeed)),
    'X25519',
    false,
    ['deriveBits'],
  );
  const wrap = c.wrap.Envelope.fromJson(c.text(JSON.stringify(v.wrap))),
    enc = await c.RecipientKey.fromHandle(encKey, wrap.header().recipientKey),
    original = c.certificate.Chain.fromJson(c.text(JSON.stringify(v.chain))),
    certificate = c.certificate.input({
      ...original.certificate(),
      deviceId: creator,
      expiresAt: expiredCreator ? 100 : original.certificate().expiresAt,
      encryptionKey: enc.publicKey(),
    }),
    chainBytes = c.text(
      JSON.stringify({
        version: 1,
        issuerStatement: v.chain.issuerStatement,
        deviceCertificate: c.encodeBinary(certificate),
        issuerSignature: c.encodeBinary(await c.sign(signer, certificate)),
      }),
    ),
    chain = c.certificate.Chain.fromJson(chainBytes);
  const callerCertificate = c.certificate.input({
      ...original.certificate(),
      deviceId: v.device,
      expiresAt: 10000,
      encryptionKey: enc.publicKey(),
    }),
    callerBytes = c.text(
      JSON.stringify({
        version: 1,
        issuerStatement: v.chain.issuerStatement,
        deviceCertificate: c.encodeBinary(callerCertificate),
        issuerSignature: c.encodeBinary(await c.sign(signer, callerCertificate)),
      }),
    );
  const registration = {
    deviceId: v.device,
    chain: expiredCreator ? c.certificate.Chain.fromJson(callerBytes) : chain,
    keys: { enc },
  } as Registration;
  const a = new Admission(v.space, v.page, '1', bytes(v.public), registration);
  await a.membership(
    {
      revision: '1',
      statementHash: c.encodeBinary(bytes(v.statementHash)),
      ownerKey: c.encodeBinary(bytes(v.public)),
      statements: [encoded(v.statement)],
      more: false,
    },
    true,
  );
  const appendStatement = async (operation: string, value: unknown) => {
    const payload = c.text(JSON.stringify(value)),
      input = c.statement.input({
        space: v.space,
        revision: (a.head!.revision + 1n).toString(),
        previousHash: a.head!.hash,
        operation,
        payloadDigest: await c.digest(payload),
      }),
      envelope = c.statement.Envelope.fromJson(
        c.text(
          JSON.stringify({
            statement: c.encodeBinary(input),
            payload: c.encodeBinary(payload),
            signature: c.encodeBinary(await c.sign(signer, input)),
          }),
        ),
      );
    await a.membership(
      {
        revision: (a.head!.revision + 1n).toString(),
        statementHash: c.encodeBinary(await envelope.hash()),
        ownerKey: c.encodeBinary(bytes(v.public)),
        statements: [c.encodeBinary(envelope.toJson())],
        more: false,
      },
      true,
    );
  };
  await appendStatement('page.share', { pageId: v.page, epoch: '1', mode: 'private' });
  await a.chains([
    { deviceId: creator, chain: c.encodeBinary(chainBytes) },
    ...(expiredCreator ? [{ deviceId: v.device, chain: c.encodeBinary(callerBytes) }] : []),
  ]);
  await a.wraps([encoded(v.wrap)]);
  const objects = new Objects(a),
    runFold = await worker();
  let projection: PageView = { source: 'Source', title: 'Title' },
    active = true;
  const owner: AttachmentReadOwner = {
    snapshot: async (epoch = '1') => {
      if (!active || epoch !== '1') throw new Error('Attachment history unavailable');
      return {
        admission: a,
        objects,
        projection: structuredClone(projection),
        revision: await objects.revision(),
      };
    },
  };
  const fold = async (command: FoldCommand) => {
    const result = await runFold(command);
    projection = result;
    return result;
  };
  const content = new Y.Doc();
  content.getText('html').insert(0, 'Source');
  content.getMap('meta').set('title', 'Title');
  // This initial plaintext has no attachment reference; the stream is admitted below.
  const sealed: { seq: string; envelopeHash: string; envelope: string }[] = [];
  let sequence = 0n,
    previous = new Uint8Array(32);
  const admit = async (namespace: 'own' | 'content', update: Uint8Array) => {
    const env = await c.Envelope.seal(
      {
        space: v.space,
        page: v.page,
        epoch: '1',
        kind: 'update',
        namespace,
        authorDevice: creator,
        membershipRevision: '2',
        streamSeq: (++sequence).toString(),
        prevHash: previous,
      },
      a.readRoot('1'),
      signer,
      update,
    );
    const hash = await env.hash();
    const entry = {
      seq: sequence.toString(),
      envelopeHash: c.encodeBinary(hash),
      envelope: c.encodeBinary(env.toJson()),
    };
    sealed.push(entry);
    const result = await objects.admit(creator, entry);
    previous = hash;
    expect(result).not.toBeNull();
    await fold({
      type: 'apply',
      updates: namespace === 'content' ? [result!.update] : [],
      own: namespace === 'own' ? [{ writer: creator, update: result!.update }] : [],
    });
  };
  await admit('content', Y.encodeStateAsUpdate(content));
  const plaintext = c.text('Original attachment'),
    env = await c.Envelope.seal(
      {
        space: v.space,
        page: v.page,
        epoch: '1',
        kind: 'asset',
        namespace: 'content',
        authorDevice: creator,
        membershipRevision: '2',
        streamSeq: '0',
        prevHash: new Uint8Array(32),
      },
      a.readRoot('1'),
      signer,
      plaintext,
    ),
    raw = env.toJson();
  const d = c.attachment.attachmentDescriptor({
    version: 1,
    attachmentId: '00000000-0000-4000-8000-000000000041',
    space: v.space,
    page: v.page,
    epoch: '1',
    namespace: 'content',
    objectId: c.decodeHeader(env.header()).objectId,
    authorDevice: creator,
    membershipRevision: '2',
    source: { kind: 'document', sourceDigest: hex(await c.digest(c.text('Source'))) },
    envelopeHash: hex(await env.hash()),
    signature: c.encodeBinary(env.signature()),
    payloadSha256: hex(await c.digest(raw)),
    payloadBytes: String(raw.length),
    plaintextBytes: String(plaintext.length),
    filename: 'original.txt',
    mediaType: 'text/plain',
  });
  const deadline = () => performance.now() + 5000;
  let available = true,
    mutation: (() => Promise<void>) | undefined;
  const backend = {
    readCommitted: async (namespace: Uint8Array, key: Uint8Array) => {
      expect(hex(namespace)).toHaveLength(64);
      expect(hex(key)).toBe(d.objectId);
      if (!available) throw new Error('Pending object');
      await mutation?.();
      return raw.slice();
    },
  };
  const publish = async (withProof = true) => {
    // A different creator's historical record is independently signed and
    // cut-admitted below; it cannot use this actual caller's fresh preparation.
    const proof = expiredCreator
      ? {
          root: 'intents' as const,
          key: d.attachmentId,
          value: c.attachment.attachmentPublication({
            version: 1,
            kind: 'attachment-publication',
            spaceId: d.space,
            pageId: d.page,
            epoch: d.epoch,
            senderDevice: creator,
            membershipRevision: d.membershipRevision,
            attachmentId: d.attachmentId,
            descriptorHash: hex(await c.attachment.attachmentHash(d)),
            source: d.source,
            baseRevision: await objects.revision(),
          }),
        }
      : await prepareAttachmentPublication(
          owner,
          d,
          await objects.revision(),
          backend,
          deadline(),
          'private',
        );
    const own = new Y.Doc();
    if (withProof) own.getMap(proof.root).set(proof.key, proof.value);
    const comment = {
      version: 1,
      kind: 'comment',
      spaceId: v.space,
      pageId: v.page,
      epoch: '1',
      senderDevice: creator,
      revision: '1',
      deleted: false,
      deviceName: 'Browser',
      at: '50',
      messageId: '00000000-0000-4000-8000-000000000043',
      thread: { writer: creator, id: creator },
      body: 'Chat/annotation',
      attachments: [d],
    };
    own.getMap('messages').set(`${comment.messageId}:1`, comment);
    // An ordinary historical comment exercises the broader fold parity, without
    // any attachment descriptor or attachment read owner.
    const plainComment = {
      ...comment,
      messageId: '00000000-0000-4000-8000-000000000095',
      body: 'Plain historical comment',
    };
    delete (plainComment as Partial<typeof comment>).attachments;
    own.getMap('messages').set(`${plainComment.messageId}:1`, plainComment);
    await admit('own', Y.encodeStateAsUpdate(own));
    const vector = Y.encodeStateVector(content);
    content.getMap('meta').set('attachments', [d]);
    await admit('content', Y.encodeStateAsUpdate(content, vector));
  };
  const selector = async (): Promise<c.attachment.AttachmentSelector> => ({
    kind: 'document-current',
    attachmentId: d.attachmentId,
    descriptorHash: hex(await c.attachment.attachmentHash(d)),
    contentRevision: await objects.revision(),
  });
  return {
    a,
    creator,
    creatorChain: c.encodeBinary(chainBytes),
    sealed,
    objects,
    owner,
    backend,
    d,
    raw,
    plaintext,
    deadline,
    selector,
    publish,
    appendStatement,
    admit,
    fold,
    unavailable: () => {
      available = false;
    },
    race: (fn: () => Promise<void>) => {
      mutation = fn;
    },
    close: () => {
      active = false;
      a.closeKeys();
    },
  };
}
it('fences publication and reads exact authenticated document/Chat/annotation references', async () => {
  const f = await fixture();
  await expect(
    AdmittedAttachmentRead.capture(f.owner, await f.selector(), f.deadline(), 'private'),
  ).rejects.toThrow();
  await f.publish();
  const read = await AdmittedAttachmentRead.capture(
    f.owner,
    await f.selector(),
    f.deadline(),
    'private',
  );
  expect(await read.disclose(f.backend, 'private')).toEqual(f.plaintext);
  const altered = read.descriptor;
  altered.objectId = '01'.repeat(32);
  expect(read.descriptor).toEqual(f.d);
  const message: c.attachment.AttachmentSelector = {
    kind: 'message',
    writerId: v.device,
    messageId: '00000000-0000-4000-8000-000000000043',
    messageRevision: '1',
    attachmentId: f.d.attachmentId,
    descriptorHash: hex(await c.attachment.attachmentHash(f.d)),
  };
  expect(
    await (
      await AdmittedAttachmentRead.capture(f.owner, message, f.deadline(), 'private')
    ).disclose(f.backend, 'private'),
  ).toEqual(f.plaintext);
  await expect(
    AdmittedAttachmentRead.capture(
      f.owner,
      { ...message, messageRevision: '2' },
      f.deadline(),
      'private',
    ),
  ).rejects.toThrow();
  await expect(
    AdmittedAttachmentRead.capture(
      f.owner,
      { ...(await f.selector()), descriptorHash: '00'.repeat(32) },
      f.deadline(),
      'private',
    ),
  ).rejects.toThrow();
  f.unavailable();
  await expect(read.disclose(f.backend, 'private')).rejects.toThrow('Pending');
  f.close();
  await expect(read.disclose(f.backend, 'private')).rejects.toThrow();
});

it('the object adapter verifies complete committed ciphertext before preparing publication', async () => {
  const f = await fixture(),
    base = await f.objects.revision(),
    deadline = f.deadline();
  const original = await FrozenAttachmentUpload.capture(
    f.d,
    base,
    '11111111-1111-4111-8111-111111111111',
    f.raw,
  );
  const methods: string[] = [];
  let damaged = false;
  const channel = new AttachmentObjectChannel(async (request) => {
    methods.push(request.method);
    switch (request.method) {
      case 'begin':
        return { ok: { result: 'pending', nextIndex: 0, received: 0 } };
      case 'part':
        expect(c.binary(request.bytes, 32_768)).toEqual(f.raw);
        return { ok: { result: 'progress', nextIndex: 1, received: f.raw.length } };
      case 'commit':
        return {
          ok: {
            result: 'committed',
            opaqueKey: f.d.objectId,
            payloadSha256: f.d.payloadSha256,
            payloadBytes: f.raw.length,
          },
        };
      case 'verify':
        expect(request).toMatchObject({
          transferId: original.transferId,
          descriptor: original.descriptor,
          base,
        });
        return {
          ok: {
            result: 'read',
            offset: request.offset,
            totalBytes: f.raw.length,
            bytes: c.encodeBinary(
              damaged
                ? new Uint8Array(request.count)
                : f.raw.slice(request.offset, request.offset + request.count),
            ),
          },
        };
      default:
        throw new Error('Unexpected request; no retry or publication call allowed');
    }
  });
  await channel.begin(original, deadline);
  await channel.part(original, 0, deadline);
  await channel.commit(original, deadline);
  const record = await prepareAttachmentPublication(
    f.owner,
    f.d,
    base,
    channel.verifier(original),
    deadline,
    'private',
  );
  expect(record).toMatchObject({ root: 'intents', key: f.d.attachmentId });
  expect(await f.objects.revision()).toBe(base);
  expect(methods).toEqual(['begin', 'part', 'commit', 'verify']);
  damaged = true;
  await expect(
    prepareAttachmentPublication(
      f.owner,
      f.d,
      base,
      channel.verifier(original),
      deadline,
      'private',
    ),
  ).rejects.toThrow();
  expect(methods).toEqual(['begin', 'part', 'commit', 'verify', 'verify']);
  expect(await f.objects.revision()).toBe(base);
});
it('keeps archive reads, rejects archive publication, deletion and head-change races', async () => {
  const f = await fixture();
  await f.publish();
  const read = await AdmittedAttachmentRead.capture(
    f.owner,
    await f.selector(),
    f.deadline(),
    'private',
  );
  f.race(async () => {
    await f.appendStatement('page.archive', { pageId: v.page });
  });
  await expect(read.disclose(f.backend, 'private')).rejects.toThrow();
  f.race(async () => {});
  const archived = await AdmittedAttachmentRead.capture(
    f.owner,
    await f.selector(),
    f.deadline(),
    'private',
  );
  expect(await archived.disclose(f.backend, 'private')).toEqual(f.plaintext);
  await expect(
    prepareAttachmentPublication(
      f.owner,
      f.d,
      await f.objects.revision(),
      f.backend,
      f.deadline(),
      'private',
    ),
  ).rejects.toThrow('Page unavailable');
  await f.appendStatement('page.delete', { pageId: v.page });
  await expect(archived.disclose(f.backend, 'private')).rejects.toThrow();
});
it('refuses incomplete committed evidence before producing any publication', async () => {
  const f = await fixture();
  f.unavailable();
  await expect(f.publish()).rejects.toThrow('Pending');
  await expect(
    AdmittedAttachmentRead.capture(f.owner, await f.selector(), f.deadline(), 'private'),
  ).rejects.toThrow();
});

it('requires the original cut-admitted creation proof and one unexpired read budget', async () => {
  const f = await fixture();
  await f.publish(false);
  await expect(
    AdmittedAttachmentRead.capture(f.owner, await f.selector(), f.deadline(), 'private'),
  ).rejects.toThrow();
  const positive = await fixture();
  await positive.publish();
  const read = await AdmittedAttachmentRead.capture(
    positive.owner,
    await positive.selector(),
    positive.deadline(),
    'private',
  );
  expect(await read.disclose(positive.backend, 'private')).toEqual(positive.plaintext);
  await expect(
    AdmittedAttachmentRead.capture(
      positive.owner,
      await positive.selector(),
      performance.now() - 1,
      'private',
    ),
  ).rejects.toThrow();
  await expect(
    prepareAttachmentPublication(
      positive.owner,
      positive.d,
      await positive.objects.revision(),
      positive.backend,
      performance.now() - 1,
      'private',
    ),
  ).rejects.toThrow();
});

it('admits expired original attachment/comment creators while refusing an expired actual caller', async () => {
  const f = await fixture(true);
  await f.publish();
  vi.spyOn(Date, 'now').mockReturnValue(200);
  await expect(f.a.chains([{ deviceId: f.creator, chain: f.creatorChain }])).resolves.toHaveLength(
    1,
  );
  const objects = new Objects(f.a),
    updates: Uint8Array[] = [],
    own: { writer: string; update: Uint8Array }[] = [];
  for (const entry of f.sealed) {
    const admitted = await objects.admit(f.creator, entry);
    if (admitted!.namespace === 'own') own.push({ writer: f.creator, update: admitted!.update });
    else updates.push(admitted!.update);
  }
  expect(objects.statusWriter(f.creator)).toBe(true);
  const replay = await worker(),
    view = await replay({ type: 'apply', updates, own });
  const owner: AttachmentReadOwner = {
    snapshot: async () => ({
      ...(await f.owner.snapshot()),
      objects,
      projection: structuredClone(view),
      revision: await objects.revision(),
    }),
  };
  const ordinary = view.own[f.creator].messages['00000000-0000-4000-8000-000000000095:1'];
  expect(ordinary.body).toBe('Plain historical comment');
  expect(ordinary).not.toHaveProperty('attachments');
  const read = await AdmittedAttachmentRead.capture(
    owner,
    await f.selector(),
    f.deadline(),
    'private',
  );
  expect(await read.disclose(f.backend, 'private')).toEqual(f.plaintext);
  expect(() => f.a.author(f.creator, f.a.head!.revision.toString())).toThrow();
  vi.spyOn(Date, 'now').mockReturnValue(10000);
  expect(() => f.a.validateRead('private')).toThrow();
  await expect(read.disclose(f.backend, 'private')).rejects.toThrow();
});

it('historical ciphertext uses a detached actual Worker and never substitutes or mutates the live projection', async () => {
  const f = await fixture();
  await f.publish();
  const revision = (f.a.head!.revision + 1n).toString();
  await f.appendStatement('epoch.advance', {
    pageId: v.page,
    epoch: '2',
    cuts: [],
    wraps: [],
    baseline: {
      pageId: v.page,
      epoch: '2',
      sourceDigest: c.encodeBinary(new Uint8Array(32)),
      baselineCommitment: c.encodeBinary(new Uint8Array(32)),
      title: 'Live',
      objectEnvelopeHash: c.encodeBinary(new Uint8Array(32)),
      membershipRevision: revision,
    },
  });
  const a = new Admission(v.space, v.page, '2', bytes(v.public), f.a.registration);
  await a.membership(
    {
      revision,
      statementHash: c.encodeBinary(f.a.head!.hash),
      ownerKey: c.encodeBinary(bytes(v.public)),
      statements: records.get(`log:${v.space}`),
      more: false,
    },
    true,
  );
  await a.chains([{ deviceId: f.creator, chain: f.creatorChain }]);
  await a.wraps([encoded(v.wrap)]);
  const liveObjects = new Objects(a);
  const live = {
    admission: a,
    objects: liveObjects,
    projection: { source: 'Live source', title: 'Live title' },
    revision: await liveObjects.revision(),
  };
  const originalKey = a.readRoot('1'),
    head = a.head;
  const frame = (fields: Record<string, unknown>) =>
    JSON.stringify({
      version: 1,
      type: 'catchup',
      space: v.space,
      page: v.page,
      epoch: '1',
      streams: [],
      more: true,
      ...fields,
    });
  const raw = [
    frame({
      baseline: null,
      membershipHead: {
        revision,
        statementHash: c.encodeBinary(a.head!.hash),
        ownerKey: c.encodeBinary(bytes(v.public)),
        statements: [],
        more: false,
      },
    }),
    ...f.sealed.map((entry) =>
      frame({
        streams: [
          {
            streamId: f.creator,
            namespace: c.decodeHeader(
              c.Envelope.fromJson(c.binary(entry.envelope, c.MAX_ENVELOPE_JSON)).header(),
            ).context.namespace,
            checkpoint: null,
            tail: [entry],
          },
        ],
      }),
    ),
    frame({ more: false }),
  ];
  let released = false;
  const source = {
    async *frames(epoch: string, _deadline: number) {
      expect(epoch).toBe('1');
      try {
        for (const value of raw) yield value;
      } finally {
        released = true;
      }
    },
  };
  const run = await worker();
  const detached = {
    onmessage: null as unknown as (event: { data: unknown }) => void,
    onerror: null,
    terminate: vi.fn(),
    postMessage: async (job: { id: number; command: FoldCommand }) => {
      const result = await run(job.command);
      detached.onmessage({ data: { ...result, id: job.id } });
    },
  };
  const historical = await attachmentHistory(
    async () => live,
    source,
    '1',
    'private',
    f.deadline(),
    detached as unknown as Worker,
  );
  expect(historical.projection.source).toBe('Source');
  expect(historical.projection.attachments).toEqual([f.d]);
  expect(
    historical.projection.own![f.creator].messages['00000000-0000-4000-8000-000000000095:1'],
  ).toMatchObject({ body: 'Plain historical comment' });
  expect(live.projection).toEqual({ source: 'Live source', title: 'Live title' });
  expect(a.head).toEqual(head);
  expect(a.readRoot('1')).toBe(originalKey);
  expect(released).toBe(true);
  expect(detached.terminate).toHaveBeenCalledOnce();
});
