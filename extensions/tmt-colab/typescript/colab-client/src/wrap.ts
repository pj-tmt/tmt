/** Owner-authenticated, single-shot RFC9180 Base (0020/0001/0002), open only. */
import {
  binary,
  concat,
  copy,
  decimal,
  decodeText,
  encodeBinary,
  equal,
  exactKeys,
  fields,
  frame,
  generatedId,
  requireValue,
  spaceId,
  text,
  type Bytes,
} from './bytes.js';
import { deriveSpaceId, digest, hmac, strictVerify, validEdPoint } from './crypto.js';
import { strictJson } from './json.js';
import { RecipientKey } from './keys.js';
export interface Header {
  space: string;
  page: string;
  epoch: string;
  recipientKind: string;
  recipientId: string;
  recipientKey: Uint8Array;
  signerKey: Uint8Array;
  membershipRevision: string;
}
export function input(v: Header): Bytes {
  spaceId(v.space);
  generatedId(v.page);
  generatedId(v.recipientId);
  decimal(v.epoch);
  decimal(v.membershipRevision);
  requireValue(
    ['member', 'device', 'link', 'bridge'].includes(v.recipientKind) && validEdPoint(v.signerKey),
  );
  const out = frame(
    text('tmt-colab-wrap-v1'),
    text('1'),
    text('base-x25519-hkdfsha256-aes256gcm'),
    text(v.space),
    text(v.page),
    text(v.epoch),
    text(v.recipientKind),
    text(v.recipientId),
    copy(v.recipientKey, 32),
    copy(v.signerKey, 32),
    text(v.membershipRevision),
    text('epoch-key'),
  );
  requireValue(out.length <= 1024);
  return out;
}
export function decode(raw: Uint8Array): Header {
  const f = fields(raw, 12);
  requireValue(
    decodeText(f[0]) === 'tmt-colab-wrap-v1' &&
      decodeText(f[1]) === '1' &&
      decodeText(f[2]) === 'base-x25519-hkdfsha256-aes256gcm' &&
      decodeText(f[11]) === 'epoch-key',
  );
  const v = {
    space: decodeText(f[3]),
    page: decodeText(f[4]),
    epoch: decodeText(f[5]),
    recipientKind: decodeText(f[6]),
    recipientId: decodeText(f[7]),
    recipientKey: copy(f[8], 32),
    signerKey: copy(f[9], 32),
    membershipRevision: decodeText(f[10]),
  };
  requireValue(equal(input(v), raw));
  return v;
}
export class Envelope {
  #header: Bytes;
  #enc: Bytes;
  #ct: Bytes;
  #sig: Bytes;
  private constructor(h: Bytes, enc: Bytes, ct: Bytes, sig: Bytes) {
    this.#header = h;
    this.#enc = enc;
    this.#ct = ct;
    this.#sig = sig;
  }
  static fromJson(raw: Uint8Array): Envelope {
    const v = strictJson(raw, 2048);
    exactKeys(v, ['header', 'enc', 'ciphertext', 'signature']);
    const h = binary(v.header, 1024);
    decode(h);
    return new Envelope(
      h,
      binary(v.enc, 32, 32),
      binary(v.ciphertext, 48, 48),
      binary(v.signature, 64, 64),
    );
  }
  header(): Header {
    return decode(this.#header);
  }
  toJson(): Bytes {
    return text(
      JSON.stringify({
        header: encodeBinary(this.#header),
        enc: encodeBinary(this.#enc),
        ciphertext: encodeBinary(this.#ct),
        signature: encodeBinary(this.#sig),
      }),
    );
  }
  async verifyOwner(owner: Uint8Array): Promise<void> {
    const key = copy(owner, 32),
      h = this.header();
    requireValue(equal(h.signerKey, key) && (await deriveSpaceId(key)) === h.space);
    requireValue(
      await strictVerify(
        key,
        this.#sig,
        frame(
          text('tmt-colab-wrap-signature-v1'),
          text('1'),
          this.#header,
          this.#enc,
          await digest(this.#ct),
        ),
      ),
    );
  }
  /** Expected context/recipient/owner must come from the latest verified log, not this header. */
  async open(expected: Header, recipient: RecipientKey, owner: Uint8Array): Promise<Bytes> {
    const bytes = input(expected),
      key = copy(owner, 32),
      publicKey = recipient.publicKey(),
      handle = recipient.handle();
    requireValue(
      equal(bytes, this.#header) &&
        equal(expected.signerKey, key) &&
        equal(expected.recipientKey, publicKey),
    );
    await this.verifyOwner(key);
    // RFC9180 4.1, 5.1: enc||pkR KEM context, mode_base=0, no PSK; sequence zero.
    // https://www.rfc-editor.org/rfc/rfc9180.html#section-5.1
    const peer = await crypto.subtle.importKey('raw', this.#enc, 'X25519', false, []);
    const dh = new Uint8Array(
      await crypto.subtle.deriveBits({ name: 'X25519', public: peer }, handle, 256),
    );
    requireValue(dh.some((n) => n !== 0));
    const kem = concat(text('KEM'), new Uint8Array([0, 32])),
      suite = concat(text('HPKE'), new Uint8Array([0, 32, 0, 1, 0, 2]));
    let shared: Bytes | undefined, secret: Bytes | undefined, aes: Bytes | undefined;
    try {
      const prk = await extract(kem, 'eae_prk', dh);
      try {
        shared = await expand(kem, prk, 'shared_secret', concat(this.#enc, publicKey), 32);
      } finally {
        prk.fill(0);
      }
      const info = frame(text('tmt-colab-hpke-info-v1'), this.#header);
      const context = concat(
        new Uint8Array([0]),
        await extract(suite, 'psk_id_hash', new Uint8Array()),
        await extract(suite, 'info_hash', info),
      );
      secret = await extract(suite, 'secret', new Uint8Array(), shared);
      aes = await expand(suite, secret, 'key', context, 32);
      const nonce = await expand(suite, secret, 'base_nonce', context, 12);
      const native = await crypto.subtle.importKey('raw', aes, 'AES-GCM', false, ['decrypt']);
      return copy(
        new Uint8Array(
          await crypto.subtle.decrypt(
            { name: 'AES-GCM', iv: nonce, additionalData: this.#header, tagLength: 128 },
            native,
            this.#ct,
          ),
        ),
        32,
      );
    } finally {
      dh.fill(0);
      shared?.fill(0);
      secret?.fill(0);
      aes?.fill(0);
    }
  }
}
// Only the fixed RFC9180 schedule uses these private helpers. No intermediate-secret API.
function extract(
  suite: Bytes,
  label: string,
  value: Bytes,
  salt = new Uint8Array(32),
): Promise<Bytes> {
  return hmac(salt, concat(text('HPKE-v1'), suite, text(label), value));
}
async function expand(
  suite: Bytes,
  key: Bytes,
  label: string,
  info: Bytes,
  size: number,
): Promise<Bytes> {
  requireValue(size > 0 && size <= 32);
  return (
    await hmac(
      key,
      concat(
        new Uint8Array([0, size]),
        text('HPKE-v1'),
        suite,
        text(label),
        info,
        new Uint8Array([1]),
      ),
    )
  ).slice(0, size);
}
