import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import { Admission } from '../src/admission.js';
import { Objects } from '../src/objects.js';
import { HostedPageReader } from '../src/hosted-reader.js';
import { Fold } from '../src/fold.js';
import * as Y from 'yjs';
import type { FoldCommand, DecoderCommand } from '../src/fold-protocol.js';
import type {
  SealedDocument,
  SealedSyncPort,
  SyncCall,
  SyncQueryRequest,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import type { Registration } from '../src/registration.js';

vi.mock('../src/storage.js', () => ({ record: async () => undefined }));
vi.stubGlobal('navigator', { locks: { request: async (_key: string, fn: () => unknown) => fn() } });

// The same frozen bytes native `src/fold_vectors.rs` replays at model level: each case is a
// catch-up of anchor frames and fold frames, and marks the ones both sides admit and refuse.
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/fold-v1.json', import.meta.url), 'utf8'),
);
interface Row {
  name: string;
  log: string;
  frames: { streamId: string; namespace: string; checkpoint: unknown; anchor?: unknown }[];
  expect: string;
}
async function admission(log: string, preloadChains = true): Promise<Admission> {
  const statements: string[] = v.logs[log];
  const last = c.statement.Envelope.fromJson(c.binary(statements.at(-1)!, 32 * 1024));
  const a = new Admission(
    v.spaceId,
    v.pageId,
    v.epoch,
    c.binary(v.ownerKey, 32, 32),
    {} as Registration,
    {
      linkId: '70000000-0000-4000-8000-000000000001',
      principal: 'principal',
      expiresAt: Number.MAX_SAFE_INTEGER,
    },
  );
  await a.membership(
    {
      revision: String(statements.length),
      statementHash: c.encodeBinary(await last.hash()),
      ownerKey: v.ownerKey,
      statements,
      more: false,
    },
    true,
  );
  a.root = await crypto.subtle.importKey('raw', c.binary(v.epochSecret, 32, 32), 'HKDF', false, [
    'deriveBits',
  ]);
  if (preloadChains) await a.chains(v.chains);
  return a;
}
/** Every frame in catch-up order; the first refusal ends the replay, as it ends a catch-up. */
async function replay(row: Row) {
  const objects = new Objects(await admission(row.log));
  const admitted: { namespace: string; writer: string }[] = [];
  let refused: unknown;
  for (const frame of row.frames) {
    try {
      const { checkpoints } = await objects.streams([frame]);
      admitted.push(...checkpoints);
    } catch (error) {
      refused = error;
      break;
    }
  }
  return { admitted, refused };
}

for (const row of v.cases as Row[]) {
  it(`${row.expect === 'admit' ? 'admits' : 'rejects'}: ${row.name}`, async () => {
    const { admitted, refused } = await replay(row);
    const folds = row.frames.filter((f) => f.checkpoint !== null);
    if (row.expect === 'admit') {
      expect(refused).toBeUndefined();
      // The writer of a folded record is the covered device, never the signer.
      expect(admitted).toHaveLength(folds.length);
      for (const [i, fold] of folds.entries())
        expect(admitted[i]).toMatchObject({ namespace: fold.namespace, writer: fold.streamId });
    } else {
      // A refusal of the model, not a crash of the replay.
      expect((refused as Error).message).toMatch(
        /^(Invalid colab-v1 value or proof|Fresh membership catchup required)$/,
      );
      // Nothing from a refused fold is applied.
      expect(admitted).toHaveLength(0);
    }
  });
}

it('refuses a fold header as a thirteen-field header, the way a reader of today does', () => {
  const header = c.binary(v.oldReader.header, 4096);
  expect(v.oldReader).toMatchObject({ fields: 14, readerFields: 13 });
  expect(() => c.fields(header, v.oldReader.readerFields)).toThrow();
  expect(c.fields(header, v.oldReader.fields)).toHaveLength(v.oldReader.fields);
  expect(c.decodeHeader(header)).toMatchObject({
    context: { kind: 'fold' },
    coveredDevice: v.cases[0].frames[0].streamId,
  });
});

