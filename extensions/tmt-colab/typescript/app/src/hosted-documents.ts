import {
  binary,
  concat,
  decodeHeader,
  digest,
  Envelope,
  equal,
  exactKeys,
  generatedId,
  requireValue,
  spaceId,
  strictJson,
  text,
} from '@tmt/colab-client';
import type { SealedDocument } from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import { hex } from './export.js';
import type { ObjectEntry } from './objects.js';
import { HostedReshareRequired } from './hosted-authority.js';

export interface HostedScope {
  readonly space: string;
  readonly page: string;
  readonly epoch: string;
}
/** Exact bytes still require the existing signed Colab admission, never a transport head. */
export interface HostedOriginal {
  bytes: Uint8Array;
  envelope: Envelope;
  header: ReturnType<typeof decodeHeader>;
  hash: Uint8Array;
}

/** Project one existing own envelope into the same native log mapping; never seal content here. */
export async function hostedOwnLog(
  entry: ObjectEntry,
  scope: HostedScope,
  uid: string,
  expiresAtMs: number,
): Promise<SealedDocument> {
  const envelope = binary(entry.envelope, 354304),
    header = decodeHeader(Envelope.fromJson(envelope).header());
  requireValue(header.context.namespace === 'own');
  const expiresAt = new Date(expiresAtMs).toISOString().replace('.000Z', 'Z');
  const row = {
    envelope: { stringValue: new TextDecoder('utf-8', { fatal: true }).decode(envelope) },
    epoch: { integerValue: scope.epoch },
    expiresAt: { timestampValue: expiresAt },
    hash: { stringValue: entry.envelopeHash },
    page: { stringValue: scope.page },
    seq: { integerValue: entry.seq },
    space: { stringValue: scope.space },
    stream: { stringValue: header.context.authorDevice },
    uid: { stringValue: uid },
  };
  const bytes = text(JSON.stringify(row)),
    document = {
      target: {
        resource: 'log',
        id: `${scope.space}_${scope.page}_${scope.epoch}_${header.context.authorDevice}_${entry.seq}`,
      },
      bytes,
      digest: hex(await digest(bytes)),
    };
  // Reuse the reviewed original mapping, not a second field/envelope validator.
  await hostedLog(document, scope);
  return document;
}

type Fields = Record<string, unknown>;
type Resource = 'log' | 'checkpoints' | 'authority';
const BASE = ['space', 'page', 'epoch', 'uid', 'expiresAt'];
const KEYS = {
  log: [...BASE, 'stream', 'seq', 'hash', 'envelope'],
  checkpoints: [...BASE, 'object', 'index', 'seq', 'count', 'bytes'],
  authority: [...BASE, 'kind', 'subject', 'seq', 'count', 'recipient', 'bytes'],
} as const;

/** Native twin: documents::originals::log, then existing Objects admits the signature and chain. */
export async function hostedLog(
  document: SealedDocument,
  scope: HostedScope,
): Promise<HostedOriginal & { publisherUid: string }> {
  const row = await fields(document, 'log', scope);
  const raw = string(row, 'envelope');
  const bytes = text(raw);
  requireValue(raw.length <= 354304 && bytes.length === raw.length);
  const result = await original(bytes, scope, 'update');
  requireValue(
    result.header.context.authorDevice === string(row, 'stream') &&
      result.header.context.streamSeq === number(row, 'seq').toString(),
  );
  requireValue(equal(result.hash, binary(string(row, 'hash'), 32, 32)));
  return { ...result, publisherUid: string(row, 'uid') };
}

/** One complete ordered object query; partial chunks cannot become an admitted checkpoint. */
export async function hostedCheckpoint(
  documents: readonly SealedDocument[],
  scope: HostedScope,
): Promise<HostedOriginal> {
  const rows = await ordered(documents, 'checkpoints', scope, 513);
  const bytes = pieces(rows);
  const result = await original(bytes, scope, ['checkpoint', 'fold']);
  requireValue(result.header.objectId === string(rows[0], 'object'));
  return result;
}

