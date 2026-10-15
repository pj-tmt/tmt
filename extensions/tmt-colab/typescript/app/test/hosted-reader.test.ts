import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import type {
  SealedDocument,
  SealedSyncPort,
  SyncCall,
  SyncQueryRequest,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import { Admission } from '../src/admission.js';
import type { Registration } from '../src/registration.js';
import type { Fold } from '../src/fold.js';
import type { FoldCommand } from '../src/fold-protocol.js';
import { HostedPageReader } from '../src/hosted-reader.js';

vi.mock('../src/storage.js', () => ({
  record: () => {
    throw new Error('Reader must not persist authority');
  },
}));
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const hex = (value: string) =>
  Uint8Array.from(value.match(/../g) ?? [], (byte) => parseInt(byte, 16));
const hash = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
const scope = { space: v.space as string, page: v.page as string, epoch: '1' };
const call = (): SyncCall => ({
  signal: new AbortController().signal,
  deadlineAtMs: Date.now() + 30_000,
});
function document(
  resource: string,
  id: string,
  values: Record<string, string | number>,
): SealedDocument {
  const fields = Object.fromEntries(
    Object.entries({
      ...scope,
      epoch: 1,
      uid: 'owner-uid',
      expiresAt: '2027-01-01T00:00:00Z',
      ...values,
    })
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
      .map(([key, value]) => [
        key,
        typeof value === 'number'
          ? { integerValue: String(value) }
          : key === 'expiresAt'
            ? { timestampValue: value }
            : { stringValue: value },
      ]),
  );
  const bytes = c.text(JSON.stringify(fields));
  return { target: { resource, id }, bytes, digest: hash(bytes) };
}
const base = `${scope.space}_${scope.page}_1`;
function authority(kind: string, bytes: Uint8Array, recipient = '') {
  const subject = hash(bytes),
    count = Math.ceil(bytes.length / 32768);
  return Array.from({ length: count }, (_, index) =>
    document('authority', `${base}_${kind}_${subject}_${index + 1}`, {
      kind,
      subject,
      seq: index + 1,
      count,
      recipient,
      bytes: c.encodeBinary(bytes.slice(index * 32768, (index + 1) * 32768)),
    }),
  );
}
async function fixture() {
  const signer = await crypto.subtle.importKey(
    'pkcs8',
    c.concat(hex('302e020100300506032b657004220420'), hex(v.seed)),
    'Ed25519',
    false,
    ['sign'],
  );
  const statements = [c.text(JSON.stringify(v.statement))];
  async function append(operation: string, value: unknown) {
    const previous = c.statement.Envelope.fromJson(statements.at(-1)!);
    const payload = c.text(JSON.stringify(value));
    const input = c.statement.input({
      space: scope.space,
      operation,
      revision: String(statements.length + 1),
      previousHash: await previous.hash(),
      payloadDigest: await c.digest(payload),
    });
    statements.push(
      c.text(
        JSON.stringify({
          statement: c.encodeBinary(input),
          payload: c.encodeBinary(payload),
          signature: c.encodeBinary(await c.sign(signer, input)),
        }),
      ),
    );
  }
  await append('page.share', { pageId: scope.page, mode: 'link', epoch: '1' });
  await append('link.add', {
    linkId: v.linkId,
    role: 'viewer',
    linkSignKey: c.encodeBinary(hex(v.public)),
    linkEncKey: c.encodeBinary(
      c.wrap.Envelope.fromJson(c.text(JSON.stringify(v.wrap))).header().recipientKey,
    ),
    pages: [scope.page],
  });
  const admission = new Admission(scope.space, scope.page, '1', hex(v.public), {} as Registration, {
    linkId: v.linkId,
    principal: 'holder-uid',
    expiresAt: Number.MAX_SAFE_INTEGER,
  });
  // The read adapter receives an Admission whose key was already opened by the wrap verifier.
  admission.root = await crypto.subtle.importKey('raw', hex(v.epochKey), 'HKDF', false, [
    'deriveBits',
  ]);
  const rows = statements.flatMap((bytes) => authority('membership', bytes));
  rows.push(...authority('chain', c.text(JSON.stringify(v.chain))));
  const checkpoints: SealedDocument[] = [],
    logs: SealedDocument[] = [];
  const requests: SyncQueryRequest[] = [];
  const port = {
    context: {
      resources: ['log', 'checkpoints', 'authority'].map((name) => ({
        name,
        sync: {
          query: {
            seqField: 'seq',
            discovery: {
              name: name === 'log' ? 'writers' : name === 'checkpoints' ? 'objects' : 'records',
              seqEquals: null,
            },
          },
        },
      })),
    },
    async query(request: SyncQueryRequest) {
      requests.push(structuredClone(request));
      const { query } = request;
      if (request.shape === 'discovery') {
        const kind =
          'kind' in query.scope ? (query.scope.kind as { stringValue: string }).stringValue : '';
        const found =
          query.resource === 'authority'
            ? rows.filter((row) => JSON.parse(c.decodeText(row.bytes)).kind.stringValue === kind)
            : query.resource === 'log'
              ? logs
              : checkpoints;
        return { shape: 'discovery', page: { documents: found } };
      }
      if (!('stream' in query)) throw new Error('Wrong query');
      const found = (query.resource === 'log' ? logs : checkpoints)
        .filter((row) => {
          const fields = JSON.parse(c.decodeText(row.bytes));
          return (
            fields[query.resource === 'log' ? 'stream' : 'object'].stringValue === query.stream &&
            Number(fields.seq.integerValue) > (query.cursor?.seq ?? 0)
          );
        })
        .sort(
          (a, b) =>
            Number(JSON.parse(c.decodeText(a.bytes)).seq.integerValue) -
            Number(JSON.parse(c.decodeText(b.bytes)).seq.integerValue),
        )
        .slice(0, 1);
      return {
        shape: 'stream',
        page: {
          documents: found,
          ...(found.length
            ? {
                cursor: {
                  stream: query.stream,
                  seq: Number(JSON.parse(c.decodeText(found[0].bytes)).seq.integerValue),
                },
              }
            : {}),
        },
      };
    },
  } as unknown as SealedSyncPort;
  const folded: FoldCommand[] = [];
  const fold = {
    async run(command: FoldCommand) {
      folded.push(structuredClone(command));
      return { source: 'verified page', title: 'Title', own: {}, update: new Uint8Array() };
    },
    close: vi.fn(),
  } as Pick<Fold, 'run' | 'close'>;
  const reader = new HostedPageReader(
    port,
    admission,
    { revision: '1', statement: hex(v.statementHash) },
    new Map(
      ['log', 'checkpoints', 'authority'].map((name) => [
        name,
        { documents: 1024, bytes: 4 * 1024 * 1024 },
      ]),
    ),
    fold,
  );
  const context: c.Context = {
    ...scope,
    kind: 'update',
    namespace: 'content',
    authorDevice: v.page,
    streamSeq: '1',
    membershipRevision: '1',
    prevHash: new Uint8Array(32),
  };
  async function seal(change: Partial<c.Context> = {}, author = signer) {
    return c.Envelope.seal(
      { ...context, ...change },
      hex(v.epochKey),
      author,
      c.text('authenticated update'),
    );
  }
  async function log(envelope: c.Envelope) {
    const header = c.decodeHeader(envelope.header()).context;
    const row = document('log', `${base}_${header.authorDevice}_${header.streamSeq}`, {
      stream: header.authorDevice,
      seq: Number(header.streamSeq),
      hash: c.encodeBinary(await envelope.hash()),
      envelope: c.decodeText(envelope.toJson()),
    });
    logs.push(row);
    return row;
  }
  async function checkpoint(envelope: c.Envelope) {
    const object = c.decodeHeader(envelope.header()).objectId,
      bytes = envelope.toJson(),
      count = Math.ceil(bytes.length / 32768);
    checkpoints.push(
      ...Array.from({ length: count }, (_, index) =>
        document('checkpoints', `${base}_${object}_${index}`, {
          object,
          index,
          seq: index + 1,
          count,
          bytes: c.encodeBinary(bytes.slice(index * 32768, (index + 1) * 32768)),
        }),
      ),
    );
  }
  async function writer(deviceId: string) {
    const previous = c.certificate.Chain.fromJson(c.text(JSON.stringify(v.chain))),
      input = c.certificate.input({ ...previous.certificate(), deviceId });
    const raw = c.text(
      JSON.stringify({
        ...v.chain,
        deviceCertificate: c.encodeBinary(input),
        issuerSignature: c.encodeBinary(await c.sign(signer, input)),
      }),
    );
    rows.push(...authority('chain', raw));
  }
  async function linkWriter() {
    const seed = new Uint8Array(32).fill(23),
      linkId = '00000000-0000-4000-8000-000000000777',
      keys = await c.link.deriveLink(seed, scope.space, linkId),
      device = await c.link.deriveDevice(seed, scope.space, linkId);
    await append('link.add', {
      linkId,
      role: 'viewer',
      linkSignKey: c.encodeBinary(keys.signingPublic),
      linkEncKey: c.encodeBinary(keys.encryption.publicKey()),
      pages: [scope.page],
    });
    const issuer = c.statement.Envelope.fromJson(statements.at(-1)!);
    rows.push(...authority('membership', statements.at(-1)!));
    const bytes = await c.link.certifyDevice(keys, {
      deviceId: device.id,
      signingPublic: device.signingPublic,
      encryptionPublic: keys.encryption.publicKey(),
      membershipRevision: String(statements.length),
      issuerStatement: await issuer.hash(),
      issuedAt: c.link.DEVICE_ISSUED_AT,
      expiresAt: c.link.DEVICE_EXPIRES_AT,
    });
    const chain = c.certificate.Chain.fromJson(bytes);
    await chain.verify(await issuer.hash(), chain.certificate(), keys.signingPublic);
    rows.push(...authority('chain', bytes));
    return device;
  }
  return {
    reader,
    admission,
    requests,
    folded,
    fold,
    seal,
    log,
    logs,
    checkpoint,
    append,
    rows,
    statements,
    writer,
    linkWriter,
  };
}

it('feeds exact signed authority and contiguous cross-namespace updates to the existing verifiers before folding', async () => {
  const f = await fixture(),
    one = await f.seal();
  await f.log(one);
  await f.log(await f.seal({ namespace: 'own', streamSeq: '2', prevHash: await one.hash() }));
  expect(
    await f.reader.refresh(call(), new Set(['authority', 'log', 'checkpoints'])),
  ).toMatchObject({ source: 'verified page', ownData: true });
  expect(f.folded).toEqual([
    {
      type: 'apply',
      updates: [c.text('authenticated update')],
      own: [{ writer: v.page, update: c.text('authenticated update') }],
    },
  ]);
  expect(
    f.requests.filter((request) => request.shape === 'stream').map((request) => request.query),
  ).toHaveLength(3);
  f.reader.close();
  expect(f.admission.root).toBeNull();
  expect(f.fold.close).toHaveBeenCalledOnce();
});
it('uses its admitted stream position on later wakes and avoids unrelated authority discovery', async () => {
  const f = await fixture(),
    one = await f.seal();
  await f.log(one);
  await f.reader.refresh(call(), new Set(['authority', 'log', 'checkpoints']));
  f.requests.length = 0;
  f.folded.length = 0;
  await f.log(await f.seal({ streamSeq: '2', prevHash: await one.hash() }));
  await f.reader.refresh(call(), new Set(['log']));
  expect(f.requests.some((request) => request.query.resource === 'authority')).toBe(false);
  expect(f.requests.find((request) => request.shape === 'stream')?.query).toMatchObject({
    cursor: { stream: v.page, seq: 1 },
  });
  expect(f.folded).toHaveLength(1);
  expect(f.folded[0]).toMatchObject({ updates: [c.text('authenticated update')] });
  f.reader.close();
});
it('loads a complete checkpoint pair then verifies the tail from their shared prefix', async () => {
  const f = await fixture(),
    prefix = new Uint8Array(32).fill(7);
  await f.linkWriter();
  await f.checkpoint(await f.seal({ kind: 'checkpoint', streamSeq: '4', prevHash: prefix }));
  await f.checkpoint(
    await f.seal({ kind: 'checkpoint', namespace: 'own', streamSeq: '4', prevHash: prefix }),
  );
  await f.log(await f.seal({ streamSeq: '5', prevHash: prefix }));
  await f.reader.refresh(call(), new Set(['authority', 'log', 'checkpoints']));
  expect(f.folded.map((command) => command.type)).toEqual(['checkpoint', 'checkpoint', 'apply']);
  expect(
    f.requests.find((request) => request.shape === 'stream' && request.query.resource === 'log')
      ?.query,
  ).toMatchObject({ cursor: { seq: 4 } });
  f.reader.close();
});
it('does not decode or expose a partial checkpoint pair or a forged update', async () => {
  for (const mode of ['partial', 'forged']) {
    const f = await fixture();
    if (mode === 'partial')
      await f.checkpoint(await f.seal({ kind: 'checkpoint', streamSeq: '4' }));
    else {
      const envelope = await f.seal(),
        value = JSON.parse(c.decodeText(envelope.toJson()));
      value.signature = c.encodeBinary(new Uint8Array(64));
      await f.log(c.Envelope.fromJson(c.text(JSON.stringify(value))));
    }
    await expect(
      f.reader.refresh(call(), new Set(['authority', 'log', 'checkpoints'])),
    ).rejects.toBeDefined();
    expect(f.folded).toEqual([]);
    f.reader.close();
  }
});
it('a missing sequence requires a fresh catch-up, without folding the successful prefix', async () => {
  const f = await fixture();
  await f.log(await f.seal());
  await f.log(await f.seal({ streamSeq: '3' }));
  await expect(
    f.reader.refresh(call(), new Set(['authority', 'log', 'checkpoints'])),
  ).rejects.toMatchObject({ state: 'catch-up-required' });
  expect(f.folded).toEqual([]);
  f.reader.close();
});
it('stops before reading or decoding when the caller is cancelled or the reader is closed', async () => {
  const f = await fixture();
  await expect(
    f.reader.refresh({ ...call(), signal: AbortSignal.abort() }, new Set(['log'])),
  ).rejects.toMatchObject({ state: 'cancelled' });
  expect(f.requests).toEqual([]);
  f.reader.close();
  await expect(f.reader.refresh(call(), new Set(['log']))).rejects.toMatchObject({
    state: 'cancelled',
  });
  expect(f.folded).toEqual([]);
});

it('finds a late seq-one writer on an initially empty page without an authority polling timer', async () => {
  const f = await fixture();
  await f.linkWriter();
  await f.reader.refresh(call(), new Set(['authority', 'log', 'checkpoints']));
  f.requests.length = 0;
  const writer = '00000000-0000-4000-8000-000000000456';
  await f.writer(writer);
  await f.log(await f.seal({ authorDevice: writer }));
  await f.reader.refresh(call(), new Set(['log']));
  const kinds = f.requests
    .filter((request) => request.query.resource === 'authority')
    .map((request) => request.query.scope.kind);
  expect(kinds).toEqual([{ stringValue: 'chain' }]);
  expect(f.folded.at(-1)).toMatchObject({ updates: [c.text('authenticated update')] });
  f.reader.close();
});

it('does not substitute an owner or device wrap for the link-addressed epoch key', async () => {
  const f = await fixture();
  f.admission.root = null;
  f.rows.push(...authority('wrap', c.text(JSON.stringify(v.wrap)), `link:${v.linkId}`));
  const verifier = vi.spyOn(f.admission, 'wraps');
  await expect(
    f.reader.refresh(call(), new Set(['authority', 'log', 'checkpoints'])),
  ).rejects.toThrow();
  expect(verifier).toHaveBeenCalledOnce();
  expect(f.admission.root).toBeNull();
  expect(f.folded).toEqual([]);
  f.reader.close();
});

it('reads an owner stream when discovery also includes a valid nonwriting link-device chain', async () => {
  const f = await fixture();
  const link = await f.linkWriter();
  await f.log(await f.seal());
  try {
    expect(
      await f.reader.refresh(call(), new Set(['authority', 'log', 'checkpoints'])),
    ).toMatchObject({ source: 'verified page' });
    expect(f.folded).toEqual([
      { type: 'apply', updates: [c.text('authenticated update')], own: [] },
    ]);
    // The unrelated proof is not installed as an owner-author capability.
    expect(() => f.admission.author(link.id, String(f.statements.length))).toThrow();
  } finally {
    f.reader.close();
  }
});

it('keeps a referenced link-issued writer in strict owner-member admission and refuses it', async () => {
  const f = await fixture();
  const link = await f.linkWriter();
  await f.log(await f.seal({ authorDevice: link.id }, link.sign));
  const admission = vi.spyOn(f.admission, 'chains');
  try {
    await expect(
      f.reader.refresh(call(), new Set(['authority', 'log', 'checkpoints'])),
    ).rejects.toThrow();
    expect(admission).toHaveBeenCalledOnce();
    const supplied = admission.mock.calls[0][0] as { deviceId: string; chain: string }[];
    expect(supplied.some((value) => value.deviceId === link.id)).toBe(true);
    expect(f.folded).toEqual([]);
  } finally {
    f.reader.close();
  }
});

it('refuses missing or forged referenced writer proof even with a valid unrelated link chain', async () => {
  for (const mode of ['missing', 'forged']) {
    const f = await fixture();
    await f.linkWriter();
    await f.log(await f.seal());
    const index = f.rows.findIndex((row) => {
      const fields = JSON.parse(c.decodeText(row.bytes));
      return (
        fields.kind.stringValue === 'chain' &&
        c.certificate.Chain.fromJson(c.binary(fields.bytes.stringValue, 16384)).certificate()
          .deviceId === v.page
      );
    });
    expect(index).toBeGreaterThanOrEqual(0);
    f.rows.splice(index, 1);
    if (mode === 'forged') {
      const forged = { ...v.chain, issuerSignature: c.encodeBinary(new Uint8Array(64)) };
      f.rows.push(...authority('chain', c.text(JSON.stringify(forged))));
    }
    try {
      await expect(
        f.reader.refresh(call(), new Set(['authority', 'log', 'checkpoints'])),
      ).rejects.toThrow();
      expect(f.folded).toEqual([]);
      expect(() => f.admission.author(v.page, String(f.statements.length))).toThrow();
    } finally {
      f.reader.close();
    }
  }
});
