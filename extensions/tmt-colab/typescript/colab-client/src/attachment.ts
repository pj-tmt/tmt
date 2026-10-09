/** Bounded inert descriptors. Callers independently admit references, creator
 * keys and historical epoch eligibility before opening an asset. */
import {
  binary,
  copy,
  decimal,
  encodeBinary,
  equal,
  exactKeys,
  frame,
  generatedId,
  requireValue,
  spaceId,
  text,
  type Bytes,
} from './bytes.js';
import { digest, rootSecret, type RootSecret } from './crypto.js';
import { strictJson } from './json.js';
import { Envelope, header, decodeHeader, type Context } from './object.js';

export const DESCRIPTOR_BYTES = 2048;
export const ATTACHMENT_PLAINTEXT_BYTES = 8 * 1024 * 1024;
export const ATTACHMENT_PAYLOAD_BYTES = 12 * 1024 * 1024;
export const MESSAGE_ATTACHMENTS = 16;
export const DOCUMENT_ATTACHMENTS = 128;
export const MANIFEST_BYTES = DOCUMENT_ATTACHMENTS * DESCRIPTOR_BYTES + 1024;
export type AttachmentSource =
  | { kind: 'document'; sourceDigest: string }
  | { kind: 'message'; writerId: string; messageId: string; messageRevision: string };
export interface AttachmentDescriptor {
  version: 1;
  attachmentId: string;
  space: string;
  page: string;
  epoch: string;
  namespace: 'content' | 'own';
  objectId: string;
  authorDevice: string;
  membershipRevision: string;
  source: AttachmentSource;
  envelopeHash: string;
  signature: string;
  payloadSha256: string;
  payloadBytes: string;
  plaintextBytes: string;
  filename: string;
  mediaType: string;
}
const KEYS = [
  'version',
  'attachmentId',
  'space',
  'page',
  'epoch',
  'namespace',
  'objectId',
  'authorDevice',
  'membershipRevision',
  'source',
  'envelopeHash',
  'signature',
  'payloadSha256',
  'payloadBytes',
  'plaintextBytes',
  'filename',
  'mediaType',
];
const hex = (bytes: Uint8Array) =>
  Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
