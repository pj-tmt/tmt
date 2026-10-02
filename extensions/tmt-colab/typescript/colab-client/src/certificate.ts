/** Fixed owner->member/link->device chain. Live issuer/role/history admission is caller-owned. */
import {
  binary,
  copy,
  decimal,
  decodeText,
  equal,
  exactKeys,
  fields,
  frame,
  generatedId,
  requireValue,
  spaceId,
  text,
  time,
  type Bytes,
} from './bytes.js';
import { digest, strictVerify, validEdPoint } from './crypto.js';
import { strictJson } from './json.js';
export interface Certificate {
  space: string;
  issuerKind: string;
  issuerId: string;
  deviceId: string;
  signingKey: Uint8Array;
  encryptionKey: Uint8Array;
  membershipRevision: string;
  issuedAt: number;
  expiresAt: number;
}
export function input(v: Certificate): Bytes {
  spaceId(v.space);
  generatedId(v.issuerId);
  generatedId(v.deviceId);
  requireValue(['member', 'link'].includes(v.issuerKind) && validEdPoint(v.signingKey));
  decimal(v.membershipRevision);
  time(v.issuedAt);
  time(v.expiresAt);
  requireValue(v.expiresAt > v.issuedAt);
  return frame(
    text('tmt-colab-device-cert-v1'),
    text('1'),
    text(v.space),
    text(v.issuerKind),
    text(v.issuerId),
    text(v.deviceId),
    copy(v.signingKey, 32),
    copy(v.encryptionKey, 32),
    text(v.membershipRevision),
    text(String(v.issuedAt)),
    text(String(v.expiresAt)),
  );
}
export function decode(raw: Uint8Array): Certificate {
  const f = fields(raw, 11);
  requireValue(decodeText(f[0]) === 'tmt-colab-device-cert-v1' && decodeText(f[1]) === '1');
  const v = {
    space: decodeText(f[2]),
    issuerKind: decodeText(f[3]),
    issuerId: decodeText(f[4]),
    deviceId: decodeText(f[5]),
    signingKey: copy(f[6], 32),
    encryptionKey: copy(f[7], 32),
    membershipRevision: decodeText(f[8]),
    issuedAt: Number(decimal(decodeText(f[9]), true)),
    expiresAt: Number(decimal(decodeText(f[10]), true)),
  };
  requireValue(equal(input(v), raw));
  return v;
}
export class Chain {
  #statement: Bytes;
  #certificate: Bytes;
  #signature: Bytes;
  private constructor(statement: Bytes, certificate: Bytes, signature: Bytes) {
    this.#statement = statement;
    this.#certificate = certificate;
    this.#signature = signature;
  }
  static fromJson(raw: Uint8Array): Chain {
    const v = strictJson(raw, 16 * 1024, true);
    exactKeys(v, ['version', 'issuerStatement', 'deviceCertificate', 'issuerSignature']);
    requireValue(v.version === 1);
    const cert = binary(v.deviceCertificate, 1024);
    decode(cert);
    return new Chain(binary(v.issuerStatement, 32, 32), cert, binary(v.issuerSignature, 64, 64));
  }
  certificate(): Certificate {
    return decode(this.#certificate);
  }
  digest(): Promise<Bytes> {
    return digest(
      frame(
        text('tmt-colab-chain-v1'),
        text('1'),
        this.#statement,
        this.#certificate,
        this.#signature,
      ),
    );
  }
  /** Resolve a currently live issuer from this exact verified statement hash before calling. */
  async verify(
    statementHash: Uint8Array,
    expected: Certificate,
    issuerKey: Uint8Array,
  ): Promise<void> {
    const statement = copy(statementHash, 32),
      cert = input(expected),
      key = copy(issuerKey, 32);
    requireValue(equal(statement, this.#statement) && equal(cert, this.#certificate));
    requireValue(await strictVerify(key, this.#signature, this.#certificate));
  }
}