/** Frozen originals through the actual cloud document adapter, not direct Objects frames. */
async function cloudFold(
  index: number,
  tail: boolean | Uint8Array = false,
  decoder?: Pick<Fold, 'run' | 'close'>,
) {
  const row: Row = v.cases[index],
    a = await admission(row.log, false),
    statements: string[] = [...v.logs[row.log]],
    signer = await crypto.subtle.importKey(
      'pkcs8',
      c.concat(
        Uint8Array.from('302e020100300506032b657004220420'.match(/../g)!, (n) => parseInt(n, 16)),
        new Uint8Array(32).fill(0x41),
      ),
      'Ed25519',
      false,
      ['sign'],
    );
  // Add read policy after the frozen log without changing any signed fold or revoke.
  for (const [operation, value] of [
    ['page.share', { pageId: v.pageId, mode: 'link', epoch: '1' }],
    [
      'link.add',
      {
        linkId: a.reader!.linkId,
        role: 'viewer',
        linkSignKey: v.ownerKey,
        linkEncKey: v.ownerKey,
        pages: [v.pageId],
      },
    ],
  ] as const) {
    const previous = c.statement.Envelope.fromJson(c.binary(statements.at(-1)!, 32768)),
      payload = c.text(JSON.stringify(value)),
      input = c.statement.input({
        space: v.spaceId,
        operation,
        revision: String(statements.length + 1),
        previousHash: await previous.hash(),
        payloadDigest: await c.digest(payload),
      });
    statements.push(
      c.encodeBinary(
        c.text(
          JSON.stringify({
            statement: c.encodeBinary(input),
            payload: c.encodeBinary(payload),
            signature: c.encodeBinary(await c.sign(signer, input)),
          }),
        ),
      ),
    );
  }
  const scope = { space: v.spaceId, page: v.pageId, epoch: 1 },
    base = `${v.spaceId}_${v.pageId}_1`,
    documents: SealedDocument[] = [];
  async function document(resource: string, id: string, values: Record<string, string | number>) {
    const fields = Object.fromEntries(
        Object.entries({ ...scope, uid: 'owner-uid', expiresAt: '2027-01-01T00:00:00Z', ...values })
          .sort(([a], [b]) => a.localeCompare(b))
          .map(([key, value]) => [
            key,
            typeof value === 'number'
              ? { integerValue: String(value) }
              : key === 'expiresAt'
                ? { timestampValue: value }
                : { stringValue: value },
          ]),
      ),
      bytes = c.text(JSON.stringify(fields)),
      digest = Array.from(await c.digest(bytes), (b) => b.toString(16).padStart(2, '0')).join('');
    const result = { target: { resource, id }, bytes, digest };
    documents.push(result);
    return result;
  }
  for (const raw of statements) {
    const bytes = c.binary(raw, 32768),
      subject = Array.from(await c.digest(bytes), (b) => b.toString(16).padStart(2, '0')).join('');
    await document('authority', `${base}_membership_${subject}_1`, {
      kind: 'membership',
      subject,
      recipient: '',
      count: 1,
      seq: 1,
      bytes: raw,
    });
  }
  for (const { chain } of v.chains) {
    const bytes = c.binary(chain, 32768),
      subject = Array.from(await c.digest(bytes), (b) => b.toString(16).padStart(2, '0')).join('');
    await document('authority', `${base}_chain_${subject}_1`, {
      kind: 'chain',
      subject,
      recipient: '',
      count: 1,
      seq: 1,
      bytes: chain,
    });
  }
  for (const frame of row.frames) {
    const entry = (frame.anchor ?? frame.checkpoint) as { envelope: string } | null;
    if (!entry) continue;
    const bytes = c.binary(entry.envelope, 354304),
      envelope = c.Envelope.fromJson(bytes),
      header = c.decodeHeader(envelope.header()),
      context = header.context;
    if (frame.anchor)
      await document('log', `${base}_${context.authorDevice}_${context.streamSeq}`, {
        stream: context.authorDevice,
        seq: Number(context.streamSeq),
        hash: c.encodeBinary(await envelope.hash()),
        envelope: c.decodeText(bytes),
      });
    else {
      const count = Math.ceil(bytes.length / 32768);
      for (let i = 0; i < count; i++)
        await document('checkpoints', `${base}_${header.objectId}_${i}`, {
          object: header.objectId,
          index: i,
          seq: i + 1,
          count,
          bytes: c.encodeBinary(bytes.slice(i * 32768, (i + 1) * 32768)),
        });
    }
  }
  if (tail) {
    const entry = v.cases[0].frames[0].anchor as { envelope: string },
      anchor = c.Envelope.fromJson(c.binary(entry.envelope, 354304)),
      covered = c.decodeHeader(anchor.header()).context,
      writer = await crypto.subtle.importKey(
        'pkcs8',
        c.concat(
          Uint8Array.from('302e020100300506032b657004220420'.match(/../g)!, (n) => parseInt(n, 16)),
          new Uint8Array(32).fill(22),
        ),
        'Ed25519',
        false,
        ['sign'],
      ),
      next = await c.Envelope.seal(
        {
          ...covered,
          kind: 'update',
          namespace: 'content',
          streamSeq: '3',
          prevHash: await anchor.hash(),
        },
        c.binary(v.epochSecret, 32, 32),
        writer,
        tail instanceof Uint8Array ? tail : new Uint8Array([0, 0]),
      );
    await document('log', `${base}_${covered.authorDevice}_3`, {
      stream: covered.authorDevice,
      seq: 3,
      hash: c.encodeBinary(await next.hash()),
      envelope: c.decodeText(next.toJson()),
    });
  }
  const commands: FoldCommand[] = [],
    requests: SyncQueryRequest[] = [],
    read = vi.fn(async (target: { resource: string; id: string }) =>
      documents.find((d) => d.target.resource === target.resource && d.target.id === target.id),
    ),
    port = {
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
      read,
      async query(request: SyncQueryRequest) {
        requests.push(request);
        const q = request.query;
        let found = documents.filter((d) => d.target.resource === q.resource);
        if ('kind' in q.scope)
          found = found.filter(
            (d) =>
              JSON.parse(c.decodeText(d.bytes)).kind.stringValue ===
              (q.scope.kind as { stringValue: string }).stringValue,
          );
        if (request.shape === 'discovery')
          return { shape: 'discovery', page: { documents: found } };
        if (!('stream' in q)) throw new Error('Expected stream');
        found = found.filter((d) => {
          const f = JSON.parse(c.decodeText(d.bytes));
          return (
            f.stream.stringValue === q.stream && Number(f.seq.integerValue) > (q.cursor?.seq ?? 0)
          );
        });
        return {
          shape: 'stream',
          page: {
            documents: found,
            ...(found.length
              ? {
                  cursor: {
                    stream: q.stream,
                    seq: Number(JSON.parse(c.decodeText(found.at(-1)!.bytes)).seq.integerValue),
                  },
                }
              : {}),
          },
        };
      },
    } as unknown as SealedSyncPort,
    reader = new HostedPageReader(
      port,
      a,
      {
        revision: '1',
        statement: await c.statement.Envelope.fromJson(c.binary(statements[0], 32768)).hash(),
      },
      new Map(
        ['log', 'checkpoints', 'authority'].map((r) => [
          r,
          { documents: 1024, bytes: 4 * 1024 * 1024 },
        ]),
      ),
      {
        async run(command: FoldCommand) {
          commands.push(command);
          return decoder
            ? decoder.run(command)
            : { source: 'verified folded page', title: '', own: {}, update: new Uint8Array() };
        },
        close: () => decoder?.close(),
      } as Pick<Fold, 'run' | 'close'>,
    );
  return { reader, commands, documents, read, requests };
}
const cloudCall = (): SyncCall => ({
  signal: new AbortController().signal,
  deadlineAtMs: Date.now() + 30000,
});
for (const row of v.cloudCases as { case: number; expect: string }[]) {
  it(`cloud fold ${row.case}: ${row.expect}`, async () => {
    const f = await cloudFold(row.case);
    try {
      const result = f.reader.refresh(cloudCall(), new Set(['authority', 'log', 'checkpoints']));
      if (row.expect === 'admit') {
        await expect(result).resolves.toMatchObject({ source: 'verified folded page' });
        expect(f.commands.map((c) => c.type)).toEqual(['checkpoint', 'checkpoint', 'apply']);
        expect(f.commands.at(-1)).toMatchObject({ type: 'apply', updates: [], own: [] });
        expect(f.commands[1]).toMatchObject({
          type: 'checkpoint',
          writer: v.cases[0].frames[0].streamId,
        });
        f.commands.length = 0;
        await f.reader.refresh(cloudCall(), new Set(['log']));
        expect(f.commands.map((c) => c.type)).toEqual(['apply']);
      } else {
        await expect(result).rejects.toBeDefined();
        expect(f.commands).toEqual([]);
      }
    } finally {
      await f.reader.close();
    }
  });
}