/** A completed declared discovery run; unsigned objects and order grant no stream progress. */
export async function hostedCheckpointSet(
  documents: readonly SealedDocument[],
  scope: HostedScope,
  baselineHash?: Uint8Array,
): Promise<readonly HostedOriginal[]> {
  const groups = new Map<string, Fields[]>();
  for (const document of documents) {
    const row = await fields(document, 'checkpoints', scope),
      object = string(row, 'object');
    const group = groups.get(object) ?? [];
    group.push(row);
    groups.set(object, group);
  }
  const originals: { id: string; value: HostedOriginal }[] = [];
  const expected = baselineHash && new Uint8Array(baselineHash);
  if (expected) requireValue(expected.length === 32);
  for (const [id, rows] of groups) {
    rows.sort((a, b) => number(a, 'seq') - number(b, 'seq'));
    orderedRows(rows, 'checkpoints', 513);
    originals.push({
      id,
      value: await original(
        pieces(rows),
        scope,
        expected ? ['html', 'checkpoint', 'fold'] : ['checkpoint', 'fold'],
      ),
    });
  }
  // Only the verified descriptor may select a baseline; duplicate originals under aliases refuse.
  const matches = expected ? originals.filter(({ value }) => equal(value.hash, expected)) : [];
  if (
    expected &&
    (matches.length !== 1 ||
      matches[0].id !== hex(expected) ||
      matches[0].value.header.context.kind !== 'html')
  )
    throw new HostedReshareRequired();
  for (const entry of originals) {
    if (entry === matches[0]) continue;
    requireValue(
      ['checkpoint', 'fold'].includes(entry.value.header.context.kind) &&
        entry.value.header.objectId === entry.id,
    );
  }
  return originals.map(({ value }) => value);
}

/** Native twin: authority whole-original SHA-256, followed by Admission's signed verifier. */
export async function hostedAuthority(
  documents: readonly SealedDocument[],
  scope: HostedScope,
  kind: 'membership' | 'chain' | 'wrap',
  recipient: string,
): Promise<Uint8Array> {
  const rows = await ordered(documents, 'authority', scope, kind === 'membership' ? 11 : 1);
  return authorityOriginal(rows, kind, recipient);
}

/** Discovery order is lexical document order, not statement revision or piece sequence. */
export async function hostedAuthoritySet(
  documents: readonly SealedDocument[],
  scope: HostedScope,
  kind: 'membership' | 'chain' | 'wrap',
  recipient: string,
): Promise<readonly Uint8Array[]> {
  const groups = new Map<string, Fields[]>();
  for (const document of documents) {
    const row = await fields(document, 'authority', scope),
      subject = string(row, 'subject');
    const group = groups.get(subject) ?? [];
    group.push(row);
    groups.set(subject, group);
  }
  const originals: Uint8Array[] = [];
  for (const rows of groups.values()) {
    rows.sort((a, b) => number(a, 'seq') - number(b, 'seq'));
    orderedRows(rows, 'authority', kind === 'membership' ? 11 : 1);
    originals.push(await authorityOriginal(rows, kind, recipient));
  }
  return originals;
}

async function authorityOriginal(
  rows: readonly Fields[],
  kind: 'membership' | 'chain' | 'wrap',
  recipient: string,
): Promise<Uint8Array> {
  requireValue(
    rows.every((row) => string(row, 'kind') === kind && string(row, 'recipient') === recipient),
  );
  const bytes = pieces(rows);
  requireValue(hex(await digest(bytes)) === string(rows[0], 'subject'));
  return bytes;
}

