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
import type { OwnAuthor } from '../src/own-seal.js';
import type { Registration } from '../src/registration.js';
import { Fold } from '../src/fold.js';
import { contentBase, updatesOf } from './content-base.js';
import type { DecoderCommand, FoldCommand } from '../src/fold-protocol.js';
import * as budgetMath from '../../../../tmt-remote/typescript/remote-client/src/budget.js';
import { HostedBudget } from '../src/hosted-budget.js';
import { HostedPageReader } from '../src/hosted-reader.js';

vi.mock('../src/storage.js', () => ({
  record: () => {
    throw new Error('Reader must not persist authority');
  },
}));
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
// Public wraps from authority-reference.py::seal_wrap: authority-v1 recipientSeed,
// deterministic ephemeral seeds 01/02*32, epochs/revisions 1/3 and 2/4; no runtime sealer.
const historyWraps = [
  {
    header:
      'AAAAEXRtdC1jb2xhYi13cmFwLXYxAAAAATEAAAAgYmFzZS14MjU1MTktaGtkZnNoYTI1Ni1hZXMyNTZnY20AAAAgdXF2cGdhMjJ2Z2x3cG5ncGc3amQ3enBrNXZ6anRvdWQAAAAkMDAwMDAwMDAtMDAwMC00MDAwLTgwMDAtMDAwMDAwMDAwMDAyAAAAATEAAAAEbGluawAAACQwMDAwMDAwMC0wMDAwLTQwMDAtODAwMC0wMDAwMDAwMDAwNjAAAAAgQw9LmFlmUUWmsbonQCRIe9ZvA6LdV313U8aNfX0AwAwAAAAg11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURoAAAABMwAAAAllcG9jaC1rZXk',
    enc: 'pOCSkrZRwni5dyxWn1-puxPZBrRqtoyd-dwrRAn4ogk',
    ciphertext: 'bPWII5S5c3e3OMFu-4R5qSzJfqkMqe83C7T2PXgy-5nZibm1851-rFyuob4nN0xy',
    signature:
      'YHJpgFXNPV2fTcqsxit1T9n5fcPHAJulYWQJmwbdqcmBong4y3fOisPuF8pkN4S9Bs5TG9yAsdXALC292s2eBg',
  },
  {
    header:
      'AAAAEXRtdC1jb2xhYi13cmFwLXYxAAAAATEAAAAgYmFzZS14MjU1MTktaGtkZnNoYTI1Ni1hZXMyNTZnY20AAAAgdXF2cGdhMjJ2Z2x3cG5ncGc3amQ3enBrNXZ6anRvdWQAAAAkMDAwMDAwMDAtMDAwMC00MDAwLTgwMDAtMDAwMDAwMDAwMDAyAAAAATIAAAAEbGluawAAACQwMDAwMDAwMC0wMDAwLTQwMDAtODAwMC0wMDAwMDAwMDAwNjAAAAAgQw9LmFlmUUWmsbonQCRIe9ZvA6LdV313U8aNfX0AwAwAAAAg11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURoAAAABNAAAAAllcG9jaC1rZXk',
    enc: 'zo060cy2M-x7cMF4FKXHbs0CloUFDTRHRboFhw5YfVk',
    ciphertext: 'jwp-Wc7__1n5YQiFiWTQHjrSGobsuzgqJygs8R0-O-bDwjCykqJAfedm1sZmXpC5',
    signature:
      'VSY4EWd-udYyc4ag6JEXL7MBfLB5PNdhIlr0hdM0ezidqRi_qTeoBeKMj1iDkRkoLe9iD5wSPT60I4u9MvxmAA',
  },
];
const baselineVector = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/baseline-v1.json', import.meta.url), 'utf8'),
)[1];
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
function authority(kind: string, bytes: Uint8Array, recipient = '', epoch = '1') {
  const base = `${scope.space}_${scope.page}_${epoch}`;
  const subject = hash(bytes),
    count = Math.ceil(bytes.length / 32768);
  return Array.from({ length: count }, (_, index) =>
    document('authority', `${base}_${kind}_${subject}_${index + 1}`, {
      epoch: Number(epoch),
      kind,
      subject,
      seq: index + 1,
      count,
      recipient,
      bytes: c.encodeBinary(bytes.slice(index * 32768, (index + 1) * 32768)),
    }),
  );
}
async function fixture(
  ownAuthor?: OwnAuthor,
  epoch = '1',
  givenFold?: Pick<Fold, 'run' | 'close'>,
  baselineChange: Record<string, string> = {},
) {
  const base = `${scope.space}_${scope.page}_${epoch}`;
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
  let baseline: c.Envelope | undefined;
  let descriptor: Record<string, string> | undefined;
  if (epoch !== '1') {
    baseline = await c.Envelope.seal(
      {
        ...scope,
        epoch,
        kind: 'html',
        namespace: 'content',
        authorDevice: v.device,
        membershipRevision: String(statements.length + 1),
        streamSeq: '0',
        prevHash: new Uint8Array(32),
      },
      hex(v.epochKey),
      signer,
      c.text(JSON.stringify({ source: baselineVector.source, update: baselineVector.update })),
    );
    descriptor = {
      pageId: scope.page,
      epoch,
      sourceDigest: baselineVector.sourceDigest,
      baselineCommitment: baselineVector.commitment,
      title: baselineVector.title,
      objectEnvelopeHash: c.encodeBinary(await baseline.hash()),
      membershipRevision: String(statements.length + 1),
      ...baselineChange,
    };
    await append('epoch.advance', {
      pageId: scope.page,
      epoch,
      cuts: [],
      baseline: descriptor,
      wraps: [],
    });
  }
  const admission = new Admission(
    scope.space,
    scope.page,
    epoch,
    hex(v.public),
    { deviceId: v.page } as Registration,
    {
      linkId: v.linkId,
      principal: 'holder-uid',
      expiresAt: Number.MAX_SAFE_INTEGER,
    },
  );
  // The read adapter receives an Admission whose key was already opened by the wrap verifier.
  if (epoch === '1')
    admission.root = await crypto.subtle.importKey('raw', hex(v.epochKey), 'HKDF', false, [
      'deriveBits',
    ]);
  else {
    const key = await crypto.subtle.importKey(
      'pkcs8',
      c.concat(hex('302e020100300506032b656e04220420'), hex(v.recipientSeed)),
      'X25519',
      false,
      ['deriveBits'],
    );
    admission.registration.keys = {
      enc: await c.RecipientKey.fromHandle(
        key,
        c.wrap.Envelope.fromJson(c.text(JSON.stringify(v.wrap))).header().recipientKey,
      ),
    } as Registration['keys'];
  }
  const rows = statements.flatMap((bytes) => authority('membership', bytes, '', epoch));
  rows.push(...authority('chain', c.text(JSON.stringify(v.chain)), '', epoch));
  if (epoch !== '1')
    for (const value of [...historyWraps].reverse())
      rows.push(...authority('wrap', c.text(JSON.stringify(value)), `link:${v.linkId}`, epoch));
  const checkpoints: SealedDocument[] = [],
    logs: SealedDocument[] = [];
  if (baseline) await checkpoint(baseline, true);
  const requests: SyncQueryRequest[] = [];
  const port = {
    context: {
      uid: 'holder-uid',
      config: { projectId: 'demo-hosted' },
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
    publish: vi.fn(),
    read: vi.fn(),
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
  const fold =
    givenFold ??
    ({
      async run(command: FoldCommand) {
        folded.push(structuredClone(command));
        return {
          source: command.type === 'baseline' ? baselineVector.source : 'verified page',
          title: command.type === 'baseline' ? baselineVector.title : 'Title',
          own: {},
          update: new Uint8Array(),
        };
      },
      close: vi.fn(),
    } as Pick<Fold, 'run' | 'close'>);
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
    ownAuthor,
  );
  const context: c.Context = {
    ...scope,
    epoch,
    kind: 'update',
    namespace: 'content',
    authorDevice: v.page,
    streamSeq: '1',
    membershipRevision: epoch === '1' ? '1' : String(statements.length),
    prevHash: new Uint8Array(32),
  };
  async function seal(
    change: Partial<c.Context> = {},
    author = signer,
    body: Uint8Array = c.text('authenticated update'),
  ) {
    return c.Envelope.seal({ ...context, ...change }, hex(v.epochKey), author, body);
  }
  async function log(envelope: c.Envelope) {
    const header = c.decodeHeader(envelope.header()).context;
    const row = document('log', `${base}_${header.authorDevice}_${header.streamSeq}`, {
      epoch: Number(epoch),
      stream: header.authorDevice,
      seq: Number(header.streamSeq),
      hash: c.encodeBinary(await envelope.hash()),
      envelope: c.decodeText(envelope.toJson()),
    });
    logs.push(row);
    return row;
  }
  async function checkpoint(envelope: c.Envelope, baseline = false) {
    const object = baseline
        ? Buffer.from(await envelope.hash()).toString('hex')
        : c.decodeHeader(envelope.header()).objectId,
      bytes = envelope.toJson(),
      count = Math.ceil(bytes.length / 32768);
    checkpoints.push(
      ...Array.from({ length: count }, (_, index) =>
        document('checkpoints', `${base}_${object}_${index}`, {
          epoch: Number(epoch),
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
      device = await c.link.deriveDevice(seed, scope.space, linkId, 'door');
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
    baseline,
    descriptor,
    checkpoints,
    port,
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
  await f.reader.close();
  expect(f.admission.root).toBeNull();
  expect(f.fold.close).toHaveBeenCalledOnce();
});
it('uses its admitted stream position on later wakes with only chain inventory discovery', async () => {
  const f = await fixture(),
    one = await f.seal();
  await f.log(one);
  await f.reader.refresh(call(), new Set(['authority', 'log', 'checkpoints']));
  f.requests.length = 0;
  f.folded.length = 0;
  await f.log(await f.seal({ streamSeq: '2', prevHash: await one.hash() }));
  await f.reader.refresh(call(), new Set(['log']));
  expect(
    f.requests
      .filter((request) => request.query.resource === 'authority')
      .map((request) => request.query.scope.kind),
  ).toEqual([{ stringValue: 'chain' }]);
  expect(f.requests.find((request) => request.shape === 'stream')?.query).toMatchObject({
    cursor: { stream: v.page, seq: 1 },
  });
  expect(f.folded).toHaveLength(1);
  expect(f.folded[0]).toMatchObject({ updates: [c.text('authenticated update')] });
  await f.reader.close();
});
it('does not re-list held history and charges equal wake reads at 20, 100 and 200 originals', async () => {
  const costs: number[] = [];
  for (const held of [20, 100, 200]) {
    const f = await fixture();
    let previous = new Uint8Array(32);
    for (let seq = 1; seq <= held; seq++) {
      const envelope = await f.seal({ streamSeq: String(seq), prevHash: previous });
      await f.log(envelope);
      previous = new Uint8Array(await envelope.hash());
    }
    await f.reader.refresh(call(), new Set(['authority', 'log', 'checkpoints']));
    f.requests.length = 0;
    const query = f.port.query.bind(f.port);
    vi.spyOn(f.port, 'query').mockImplementation((request, call) => {
      if (request.shape === 'discovery' && request.query.resource === 'log')
        throw new Error('Whole-log rediscovery after initial catch-up');
      return query(request, call);
    });
    await f.log(await f.seal({ streamSeq: String(held + 1), prevHash: previous }));
    try {
      await f.reader.refresh(call(), new Set(['log']));
      const stream = f.requests.filter((request) => request.shape === 'stream');
      expect(
        stream.map((request) => 'cursor' in request.query && request.query.cursor?.seq),
      ).toEqual([held, held + 1]);
      expect(f.requests.filter((request) => request.query.resource === 'authority')).toHaveLength(
        1,
      );
      // Exercise the real origin guard with the current SDK page reservations: authority 5, log 1.
      let stored: unknown;
      const guard = new HostedBudget(budgetMath, {
        transact: async (change) => {
          const next = change(stored);
          stored = next.usage;
          return next.result;
        },
      });
      for (const request of f.requests)
        await guard.admit({
          operation: 'query',
          reads: request.query.resource === 'log' ? 1 : 5,
          writes: 0,
          rulesLookupsPerDocument: 3,
        });
      costs.push((await guard.snapshot()).reads);
      expect(f.folded.at(-1)).toMatchObject({ updates: [c.text('authenticated update')] });
    } finally {
      await f.reader.close();
    }
  }
  expect(costs).toEqual([28, 28, 28]);
});

it('queries a dormant chain again when its first original arrives without a membership notice', async () => {
  const f = await fixture(),
    writer = '00000000-0000-4000-8000-000000000456';
  await f.writer(writer);
  await f.reader.refresh(call(), new Set(['authority', 'log', 'checkpoints']));
  await f.reader.refresh(call(), new Set(['log']));
  f.requests.length = 0;
  await f.log(await f.seal({ authorDevice: writer }));
  await f.reader.refresh(call(), new Set(['log']));
  expect(
    f.requests
      .filter((request) => request.shape === 'stream')
      .map((request) => 'stream' in request.query && request.query.stream),
  ).toContain(writer);
  expect(f.folded.at(-1)).toMatchObject({ updates: [c.text('authenticated update')] });
  await f.reader.close();
});

it('finds a first writer on the queued log wake when it arrives during a chain scan', async () => {
  const f = await fixture(),
    writer = '00000000-0000-4000-8000-000000000456';
  await f.reader.refresh(call(), new Set(['authority', 'log', 'checkpoints']));
  const query = f.port.query.bind(f.port);
  let arrived = false;
  vi.spyOn(f.port, 'query').mockImplementation(async (request, call) => {
    const response = await query(request, call);
    if (!arrived && request.query.resource === 'authority') {
      arrived = true;
      await f.writer(writer);
      await f.log(await f.seal({ authorDevice: writer }));
    }
    return response;
  });
  await f.reader.refresh(call(), new Set(['log']));
  await f.reader.refresh(call(), new Set(['log']));
  expect(arrived).toBe(true);
  expect(f.folded.flatMap((command) => (command.type === 'apply' ? command.updates : []))).toEqual([
    c.text('authenticated update'),
  ]);
  await f.reader.close();
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
  await f.reader.close();
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
    await f.reader.close();
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
  await f.reader.close();
});
it('stops before reading or decoding when the caller is cancelled or the reader is closed', async () => {
  const f = await fixture();
  await expect(
    f.reader.refresh({ ...call(), signal: AbortSignal.abort() }, new Set(['log'])),
  ).rejects.toMatchObject({ state: 'cancelled' });
  expect(f.requests).toEqual([]);
  await f.reader.close();
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
  await f.reader.close();
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
  await f.reader.close();
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
    await f.reader.close();
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
    await f.reader.close();
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
      await f.reader.close();
    }
  }
});

it('an own entry another uid wrote does not stop a send: the Rules decide, so a link-mate continues the stream', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, _options: unknown, action: () => unknown) => action() },
  });
  const author = vi.fn(() => hex(v.public)),
    f = await fixture(author);
  try {
    await f.log(await f.seal({ namespace: 'own' }));
    const sent = await f.reader.submitOwn(
      [{ root: 'messages', key: 'comment', value: { text: 'Hello' } }],
      call(),
    );
    // Nothing pre-empts it: it is prepared and authored afresh; whatever the provider answers
    // (a 403 refusal or an occupied ID) is reported as that outcome, never as a standing block.
    expect(author).toHaveBeenCalled();
    expect(f.folded.some((command) => command.type === 'prepare-own')).toBe(true);
    expect(sent.outcome).not.toBe('writer-in-use');
  } finally {
    await f.reader.close();
    vi.unstubAllGlobals();
  }
});

