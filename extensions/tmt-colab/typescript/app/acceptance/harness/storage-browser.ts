// Test-only browser consumer. It imports the shipped owners; the fixture adds no
// admission, backend or wire result, and never enters the production app build.
import * as c from '@tmt/colab-client';
import { Admission } from '../../src/admission.js';
import { discover } from '../../src/bootstrap.js';
import { Connection } from '../../src/connection.js';
import { register, remoteSdk, verifyRegistration } from '../../src/registration.js';
import { FrozenAttachmentUpload } from '../../src/attachment-channel.js';
import {
  AdmittedAttachmentRead,
  currentBase,
  prepareAttachmentPublication,
  attachmentNamespace,
} from '../../src/attachments.js';
import { Writer } from '../../src/writer.js';

let connection: Connection | undefined;
let writer: Writer | undefined;
let original: FrozenAttachmentUpload | undefined;
let selector: c.attachment.AttachmentSelector | undefined;
let expected: Uint8Array | undefined;
let readDescriptor: c.attachment.AttachmentDescriptor | undefined;
const hex = (bytes: Uint8Array) =>
  [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('');
const deadline = () => performance.now() + 15_000;
function current() {
  if (!connection?.active) throw new Error('Storage fixture is disconnected');
  return connection;
}
function captured() {
  if (!original) throw new Error('No frozen original');
  return original;
}
export async function connect(mountAddress: string, pageId: string) {
  disconnect();
  const mount = new URL(mountAddress),
    registration = await register(mount, await remoteSdk()),
    boot = await discover(mount, (space, owner) => verifyRegistration(registration, space, owner)),
    page = boot.pages.find((value) => value.pageId === pageId);
  if (!page) throw new Error('Missing fixture page');
  const admission = new Admission(boot.space, pageId, page.epoch, boot.owner, registration);
  await admission.restore();
  const opened = new Connection(
    admission,
    mount,
    page.sharing,
    () => {},
    () => {},
  );
  connection = opened;
  await opened.ready;
  writer = new Writer(
    `writer:${boot.space}:${pageId}:${page.epoch}:${registration.deviceId}`,
    async () => current(),
  );
  return {
    source: (await opened.attachmentSnapshot()).projection.source,
    device: registration.deviceId,
    epoch: page.epoch,
  };
}
export function disconnect() {
  writer?.close();
  writer = undefined;
  connection?.close();
  connection = undefined;
}
export async function config() {
  return current().attachmentObjects.config(deadline());
}
export async function capture() {
  const peer = current(),
    a = peer.admission,
    device = a.registration.deviceId,
    messageId = crypto.randomUUID();
  expected = c.text('Routed encrypted bytes '.repeat(3500));
  const envelope = await c.Envelope.seal(
    {
      space: a.space,
      page: a.page,
      epoch: a.epoch,
      kind: 'asset',
      namespace: 'own',
      authorDevice: device,
      membershipRevision: a.head!.revision.toString(),
      streamSeq: '0',
      prevHash: new Uint8Array(32),
    },
    a.readRoot(a.epoch),
    a.registration.keys.sign,
    expected,
  );
  const raw = envelope.toJson(),
    descriptor = c.attachment.attachmentDescriptor({
      version: 1,
      attachmentId: crypto.randomUUID(),
      space: a.space,
      page: a.page,
      epoch: a.epoch,
      namespace: 'own',
      objectId: c.decodeHeader(envelope.header()).objectId,
      authorDevice: device,
      membershipRevision: a.head!.revision.toString(),
      source: { kind: 'message', writerId: device, messageId, messageRevision: '1' },
      envelopeHash: hex(await envelope.hash()),
      signature: c.encodeBinary(envelope.signature()),
      payloadSha256: hex(await c.digest(raw)),
      payloadBytes: String(raw.length),
      plaintextBytes: String(expected.length),
      filename: 'routed.txt',
      mediaType: 'text/plain',
    });
  readDescriptor = descriptor;
  original = await FrozenAttachmentUpload.capture(
    descriptor,
    // A message attachment is fenced by the membership head, not by the page revision.
    await currentBase(descriptor, await peer.attachmentSnapshot()),
    crypto.randomUUID(),
    raw,
  );
  selector = c.attachment.attachmentSelector({
    kind: 'message',
    writerId: device,
    messageId,
    messageRevision: '1',
    attachmentId: descriptor.attachmentId,
    descriptorHash: hex(await c.attachment.attachmentHash(descriptor)),
  });
  return {
    transferId: original.transferId,
    bytes: raw.length,
    parts: Math.ceil(raw.length / 32768),
    digest: descriptor.payloadSha256,
    plaintextDigest: hex(await c.digest(expected)),
  };
}
export async function begin() {
  return current().attachmentObjects.begin(captured(), deadline());
}
export async function status() {
  return current().attachmentObjects.status(captured(), deadline());
}
export async function part(index: number) {
  return current().attachmentObjects.part(captured(), index, deadline());
}
export async function commit() {
  return current().attachmentObjects.commit(captured(), deadline());
}
export async function prematureRead() {
  const peer = current();
  if (!selector) throw new Error('No selector');
  const reader = peer.attachmentObjects.read(selector, captured().descriptor);
  try {
    await reader.readCommitted(
      await attachmentNamespace(peer.admission.space, peer.admission.page),
      Uint8Array.from(captured().descriptor.objectId.match(/../g)!, (value) => parseInt(value, 16)),
      deadline(),
    );
    return 'disclosed';
  } catch {
    return 'refused';
  }
}
export async function publish() {
  const peer = current(),
    value = captured(),
    d = value.descriptor;
  if (!writer || !selector || d.source.kind !== 'message') throw new Error('No publication owner');
  const owner = {
      snapshot: (epoch?: string, bound?: number) => peer.attachmentSnapshot(epoch, bound),
    },
    proof = await prepareAttachmentPublication(
      owner,
      d,
      value.base,
      peer.attachmentObjects.verifier(value),
      deadline(),
      'private',
    );
  await writer.submitOwnRecords([
    proof,
    {
      root: 'messages',
      key: `${d.source.messageId}:1`,
      value: JSON.parse(
        JSON.stringify({
          version: 1,
          kind: 'comment',
          spaceId: d.space,
          pageId: d.page,
          epoch: d.epoch,
          senderDevice: d.authorDevice,
          revision: '1',
          deleted: false,
          deviceName: 'Storage fixture',
          at: String(Date.now()),
          messageId: d.source.messageId,
          thread: { writer: d.authorDevice, id: d.authorDevice },
          body: 'Routed attachment',
          attachments: [d],
        }),
      ),
    },
  ]);
  return read();
}
export async function read() {
  const peer = current(),
    d = readDescriptor;
  if (!d || !selector || !expected) throw new Error('No reference');
  const owner = {
      snapshot: (epoch?: string, bound?: number) => peer.attachmentSnapshot(epoch, bound),
    },
    admitted = await AdmittedAttachmentRead.capture(owner, selector, deadline(), 'private'),
    plaintext = await admitted.disclose(peer.attachmentObjects.read(selector, d), 'private');
  try {
    if (!c.equal(plaintext, expected)) throw new Error('Routed plaintext differs');
    return { bytes: plaintext.length, digest: hex(await c.digest(plaintext)) };
  } finally {
    plaintext.fill(0);
  }
}

export function binding() {
  if (!selector || !readDescriptor || !expected) throw new Error('No reference binding');
  return { descriptor: readDescriptor, selector };
}
export function reference(value: { descriptor: unknown; selector: unknown }) {
  readDescriptor = c.attachment.attachmentDescriptor(value.descriptor);
  selector = c.attachment.attachmentSelector(value.selector);
  // Known fixture plaintext is an oracle, never a native response or admission.
  expected = c.text('Routed encrypted bytes '.repeat(3500));
}