async function fields(
  document: SealedDocument,
  resource: Resource,
  scope: HostedScope,
): Promise<Fields> {
  const bytes = new Uint8Array(document.bytes);
  const expectedDigest = document.digest,
    target = { ...document.target };
  spaceId(scope.space);
  generatedId(scope.page);
  requireValue(/^[1-9][0-9]*$/.test(scope.epoch) && Number.isSafeInteger(Number(scope.epoch)));
  requireValue(target.resource === resource && bytes.length <= 393216);
  // The bound SDK owns canonical REST Values. Colab owns the declared routing/ID twin.
  requireValue(expectedDigest === hex(await digest(bytes)));
  const row = strictJson(bytes, 393216);
  exactKeys(row, KEYS[resource]);
  requireValue(
    string(row, 'space') === scope.space &&
      string(row, 'page') === scope.page &&
      number(row, 'epoch').toString() === scope.epoch,
  );
  requireValue(/^[A-Za-z0-9_+-]{1,128}$/.test(string(row, 'uid')));
  exactKeys(row.expiresAt, ['timestampValue']);
  requireValue(typeof row.expiresAt.timestampValue === 'string');
  const base = `${scope.space}_${scope.page}_${scope.epoch}`;
  let id: string;
  if (resource === 'log') {
    const stream = string(row, 'stream');
    generatedId(stream);
    id = `${base}_${stream}_${number(row, 'seq')}`;
  } else if (resource === 'checkpoints') {
    const object = string(row, 'object'),
      index = number(row, 'index', true),
      count = number(row, 'count');
    requireValue(
      /^[0-9a-f]{64}$/.test(object) &&
        index < count &&
        count <= 513 &&
        number(row, 'seq') === index + 1,
    );
    id = `${base}_${object}_${index}`;
  } else {
    const kind = string(row, 'kind'),
      recipient = string(row, 'recipient'),
      subject = string(row, 'subject'),
      sequence = number(row, 'seq'),
      count = number(row, 'count');
    requireValue(
      /^[0-9a-f]{64}$/.test(subject) &&
        sequence <= count &&
        count <= (kind === 'membership' ? 11 : 1),
    );
    requireValue(
      kind === 'membership' || kind === 'chain'
        ? recipient === ''
        : kind === 'wrap' &&
            (recipient.startsWith('link:')
              ? /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(
                  recipient.slice(5),
                )
              : /^[A-Za-z0-9_+-]{1,128}$/.test(recipient)),
    );
    id = `${base}_${kind}_${subject}_${sequence}`;
  }
  requireValue(target.id === id);
  return row;
}
function string(row: Fields, name: string): string {
  const value = row[name];
  exactKeys(value, ['stringValue']);
  requireValue(typeof value.stringValue === 'string');
  return value.stringValue;
}
function number(row: Fields, name: string, zero = false): number {
  const value = row[name];
  exactKeys(value, ['integerValue']);
  const decimal = value.integerValue;
  requireValue(typeof decimal === 'string' && /^(0|[1-9][0-9]*)$/.test(decimal));
  const parsed = Number(decimal);
  requireValue(Number.isSafeInteger(parsed) && parsed >= (zero ? 0 : 1));
  return parsed;
}
async function ordered(
  documents: readonly SealedDocument[],
  resource: 'checkpoints' | 'authority',
  scope: HostedScope,
  maximum: number,
): Promise<Fields[]> {
  requireValue(documents.length > 0 && documents.length <= maximum);
  const rows: Fields[] = [];
  for (const document of documents) rows.push(await fields(document, resource, scope));
  orderedRows(rows, resource, maximum);
  return rows;
}
function orderedRows(
  rows: readonly Fields[],
  resource: 'checkpoints' | 'authority',
  maximum: number,
): void {
  requireValue(rows.length > 0 && rows.length <= maximum);
  const first = rows[0];
  requireValue(number(first, 'count') === rows.length);
  const common =
    resource === 'checkpoints' ? ['object', 'uid'] : ['kind', 'subject', 'uid', 'recipient'];
  for (const [index, row] of rows.entries()) {
    requireValue(number(row, 'count') === rows.length && number(row, 'seq') === index + 1);
    requireValue(common.every((key) => string(row, key) === string(first, key)));
  }
}
function pieces(rows: readonly Fields[]): Uint8Array {
  const chunks = rows.map((row, index) => {
    const chunk = binary(string(row, 'bytes'), 32768);
    requireValue(chunk.length > 0 && (index + 1 === rows.length || chunk.length === 32768));
    return chunk;
  });
  return concat(...chunks);
}
async function original(
  bytes: Uint8Array,
  scope: HostedScope,
  kind: 'update' | 'checkpoint' | readonly ('html' | 'checkpoint' | 'fold')[],
): Promise<HostedOriginal> {
  const envelope = Envelope.fromJson(bytes),
    header = decodeHeader(envelope.header());
  requireValue(
    header.context.space === scope.space &&
      header.context.page === scope.page &&
      header.context.epoch === scope.epoch &&
      (typeof kind === 'string'
        ? header.context.kind === kind
        : kind.some((allowed) => allowed === header.context.kind)),
  );
  return { bytes, envelope, header, hash: await envelope.hash() };
}
