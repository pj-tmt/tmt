/** Exact root-signed membership chain; persist the returned head before dependent state. */
import {
  binary,
  copy,
  decimal,
  decodeText,
  equal,
  encodeBinary,
  exactKeys,
  fields,
  frame,
  generatedId,
  requireValue,
  spaceId,
  text,
  type Bytes,
} from './bytes.js';
import { operation } from './auth.js';
import { deriveSpaceId, digest, strictVerify, validEdPoint } from './crypto.js';
import { strictJson } from './json.js';
import * as payload from './payload.js';
export interface Header {
  space: string;
  revision: string;
  previousHash: Uint8Array;
  operation: string;
  payloadDigest: Uint8Array;
}
export interface OwnerMember {
  id: string;
  signingKey: Uint8Array;
  encryptionKey: Uint8Array;
}
export interface Head {
  revision: bigint;
  hash: Uint8Array;
  ownerMember: OwnerMember;
}
export interface Verified {
  header: Header;
  payload: payload.Payload;
  head: Head;
}
export function input(v: Header): Bytes {
  spaceId(v.space);
  const revision = decimal(v.revision);
  operation(v.operation);
  const previous = copy(v.previousHash, 32);
  requireValue(revision !== 1n || previous.every((n) => n === 0));
  return frame(
    text('tmt-colab-membership-v1'),
    text('1'),
    text(v.space),
    text(v.revision),
    previous,
    text(v.operation),
    copy(v.payloadDigest, 32),
  );
}
export function decode(raw: Uint8Array): Header {
  const f = fields(raw, 7);
  requireValue(decodeText(f[0]) === 'tmt-colab-membership-v1' && decodeText(f[1]) === '1');
  const v = {
    space: decodeText(f[2]),
    revision: decodeText(f[3]),
    previousHash: copy(f[4], 32),
    operation: decodeText(f[5]),
    payloadDigest: copy(f[6], 32),
  };
  requireValue(equal(input(v), raw));
  return v;
}
function snapshot(head: Head): Head {
  requireValue(typeof head.revision === 'bigint');
  decimal(String(head.revision));
  generatedId(head.ownerMember.id);
  requireValue(validEdPoint(head.ownerMember.signingKey));
  return {
    revision: head.revision,
    hash: copy(head.hash, 32),
    ownerMember: {
      id: head.ownerMember.id,
      signingKey: copy(head.ownerMember.signingKey, 32),
      encryptionKey: copy(head.ownerMember.encryptionKey, 32),
    },
  };
}
export class Envelope {
  #statement: Bytes;
  #payload: Bytes;
  #signature: Bytes;
  private constructor(statement: Bytes, bytes: Bytes, signature: Bytes) {
    this.#statement = statement;
    this.#payload = bytes;
    this.#signature = signature;
  }
  static fromJson(raw: Uint8Array): Envelope {
    const v = strictJson(raw, Math.floor(((payload.MAX_BYTES + 1024) * 4) / 3) + 2048);
    exactKeys(v, ['statement', 'payload', 'signature']);
    const statement = binary(v.statement, 1024),
      h = decode(statement),
      bytes = binary(v.payload, payload.MAX_BYTES);
    payload.decode(h.operation, bytes);
    return new Envelope(statement, bytes, binary(v.signature, 64, 64));
  }
  toJson(): Bytes {
    return text(
      JSON.stringify({
        statement: encodeBinary(this.#statement),
        payload: encodeBinary(this.#payload),
        signature: encodeBinary(this.#signature),
      }),
    );
  }
  hash(): Promise<Bytes> {
    return digest(frame(text('tmt-colab-membership-hash-v1'), this.#statement, this.#signature));
  }
  /** Caller supplies pinned URL space/root and its highest durable verified head, never backend hints. */
  async verifyNext(space: string, owner: Uint8Array, previous: Head | null): Promise<Verified> {
    const key = copy(owner, 32),
      prior = previous === null ? null : snapshot(previous),
      h = decode(this.#statement);
    requireValue(
      h.space === space &&
        (await deriveSpaceId(key)) === space &&
        equal(await digest(this.#payload), h.payloadDigest),
    );
    const revision = decimal(h.revision);
    requireValue(
      prior === null
        ? revision === 1n && h.previousHash.every((n) => n === 0)
        : prior.revision + 1n === revision && equal(prior.hash, h.previousHash),
    );
    requireValue(await strictVerify(key, this.#signature, this.#statement));
    const decoded = payload.decode(h.operation, this.#payload);
    const ownerMember = ownerBinding(decoded, prior);
    if (decoded.operation === 'epoch.advance') {
      const v = decoded.value;
      requireValue(v.baseline.membershipRevision === h.revision);
      for (const cut of v.cuts)
        requireValue(cut.pageId === v.pageId && decimal(cut.epoch) < decimal(v.epoch));
      for (const w of v.wraps) {
        requireValue(w.header().space === space);
        await w.verifyOwner(key);
      }
    }
    return {
      header: h,
      payload: decoded,
      head: { revision, hash: await this.hash(), ownerMember },
    };
  }
}
function ownerBinding(value: payload.Payload, prior: Head | null): OwnerMember {
  if (prior === null) {
    requireValue(value.operation === 'member.add' && value.value.role === 'editor');
    return {
      id: value.value.memberId,
      signingKey: binary(value.value.signKey, 32, 32),
      encryptionKey: binary(value.value.encKey, 32, 32),
    };
  }
  const member = prior.ownerMember;
  if (value.operation === 'member.add')
    requireValue(
      value.value.memberId !== member.id &&
        !equal(binary(value.value.signKey, 32, 32), member.signingKey) &&
        !equal(binary(value.value.encKey, 32, 32), member.encryptionKey),
    );
  if (value.operation === 'member.remove' || value.operation === 'member.role')
    requireValue(value.value.memberId !== member.id);
  return {
    id: member.id,
    signingKey: copy(member.signingKey, 32),
    encryptionKey: copy(member.encryptionKey, 32),
  };
}