it('cloud fold takes n+1 from the anchor hash, not the fold hash, and never applies the anchor', async () => {
  const f = await cloudFold(0, true);
  try {
    await f.reader.refresh(cloudCall(), new Set(['authority', 'log', 'checkpoints']));
    expect(f.commands.map((c) => c.type)).toEqual(['checkpoint', 'checkpoint', 'apply']);
    expect(f.commands.at(-1)).toMatchObject({
      type: 'apply',
      updates: [new Uint8Array([0, 0])],
      own: [],
    });
    expect(f.read).not.toHaveBeenCalled(); // initial discovery already charged for the anchor
  } finally {
    await f.reader.close();
  }
});

it('a newly discovered folded writer uses one charged exact-ID anchor read on a later wake', async () => {
  const f = await cloudFold(0),
    later = f.documents.filter((d) => d.target.resource !== 'authority');
  f.documents.splice(
    0,
    f.documents.length,
    ...f.documents.filter((d) => d.target.resource === 'authority'),
  );
  try {
    await f.reader.refresh(cloudCall(), new Set(['authority', 'log', 'checkpoints']));
    f.commands.length = 0;
    f.documents.push(...later);
    await f.reader.refresh(cloudCall(), new Set(['checkpoints', 'log']));
    expect(f.read).toHaveBeenCalledOnce();
    expect(f.read.mock.calls[0][0]).toEqual({
      resource: 'log',
      id: `${v.spaceId}_${v.pageId}_1_${v.cases[0].frames[0].streamId}_2`,
    });
    expect(f.commands.map((c) => c.type)).toEqual(['checkpoint', 'checkpoint', 'apply']);
  } finally {
    await f.reader.close();
  }
});