it('fresh author refusal is typed before decoder preparation, sealing or provider publication', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, _options: unknown, action: () => unknown) => action() },
  });
  const author = vi.fn(() => {
      throw new Error('PRIVATE_ADMISSION_DETAIL');
    }),
    f = await fixture(author),
    seal = vi.spyOn(c.Envelope, 'seal');
  try {
    await expect(
      f.reader.submitOwn([{ root: 'messages', key: 'comment', value: { text: 'Hello' } }], call()),
    ).resolves.toMatchObject({
      outcome: 'refused',
      confirmed: 0,
      failure: { code: 'REMOTE_SCOPE_DENIED' },
    });
    expect(author).toHaveBeenCalledOnce();
    expect(seal).not.toHaveBeenCalled();
    expect(f.port.publish).not.toHaveBeenCalled();
    expect(f.folded.some((command) => command.type === 'prepare-own')).toBe(false);
  } finally {
    seal.mockRestore();
    await f.reader.close();
    vi.unstubAllGlobals();
  }
});

it('reads an epoch-two baseline and tail through the real authority and baseline verifiers with no extra reads', async () => {
  const f = await fixture(undefined, '2');
  try {
    await f.log(await f.seal());
    await expect(
      f.reader.refresh(call(), new Set(['authority', 'checkpoints', 'log'])),
    ).resolves.toMatchObject({ source: 'verified page' });
    expect(f.folded.map((value) => value.type)).toEqual(['baseline', 'apply']);
    expect(f.folded[0]).toMatchObject({
      type: 'baseline',
      title: baselineVector.title,
      update: c.binary(baselineVector.update, 4 * 1024 * 1024),
      sourceDigest: c.binary(baselineVector.sourceDigest, 32, 32),
      commitment: c.binary(baselineVector.commitment, 32, 32),
    });
    expect(f.requests.filter((value) => value.query.resource === 'checkpoints')).toHaveLength(1);
    f.requests.length = 0;
    await f.reader.refresh(call(), new Set(['log']));
    expect(f.requests.some((value) => value.query.resource === 'checkpoints')).toBe(false);
    expect(f.folded.filter((value) => value.type === 'baseline')).toHaveLength(1);
  } finally {
    await f.reader.close();
  }
});