function hashValue(value: unknown): asserts value is string {
  requireValue(typeof value === 'string' && /^[0-9a-f]{64}$/.test(value));
}
function source(value: unknown): AttachmentSource {
  requireValue(value !== null && typeof value === 'object');
  const record = value as Record<string, unknown>;
  if (record.kind === 'document') {
    exactKeys(record, ['kind', 'sourceDigest']);
    hashValue(record.sourceDigest);
    return { kind: 'document', sourceDigest: record.sourceDigest };
  }
  exactKeys(value, ['kind', 'writerId', 'messageId', 'messageRevision']);
  requireValue(value.kind === 'message');
  generatedId(value.writerId as string);
  generatedId(value.messageId as string);
  decimal(value.messageRevision as string);
  return {
    kind: 'message',
    writerId: value.writerId as string,
    messageId: value.messageId as string,
    messageRevision: value.messageRevision as string,
  };
}
function sourceInput(value: AttachmentSource): Bytes {
  return value.kind === 'document'
    ? frame(text('document'), text(value.sourceDigest))
    : frame(
        text('message'),
        text(value.writerId),
        text(value.messageId),
        text(value.messageRevision),
      );
}
export function attachmentContext(value: AttachmentDescriptor): Context {
  return {
    space: value.space,
    page: value.page,
    epoch: value.epoch,
    kind: 'asset',
    namespace: value.namespace,
    authorDevice: value.authorDevice,
    membershipRevision: value.membershipRevision,
    streamSeq: '0',
    prevHash: new Uint8Array(32),
  };
}
/** Detached canonical fields. Input object order never changes the digest. */
export function attachmentDescriptor(value: unknown): AttachmentDescriptor {
  exactKeys(value, KEYS);
  const out = Object.fromEntries(
    KEYS.map((key) => [key, value[key]]),
  ) as unknown as AttachmentDescriptor;
  requireValue(out.version === 1);
  generatedId(out.attachmentId);
  header(attachmentContext(out), out.objectId);
  out.source = source(out.source);
  requireValue(out.namespace === (out.source.kind === 'document' ? 'content' : 'own'));
  hashValue(out.envelopeHash);
  hashValue(out.payloadSha256);
  binary(out.signature, 64, 64);
  requireValue(decimal(out.payloadBytes) <= BigInt(ATTACHMENT_PAYLOAD_BYTES));
  requireValue(decimal(out.plaintextBytes, true) <= BigInt(ATTACHMENT_PLAINTEXT_BYTES));
  requireValue(
    typeof out.filename === 'string' &&
      text(out.filename).length > 0 &&
      text(out.filename).length <= 255 &&
      !/[\p{Cc}]/u.test(out.filename),
  );
  requireValue(
    typeof out.mediaType === 'string' &&
      text(out.mediaType).length <= 128 &&
      /^[a-z0-9!#$&^_.+-]+\/[a-z0-9!#$&^_.+-]+$/.test(out.mediaType),
  );
  requireValue(text(JSON.stringify(out)).length <= DESCRIPTOR_BYTES);
  return out;
}
export function decodeAttachment(raw: Uint8Array): AttachmentDescriptor {
  return attachmentDescriptor(strictJson(raw, DESCRIPTOR_BYTES, true));
}
export function attachmentJson(value: AttachmentDescriptor): Bytes {
  return text(JSON.stringify(attachmentDescriptor(value)));
}
/** The base a message-source attachment is fenced by: the space, page and epoch it was sealed
 * under, the membership head it was admitted by and its author. It covers no stream cut, so a
 * foreign write while the upload runs leaves it valid; a membership or epoch change does not.
 * Its label is distinct, so a page revision (the base of a document-source attachment) can never
 * satisfy a fence or the reverse. Mirrors native `message_fence`; both consume
 * `attachment-fence-v1.json`. */
export async function messageFence(value: {
  space: string;
  page: string;
  epoch: string;
  membershipRevision: string;
  membershipHash: Bytes;
  author: string;
}): Promise<string> {
  spaceId(value.space);
  generatedId(value.page);
  decimal(value.epoch);
  decimal(value.membershipRevision);
  requireValue(value.membershipHash.length === 32);
  generatedId(value.author);
  const hash = await digest(
    frame(
      text('tmt-colab-attachment-fence-v1'),
      text('1'),
      text(value.space),
      text(value.page),
      text(value.epoch),
      text(value.membershipRevision),
      value.membershipHash,
      text(value.author),
    ),
  );
  return `v1:${Array.from(hash, (b) => b.toString(16).padStart(2, '0')).join('')}`;
}
export function attachmentInput(value: AttachmentDescriptor): Bytes {
  const v = attachmentDescriptor(value);
  return frame(
    text('tmt-colab-attachment-v1'),
    text('1'),
    ...KEYS.slice(1).map((key) =>
      key === 'source'
        ? sourceInput(v.source)
        : text(v[key as keyof AttachmentDescriptor] as string),
    ),
  );
}
export async function attachmentHash(value: AttachmentDescriptor): Promise<Bytes> {
  return digest(attachmentInput(value));
}
export function attachmentList(
  value: unknown,
  limit: number,
  space?: string,
  page?: string,
): AttachmentDescriptor[] {
  requireValue(Array.isArray(value) && value.length <= limit);
  const out = value.map(attachmentDescriptor);
  const ids = new Set<string>();
  for (const item of out) {
    requireValue(
      (space === undefined || item.space === space) &&
        (page === undefined || item.page === page) &&
        !ids.has(item.attachmentId),
    );
    ids.add(item.attachmentId);
  }
  return out;
}
export async function openAttachment(
  value: AttachmentDescriptor,
  payload: Uint8Array,
  admitted: Context,
  secret: RootSecret,
  publicKey: Uint8Array,
): Promise<Bytes> {
  // Snapshot all mutable input before the first await, matching Envelope.open.
  requireValue(payload instanceof Uint8Array && payload.length <= ATTACHMENT_PAYLOAD_BYTES);
  const v = attachmentDescriptor(value),
    raw = copy(payload),
    expected = attachmentContext(v),
    root = rootSecret(secret),
    key = copy(publicKey, 32);
  requireValue(
    equal(header(admitted, v.objectId), header(expected, v.objectId)) &&
      BigInt(raw.length) === decimal(v.payloadBytes),
  );
  const env = Envelope.fromJson(raw),
    h = decodeHeader(env.header());
  requireValue(BigInt(env.ciphertext().length) === decimal(v.plaintextBytes, true) + 16n);
  requireValue(
    h.objectId === v.objectId &&
      equal(env.header(), header(expected, v.objectId)) &&
      encodeBinary(env.signature()) === v.signature,
  );
  requireValue(
    hex(await digest(raw)) === v.payloadSha256 && hex(await env.hash()) === v.envelopeHash,
  );
  const plain = await env.open(expected, root, key);
  if (BigInt(plain.length) !== decimal(v.plaintextBytes, true)) {
    plain.fill(0);
    throw new Error('Invalid attachment plaintext length');
  }
  return plain;
}
export interface AttachmentManifest {
  version: 1;
  space: string;
  page: string;
  snapshotId: string;
  authorDevice: string;
  membershipRevision: string;
  sourceDigest: string;
  attachments: AttachmentDescriptor[];
}
export function attachmentManifest(value: unknown): AttachmentManifest {
  const keys = [
    'version',
    'space',
    'page',
    'snapshotId',
    'authorDevice',
    'membershipRevision',
    'sourceDigest',
    'attachments',
  ];
  exactKeys(value, keys);
  const v = Object.fromEntries(
    keys.map((key) => [key, value[key]]),
  ) as unknown as AttachmentManifest;
  requireValue(v.version === 1);
  spaceId(v.space);
  generatedId(v.page);
  generatedId(v.authorDevice);
  decimal(v.membershipRevision);
  generatedId(v.snapshotId);
  hashValue(v.sourceDigest);
  v.attachments = attachmentList(v.attachments, DOCUMENT_ATTACHMENTS, v.space, v.page);
  return v;
}
export function decodeAttachmentManifest(raw: Uint8Array): AttachmentManifest {
  return attachmentManifest(strictJson(raw, MANIFEST_BYTES, true));
}
export function attachmentManifestInput(value: AttachmentManifest): Bytes {
  const v = attachmentManifest(value),
    count = new Uint8Array(4);
  new DataView(count.buffer).setUint32(0, v.attachments.length);
  const entries = frame(...v.attachments.map(attachmentInput)),
    list = new Uint8Array(4 + entries.length);
  list.set(count);
  list.set(entries, 4);
  return frame(
    text('tmt-colab-attachment-manifest-v1'),
    text('1'),
    text(v.space),
    text(v.page),
    text(v.snapshotId),
    text(v.authorDevice),
    text(v.membershipRevision),
    text(v.sourceDigest),
    list,
  );
}
export async function attachmentManifestHash(value: AttachmentManifest): Promise<Bytes> {
  return digest(attachmentManifestInput(value));
}

export const REFERENCE_BYTES = 2048;
/** Only the original creator's cut-admitted positive-sequence own stream can
 * make this inert record a creation witness. JSON fields alone never do. */
export interface AttachmentPublication {
  version: 1;
  kind: 'attachment-publication';
  spaceId: string;
  pageId: string;
  epoch: string;
  senderDevice: string;
  membershipRevision: string;
  attachmentId: string;
  descriptorHash: string;
  source: AttachmentSource;
  baseRevision: string;
}
function attachmentRevision(value: unknown): asserts value is string {
  requireValue(typeof value === 'string' && /^v1:[0-9a-f]{64}$/.test(value));
}
export function attachmentPublication(value: unknown): AttachmentPublication {
  const keys = [
    'version',
    'kind',
    'spaceId',
    'pageId',
    'epoch',
    'senderDevice',
    'membershipRevision',
    'attachmentId',
    'descriptorHash',
    'source',
    'baseRevision',
  ];
  exactKeys(value, keys);
  const v = Object.fromEntries(
    keys.map((key) => [key, value[key]]),
  ) as unknown as AttachmentPublication;
  requireValue(v.version === 1 && v.kind === 'attachment-publication');
  spaceId(v.spaceId);
  for (const id of [v.pageId, v.senderDevice, v.attachmentId]) generatedId(id);
  decimal(v.epoch);
  decimal(v.membershipRevision);
  hashValue(v.descriptorHash);
  v.source = source(v.source);
  attachmentRevision(v.baseRevision);
  requireValue(text(JSON.stringify(v)).length <= REFERENCE_BYTES);
  return v;
}
export function decodeAttachmentPublication(raw: Uint8Array): AttachmentPublication {
  return attachmentPublication(strictJson(raw, REFERENCE_BYTES, true));
}
export async function publicationMatchesDescriptor(
  value: AttachmentPublication,
  descriptor: AttachmentDescriptor,
): Promise<void> {
  const v = attachmentPublication(value),
    d = attachmentDescriptor(descriptor);
  requireValue(
    v.spaceId === d.space &&
      v.pageId === d.page &&
      v.epoch === d.epoch &&
      v.senderDevice === d.authorDevice &&
      v.membershipRevision === d.membershipRevision &&
      v.attachmentId === d.attachmentId &&
      v.descriptorHash === hex(await attachmentHash(d)) &&
      equal(sourceInput(v.source), sourceInput(d.source)),
  );
}
export type AttachmentSelector =
  | {
      kind: 'document-current';
      attachmentId: string;
      descriptorHash: string;
      contentRevision: string;
    }
  | {
      kind: 'message';
      writerId: string;
      messageId: string;
      messageRevision: string;
      attachmentId: string;
      descriptorHash: string;
    };
export function attachmentSelector(value: unknown): AttachmentSelector {
  requireValue(value !== null && typeof value === 'object');
  const v = value as Record<string, unknown>;
  let keys: string[];
  if (v.kind === 'document-current') {
    keys = ['kind', 'attachmentId', 'descriptorHash', 'contentRevision'];
    exactKeys(v, keys);
    attachmentRevision(v.contentRevision);
  } else {
    keys = ['kind', 'writerId', 'messageId', 'messageRevision', 'attachmentId', 'descriptorHash'];
    exactKeys(v, keys);
    requireValue(v.kind === 'message');
    generatedId(v.writerId as string);
    generatedId(v.messageId as string);
    decimal(v.messageRevision as string);
  }
  generatedId(v.attachmentId as string);
  hashValue(v.descriptorHash);
  const out = Object.fromEntries(keys.map((key) => [key, v[key]])) as AttachmentSelector;
  requireValue(text(JSON.stringify(out)).length <= REFERENCE_BYTES);
  return out;
}
export function decodeAttachmentSelector(raw: Uint8Array): AttachmentSelector {
  return attachmentSelector(strictJson(raw, REFERENCE_BYTES, true));
}