for (const mode of [
  'missing-anchor',
  'wrong-anchor',
  'missing-signer',
  'missing-covered',
  'incomplete',
  'alias',
  'wrong-hash',
  'scope',
  'unpaired',
] as const) {
  it(`a complete cloud fold pair refuses ${mode} before applying any data`, async () => {
    const f = await cloudFold(0);
    const change = async (
      resource: string,
      modify: (fields: ReturnType<typeof JSON.parse>) => void,
    ) => {
      const index = f.documents.findIndex((d) => d.target.resource === resource),
        old = f.documents[index],
        fields = JSON.parse(c.decodeText(old.bytes));
      modify(fields);
      const bytes = c.text(JSON.stringify(fields));
      f.documents[index] = {
        ...old,
        bytes,
        digest: Array.from(await c.digest(bytes), (b) => b.toString(16).padStart(2, '0')).join(''),
      };
    };
    if (mode === 'missing-anchor')
      f.documents.splice(
        f.documents.findIndex((d) => d.target.resource === 'log'),
        1,
      );
    if (mode === 'wrong-anchor') {
      const bytes = c.binary(v.first.envelope, 354304);
      await change('log', (fields) => {
        fields.envelope.stringValue = c.decodeText(bytes);
      });
    }
    if (mode === 'missing-signer' || mode === 'missing-covered') {
      const device = v.cases[0].frames[mode === 'missing-signer' ? 1 : 0];
      const id =
        mode === 'missing-signer'
          ? c.decodeHeader(
              c.Envelope.fromJson(
                c.binary((device.checkpoint as { envelope: string }).envelope, 354304),
              ).header(),
            ).context.authorDevice
          : device.streamId;
      const index = f.documents.findIndex((d) => {
        if (d.target.resource !== 'authority') return false;
        const fields = JSON.parse(c.decodeText(d.bytes));
        return (
          fields.kind.stringValue === 'chain' &&
          c.certificate.Chain.fromJson(c.binary(fields.bytes.stringValue, 32768)).certificate()
            .deviceId === id
        );
      });
      expect(index).toBeGreaterThanOrEqual(0);
      f.documents.splice(index, 1);
    }
    if (mode === 'incomplete')
      await change('checkpoints', (fields) => {
        fields.count.integerValue = '2';
      });
    if (mode === 'scope')
      await change('checkpoints', (fields) => {
        fields.epoch.integerValue = '2';
      });
    if (mode === 'unpaired')
      f.documents.splice(
        f.documents.findIndex((d) => d.target.resource === 'checkpoints'),
        1,
      );
    if (mode === 'alias') {
      const row = f.documents.find((d) => d.target.resource === 'checkpoints')!;
      f.documents.push({ ...row, target: { ...row.target, id: row.target.id + '-alias' } });
    }
    if (mode === 'wrong-hash')
      await change('checkpoints', (fields) => {
        const envelope = JSON.parse(c.decodeText(c.binary(fields.bytes.stringValue, 32768)));
        envelope.signature = c.encodeBinary(new Uint8Array(64));
        fields.bytes.stringValue = c.encodeBinary(c.text(JSON.stringify(envelope)));
      });
    try {
      await expect(
        f.reader.refresh(cloudCall(), new Set(['authority', 'log', 'checkpoints'])),
      ).rejects.toBeDefined();
      expect(f.commands).toEqual([]);
    } finally {
      await f.reader.close();
    }
  });
}