it.each(['missing', 'incomplete', 'hash', 'scope'])(
  'refuses a %s baseline before any plaintext reaches the fold',
  async (mode) => {
    const f = await fixture(undefined, '2');
    try {
      if (mode === 'missing') f.checkpoints.length = 0;
      else {
        const original = f.checkpoints[0],
          fields = JSON.parse(c.decodeText(original.bytes));
        if (mode === 'incomplete') fields.count.integerValue = '2';
        if (mode === 'scope') fields.epoch.integerValue = '3';
        if (mode === 'hash') {
          const envelope = JSON.parse(c.decodeText(c.binary(fields.bytes.stringValue, 32768)));
          envelope.signature = c.encodeBinary(new Uint8Array(64));
          fields.bytes.stringValue = c.encodeBinary(c.text(JSON.stringify(envelope)));
        }
        const bytes = c.text(JSON.stringify(fields));
        f.checkpoints[0] = { ...original, bytes, digest: hash(bytes) };
      }
      await expect(
        f.reader.refresh(call(), new Set(['authority', 'checkpoints', 'log'])),
      ).rejects.toMatchObject({ code: 'COLAB_HOSTED_RESHARE_REQUIRED' });
      expect(f.folded).toEqual([]);
    } finally {
      await f.reader.close();
    }
  },
);

