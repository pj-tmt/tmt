import {
  attachment,
  binary,
  decimal,
  digest,
  encodeBinary,
  equal,
  exactKeys,
  generatedId,
  requireValue,
  text,
} from '@tmt/colab-client';

import type { AttachmentHistorySource } from './attachment-history.js';
import { attachmentNamespace, type CommittedObjectVerifier } from './attachments.js';

const POLICY_BYTES = 2048;
export const ATTACHMENT_CHUNK_BYTES = 32 * 1024;
function bounded(value: unknown): Uint8Array {
  const bytes = text(JSON.stringify(value));
  requireValue(bytes.length <= POLICY_BYTES);
  return bytes;
}
/** Exact minimal opaque binding persisted by Remote. Labels and the complete
 * descriptor remain in Colab's encrypted content/captured transfer owner. */
export async function uploadPolicy(raw: attachment.AttachmentDescriptor): Promise<Uint8Array> {
  const d = attachment.attachmentDescriptor(raw);
  return bounded({
    version: 1,
    space: d.space,
    page: d.page,
    epoch: d.epoch,
    target: d.source,
    attachmentId: d.attachmentId,
    descriptorHash: hex(await attachment.attachmentHash(d)),
    objectId: d.objectId,
    envelopeHash: d.envelopeHash,
    payloadSha256: d.payloadSha256,
    payloadBytes: d.payloadBytes,
    plaintextBytes: d.plaintextBytes,
  });
}
export function readPolicy(
  raw: attachment.AttachmentDescriptor,
  peerEpoch: string,
  selector: attachment.AttachmentSelector,
): Uint8Array {
  const d = attachment.attachmentDescriptor(raw),
    reference = attachment.attachmentSelector(selector);
  decimal(peerEpoch);
  return bounded({ version: 1, space: d.space, page: d.page, epoch: peerEpoch, reference });
}
function hex(bytes: Uint8Array) {
  return [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
}
/** Immutable byte/target retention, not authority or proof of commit. The sync
 * peer and actual Session are freshly admitted by native callbacks for every call.
 * The caller allocates transferId once; uncertain recovery keeps this same owner. */
export class FrozenAttachmentUpload {
  private constructor(
    readonly transferId: string,
    private readonly capturedDescriptor: attachment.AttachmentDescriptor,
    readonly base: string,
    private readonly ciphertext: Uint8Array,
    private readonly capturedPolicy: Uint8Array,
  ) {}
  static async capture(
    raw: attachment.AttachmentDescriptor,
    base: string,
    transferId: string,
    bytes: Uint8Array,
  ): Promise<FrozenAttachmentUpload> {
    generatedId(transferId);
    requireValue(/^v1:[0-9a-f]{64}$/.test(base));
    const descriptor = attachment.attachmentDescriptor(raw),
      ciphertext = bytes.slice();
    requireValue(
      ciphertext.length === Number(decimal(descriptor.payloadBytes)) &&
        ciphertext.length <= attachment.ATTACHMENT_PAYLOAD_BYTES,
    );
    requireValue(hex(await digest(ciphertext)) === descriptor.payloadSha256);
    return new FrozenAttachmentUpload(
      transferId,
      descriptor,
      base,
      ciphertext,
      await uploadPolicy(descriptor),
    );
  }
  get descriptor(): attachment.AttachmentDescriptor {
    return attachment.attachmentDescriptor(this.capturedDescriptor);
  }
  /** Returns a copy; a changed draft cannot rewrite an original's status input. */
  policy(): Uint8Array {
    return this.capturedPolicy.slice();
  }
  part(index: number): Uint8Array {
    requireValue(Number.isSafeInteger(index) && index >= 0);
    const start = index * ATTACHMENT_CHUNK_BYTES;
    requireValue(Number.isSafeInteger(start) && start < this.ciphertext.length);
    return this.ciphertext.slice(start, start + ATTACHMENT_CHUNK_BYTES);
  }
}

export type AttachmentObjectRequest =
  | { method: 'config' }
  | { method: 'history'; historyId: string; epoch: string }
  | { method: 'historynext' | 'historycancel'; historyId: string }
  | {
      method: 'begin' | 'status';
      transferId: string;
      descriptor: attachment.AttachmentDescriptor;
      base: string;
    }
  | { method: 'part'; transferId: string; index: number; bytes: string }
  | { method: 'commit' | 'discard'; transferId: string }
  | {
      method: 'verify';
      transferId: string;
      descriptor: attachment.AttachmentDescriptor;
      base: string;
      offset: number;
      count: number;
    }
  | { method: 'read'; selector: attachment.AttachmentSelector; offset: number; count: number };
export type AttachmentObjectOutcome =
  | { ok: Record<string, unknown> }
  | { error: { code: string; limit?: string } };
/** An exact correlated reply is still not a publication permit. Each consumer
 * checks its expected result/binding and the existing native/browser admission. */
export function objectOutcome(
  raw: unknown,
  method: AttachmentObjectRequest['method'],
): AttachmentObjectOutcome {
  requireValue(raw !== null && typeof raw === 'object' && !Array.isArray(raw));
  const result = raw as Record<string, unknown>;
  if (Object.hasOwn(result, 'error')) {
    exactKeys(result, ['error']);
    requireValue(
      result.error !== null && typeof result.error === 'object' && !Array.isArray(result.error),
    );
    const error = result.error as Record<string, unknown>;
    requireValue(
      ['denied', 'unavailable', 'invalid', 'conflict', 'capacity', 'not-found', 'unknown'].includes(
        error.code as string,
      ),
    );
    exactKeys(error, error.code === 'capacity' ? ['code', 'limit'] : ['code']);
    if (error.code === 'capacity')
      requireValue(
        [
          'namespace-bytes',
          'extension-bytes',
          'installation-bytes',
          'namespace-entries',
          'extension-entries',
          'installation-entries',
          'active-intents',
          'retained-extension',
          'retained-installation',
          'requests',
        ].includes(error.limit as string),
      );
    requireValue(
      error.code !== 'unknown' || ['begin', 'part', 'commit', 'discard'].includes(method),
    );
    requireValue(error.code !== 'not-found' || method === 'read' || method === 'verify');
    return { error: error as { code: string; limit?: string } };
  }
  exactKeys(result, ['ok']);
  requireValue(result.ok !== null && typeof result.ok === 'object' && !Array.isArray(result.ok));
  const ok = result.ok as Record<string, unknown>;
  const allowed: Record<AttachmentObjectRequest['method'], readonly string[]> = {
    config: ['config'],
    history: ['history'],
    historynext: ['history'],
    historycancel: ['history-end'],
    begin: ['pending', 'committed'],
    part: ['progress'],
    commit: ['committed'],
    status: ['pending', 'committed', 'state'],
    discard: ['state'],
    read: ['read'],
    verify: ['read'],
  };
  requireValue(allowed[method].includes(ok.result as string));
  switch (ok.result) {
    case 'history':
      exactKeys(ok, ['result', 'frame', 'more']);
      requireValue(
        ok.frame !== null &&
          typeof ok.frame === 'object' &&
          !Array.isArray(ok.frame) &&
          typeof ok.more === 'boolean',
      );
      requireValue(text(JSON.stringify(ok.frame)).length <= 64 * 1024);
      break;
    case 'history-end':
      exactKeys(ok, ['result']);
      break;
    case 'pending':
      exactKeys(ok, [
        'result',
        'nextIndex',
        'received',
        ...(method === 'status' ? ['expiresAtMs'] : []),
      ]);
      if (method === 'status')
        requireValue(Number.isSafeInteger(ok.expiresAtMs) && (ok.expiresAtMs as number) > 0);
    // fall through: both forms carry canonical progress counters.
    case 'progress':
      if (ok.result === 'progress') exactKeys(ok, ['result', 'nextIndex', 'received']);
      requireValue(
        Number.isSafeInteger(ok.nextIndex) &&
          (ok.nextIndex as number) >= 0 &&
          (ok.nextIndex as number) <= 0xffff_ffff &&
          Number.isSafeInteger(ok.received) &&
          (ok.received as number) >= 0 &&
          (ok.received as number) <= attachment.ATTACHMENT_PAYLOAD_BYTES,
      );
      break;
    case 'committed':
      exactKeys(ok, ['result', 'opaqueKey', 'payloadSha256', 'payloadBytes']);
      binary(ok.opaqueKey, 32, 32);
      requireValue(
        typeof ok.payloadSha256 === 'string' &&
          /^[0-9a-f]{64}$/.test(ok.payloadSha256) &&
          Number.isSafeInteger(ok.payloadBytes) &&
          (ok.payloadBytes as number) > 0 &&
          (ok.payloadBytes as number) <= attachment.ATTACHMENT_PAYLOAD_BYTES,
      );
      break;
    case 'state':
      exactKeys(ok, ['result', 'state']);
      requireValue(
        ['expired', 'discarded', 'unavailable', 'not-observed'].includes(ok.state as string),
      );
      break;
    case 'read':
      exactKeys(ok, ['result', 'offset', 'totalBytes', 'bytes']);
      requireValue(
        Number.isSafeInteger(ok.offset) &&
          (ok.offset as number) >= 0 &&
          Number.isSafeInteger(ok.totalBytes) &&
          (ok.totalBytes as number) > 0 &&
          (ok.totalBytes as number) <= attachment.ATTACHMENT_PAYLOAD_BYTES,
      );
      binary(ok.bytes, ATTACHMENT_CHUNK_BYTES);
      break;
    case 'config':
      exactKeys(ok, ['result', 'projection', 'backend', 'capabilities', 'limits']);
      requireValue(ok.projection === 'browser');
      exactKeys(ok.backend, ['id', 'source', 'editable']);
      exactKeys(ok.capabilities, ['immutableCreate', 'chunkedRead', 'recoverByOriginalId']);
      exactKeys(ok.limits, ['payloadBytes', 'chunkBytes']);
      break;
  }
  return { ok };
}

export type AttachmentObjectCall = (
  request: AttachmentObjectRequest,
  deadline: number,
) => Promise<AttachmentObjectOutcome>;
/** No upload replay or replacement identity: uncertain callers ask status using
 * this same frozen original. Commit returns data only, never publishes content. */
export class AttachmentObjectChannel {
  constructor(private readonly call: AttachmentObjectCall) {}
  historySource(): AttachmentHistorySource {
    const call = this.call;
    return {
      async *frames(epoch, deadline) {
        decimal(epoch);
        const historyId = crypto.randomUUID();
        let finished = false;
        try {
          let request: AttachmentObjectRequest = { method: 'history', historyId, epoch };
          for (;;) {
            requireValue(performance.now() < deadline);
            const result = await call(request, deadline);
            requireValue(
              performance.now() < deadline &&
                'ok' in result &&
                result.ok.result === 'history' &&
                typeof result.ok.more === 'boolean',
            );
            finished = result.ok.more === false;
            yield JSON.stringify(result.ok.frame);
            if (finished) return;
            request = { method: 'historynext', historyId };
          }
        } finally {
          // Best-effort explicit release; native also expires the original
          // absolute stream deadline and joins it on peer/channel close.
          if (!finished && performance.now() < deadline) {
            try {
              await call({ method: 'historycancel', historyId }, deadline);
            } catch {}
          }
        }
      },
    };
  }
  config(deadline: number) {
    return this.call({ method: 'config' }, deadline);
  }
  begin(original: FrozenAttachmentUpload, deadline: number) {
    return this.call(
      {
        method: 'begin',
        transferId: original.transferId,
        descriptor: original.descriptor,
        base: original.base,
      },
      deadline,
    );
  }
  status(original: FrozenAttachmentUpload, deadline: number) {
    return this.call(
      {
        method: 'status',
        transferId: original.transferId,
        descriptor: original.descriptor,
        base: original.base,
      },
      deadline,
    );
  }
  part(original: FrozenAttachmentUpload, index: number, deadline: number) {
    return this.call(
      {
        method: 'part',
        transferId: original.transferId,
        index,
        bytes: encodeBinary(original.part(index)),
      },
      deadline,
    );
  }
  commit(original: FrozenAttachmentUpload, deadline: number) {
    return this.call({ method: 'commit', transferId: original.transferId }, deadline);
  }
  discard(original: FrozenAttachmentUpload, deadline: number) {
    return this.call({ method: 'discard', transferId: original.transferId }, deadline);
  }
  verifier(original: FrozenAttachmentUpload): CommittedObjectVerifier {
    return this.reader(original.descriptor, (offset, count) => ({
      method: 'verify',
      transferId: original.transferId,
      descriptor: original.descriptor,
      base: original.base,
      offset,
      count,
    }));
  }
  read(
    raw: attachment.AttachmentSelector,
    descriptor: attachment.AttachmentDescriptor,
  ): CommittedObjectVerifier {
    const selector = attachment.attachmentSelector(raw);
    return this.reader(descriptor, (offset, count) => ({
      method: 'read',
      selector,
      offset,
      count,
    }));
  }
  private reader(
    raw: attachment.AttachmentDescriptor,
    request: (offset: number, count: number) => AttachmentObjectRequest,
  ): CommittedObjectVerifier {
    const descriptor = attachment.attachmentDescriptor(raw);
    return {
      readCommitted: async (namespace, key, deadline) => {
        requireValue(
          equal(namespace, await attachmentNamespace(descriptor.space, descriptor.page)) &&
            hex(key) === descriptor.objectId,
        );
        const length = Number(decimal(descriptor.payloadBytes));
        requireValue(length > 0 && length <= attachment.ATTACHMENT_PAYLOAD_BYTES);
        const bytes = new Uint8Array(length);
        for (let offset = 0; offset < length;) {
          requireValue(performance.now() < deadline);
          const count = Math.min(ATTACHMENT_CHUNK_BYTES, length - offset),
            outcome = await this.call(request(offset, count), deadline);
          requireValue('ok' in outcome);
          const ok = outcome.ok;
          exactKeys(ok, ['result', 'offset', 'totalBytes', 'bytes']);
          requireValue(ok.result === 'read' && ok.offset === offset && ok.totalBytes === length);
          const chunk = binary(ok.bytes, count, count);
          bytes.set(chunk, offset);
          offset += count;
        }
        requireValue(
          performance.now() < deadline && hex(await digest(bytes)) === descriptor.payloadSha256,
        );
        return bytes;
      },
    };
  }
}