it.each([
  'REMOTE_BUDGET_EXHAUSTED',
  'REMOTE_RATE_LIMITED',
  'REMOTE_SCOPE_DENIED',
  'cancelled',
  'deadline',
])('does not retry or apply after an anchor read ends in %s', async (mode) => {
  const f = await cloudFold(0),
    later = f.documents.filter((d) => d.target.resource !== 'authority');
  f.documents.splice(
    0,
    f.documents.length,
    ...f.documents.filter((d) => d.target.resource === 'authority'),
  );
  try {
    await f.reader.refresh(cloudCall(), new Set(['authority', 'log', 'checkpoints']));
    f.commands.length = 0;
    f.documents.push(...later);
    const controller = new AbortController(),
      call = { signal: controller.signal, deadlineAtMs: Date.now() + 30000 };
    f.read.mockImplementation(async (target) => {
      if (mode === 'cancelled') controller.abort();
      else if (mode === 'deadline') call.deadlineAtMs = Date.now() - 1;
      else throw { code: mode };
      return f.documents.find(
        (d) => d.target.resource === target.resource && d.target.id === target.id,
      );
    });
    await expect(f.reader.refresh(call, new Set(['log', 'checkpoints']))).rejects.toMatchObject(
      mode === 'cancelled' || mode === 'deadline'
        ? { code: 'REMOTE_STATE_UNAVAILABLE', state: mode }
        : { code: mode },
    );
    expect(f.read).toHaveBeenCalledOnce();
    expect(f.commands).toEqual([]);
  } finally {
    await f.reader.close();
  }
});

it('opens a folded prefix and a signed later edit through the real Worker decoder', async () => {
  vi.resetModules();
  const surface = {
      onmessage: null as unknown as (event: {
        data: { id: number; command: DecoderCommand };
      }) => Promise<void>,
      postMessage: (data: unknown) => worker.onmessage?.({ data }),
    },
    worker = {
      onmessage: null as null | ((event: { data: unknown }) => void),
      onerror: null,
      terminate: vi.fn(),
      postMessage: (data: { id: number; command: DecoderCommand }) => {
        void surface.onmessage({ data });
      },
    };
  vi.stubGlobal('self', surface);
  await import('../src/fold.worker.js');
  const decoder = new Fold(worker as unknown as Worker),
    doc = new Y.Doc(),
    source = '<p>A signed edit after the owner fold</p>';
  doc.getText('html').insert(0, source);
  doc.getMap('meta').set('title', 'Folded page');
  const f = await cloudFold(0, Y.encodeStateAsUpdate(doc), decoder);
  try {
    await expect(
      f.reader.refresh(cloudCall(), new Set(['authority', 'log', 'checkpoints'])),
    ).resolves.toMatchObject({ source, title: 'Folded page' });
    expect(f.commands.map((c) => c.type)).toEqual(['checkpoint', 'checkpoint', 'apply']);
    expect(f.commands[1]).toMatchObject({
      type: 'checkpoint',
      writer: v.cases[0].frames[0].streamId,
    });
    expect(f.read).not.toHaveBeenCalled();
  } finally {
    await f.reader.close();
    doc.destroy();
    vi.unstubAllGlobals();
  }
});