it.each(['REMOTE_BUDGET_EXHAUSTED', 'REMOTE_RATE_LIMITED', 'REMOTE_SCOPE_DENIED'])(
  'does not relabel or retry %s while acquiring baseline chunks',
  async (code) => {
    const f = await fixture(undefined, '2'),
      query = f.port.query.bind(f.port);
    const reads = vi.spyOn(f.port, 'query').mockImplementation((request, call) => {
      if (request.query.resource === 'checkpoints') throw { code };
      return query(request, call);
    });
    try {
      await expect(
        f.reader.refresh(call(), new Set(['authority', 'checkpoints', 'log'])),
      ).rejects.toMatchObject({ code });
      expect(
        reads.mock.calls.filter(([request]) => request.query.resource === 'checkpoints'),
      ).toHaveLength(1);
      expect(f.folded).toEqual([]);
    } finally {
      await f.reader.close();
    }
  },
);

it('preserves cancellation during baseline discovery and joins the reader key cleanup', async () => {
  const f = await fixture(undefined, '2'),
    controller = new AbortController(),
    query = f.port.query.bind(f.port);
  vi.spyOn(f.port, 'query').mockImplementation(async (request, call) => {
    const value = await query(request, call);
    if (request.query.resource === 'checkpoints') controller.abort();
    return value;
  });
  await expect(
    f.reader.refresh(
      { ...call(), signal: controller.signal },
      new Set(['authority', 'checkpoints', 'log']),
    ),
  ).rejects.toMatchObject({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' });
  expect(f.folded).toEqual([]);
  await f.reader.close();
  expect(f.admission.root).toBeNull();
  expect(f.fold.close).toHaveBeenCalledOnce();
});

it('ends an old generation on a signed epoch switch before returning another projection', async () => {
  const f = await fixture(undefined, '2');
  try {
    await f.reader.refresh(call(), new Set(['authority', 'checkpoints', 'log']));
    const before = f.folded.length;
    await f.append('epoch.advance', {
      pageId: scope.page,
      epoch: '3',
      cuts: [],
      baseline: {
        ...f.descriptor,
        epoch: '3',
        membershipRevision: String(f.statements.length + 1),
      },
      wraps: [],
    });
    f.rows.push(...authority('membership', f.statements.at(-1)!, '', '2'));
    await expect(f.reader.refresh(call(), new Set(['authority']))).rejects.toMatchObject({
      code: 'REMOTE_STATE_UNAVAILABLE',
      state: 'catch-up-required',
    });
    expect(f.folded).toHaveLength(before);
  } finally {
    await f.reader.close();
  }
  expect(f.admission.root).toBeNull();
  expect(f.fold.close).toHaveBeenCalledOnce();
});

it('requires the baseline fold source and title to agree with the signed original before applying a tail', async () => {
  const f = await fixture(undefined, '2');
  vi.spyOn(f.fold, 'run').mockResolvedValue({
    source: 'forged projection',
    title: baselineVector.title,
    own: {},
    update: new Uint8Array(),
  });
  try {
    await expect(
      f.reader.refresh(call(), new Set(['authority', 'checkpoints', 'log'])),
    ).rejects.toMatchObject({ code: 'COLAB_HOSTED_RESHARE_REQUIRED' });
    expect(f.fold.run).toHaveBeenCalledTimes(1);
  } finally {
    await f.reader.close();
  }
});

/** Existing fold.worker decoder with the real parent admission, without a host browser. */
async function workerFold() {
  vi.resetModules();
  const surface = {
    onmessage: null as unknown as (event: {
      data: { id: number; command: DecoderCommand };
    }) => Promise<void>,
    postMessage: (data: unknown) => worker.onmessage?.({ data }),
  };
  const worker = {
    onmessage: null as null | ((event: { data: unknown }) => void),
    onerror: null,
    terminate: vi.fn(),
    postMessage: (data: { id: number; command: DecoderCommand }) => {
      void surface.onmessage({ data });
    },
  };
  vi.stubGlobal('self', surface);
  await import('../src/fold.worker.js');
  return new Fold(worker as unknown as Worker);
}

it('renders the reviewed epoch-two baseline and a real later Yjs update through the actual Worker', async () => {
  const prepare = await workerFold();
  const initial = await prepare.run({
    type: 'baseline',
    title: baselineVector.title,
    update: c.binary(baselineVector.update, 4 * 1024 * 1024),
    sourceDigest: c.binary(baselineVector.sourceDigest, 32, 32),
    commitment: c.binary(baselineVector.commitment, 32, 32),
  });
  const nextSource = '<p>After the signed baseline</p>';
  const updates = updatesOf(await prepare.prepareContent(nextSource, contentBase(initial)));
  prepare.close();
  const actual = await workerFold();
  const commands: FoldCommand[] = [];
  const f = await fixture(undefined, '2', {
    run(command) {
      commands.push(structuredClone(command));
      return actual.run(command);
    },
    close() {
      actual.close();
    },
  });
  try {
    let previous = new Uint8Array(32);
    for (let i = 0; i < updates.length; i++) {
      const envelope = await f.seal(
        { streamSeq: String(i + 1), prevHash: previous },
        undefined,
        updates[i],
      );
      previous = await envelope.hash();
      await f.log(envelope);
    }
    await expect(
      f.reader.refresh(call(), new Set(['authority', 'checkpoints', 'log'])),
    ).resolves.toMatchObject({ source: nextSource, title: baselineVector.title });
    expect(commands.map((value) => value.type)).toEqual(['baseline', 'apply']);
    expect(f.requests.filter((value) => value.query.resource === 'checkpoints')).toHaveLength(1);
    f.requests.length = 0;
    await expect(f.reader.refresh(call(), new Set(['log']))).resolves.toMatchObject({
      source: nextSource,
    });
    expect(commands.filter((value) => value.type === 'baseline')).toHaveLength(1);
    expect(f.requests.some((value) => value.query.resource === 'checkpoints')).toBe(false);
  } finally {
    await f.reader.close();
    vi.unstubAllGlobals();
  }
});

it('refuses a signed baseline whose independent commitment fails in the actual Worker', async () => {
  const actual = await workerFold();
  const commands = vi.spyOn(actual, 'run');
  const f = await fixture(undefined, '2', actual, {
    baselineCommitment: c.encodeBinary(new Uint8Array(32)),
  });
  try {
    await expect(
      f.reader.refresh(call(), new Set(['authority', 'checkpoints', 'log'])),
    ).rejects.toMatchObject({ code: 'COLAB_HOSTED_RESHARE_REQUIRED' });
    expect(commands).toHaveBeenCalledOnce();
    expect(commands.mock.calls[0][0].type).toBe('baseline');
  } finally {
    await f.reader.close();
    vi.unstubAllGlobals();
  }
});

it.each(['duplicate', 'signature'])(
  'keeps existing wrap admission for a %s epoch-two wrap set',
  async (mode) => {
    const f = await fixture(undefined, '2');
    const current = structuredClone(historyWraps[1]);
    if (mode === 'signature') current.signature = c.encodeBinary(new Uint8Array(64));
    if (mode === 'duplicate') {
      // Different whole-original whitespace, same epoch: bypassing transport dedup cannot admit it.
      f.rows.push(
        ...authority('wrap', c.text(' ' + JSON.stringify(current)), `link:${v.linkId}`, '2'),
      );
    } else {
      for (let i = f.rows.length - 1; i >= 0; i--)
        if (JSON.parse(c.decodeText(f.rows[i].bytes)).kind.stringValue === 'wrap')
          f.rows.splice(i, 1);
      f.rows.push(
        ...authority('wrap', c.text(JSON.stringify(historyWraps[0])), `link:${v.linkId}`, '2'),
        ...authority('wrap', c.text(JSON.stringify(current)), `link:${v.linkId}`, '2'),
      );
    }
    try {
      await expect(
        f.reader.refresh(call(), new Set(['authority', 'checkpoints', 'log'])),
      ).rejects.toThrow();
      expect(f.folded).toEqual([]);
    } finally {
      await f.reader.close();
    }
  },
);
