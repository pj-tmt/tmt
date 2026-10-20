import {
  binary,
  copy,
  decimal,
  decodeText,
  encodeBinary,
  equal,
  exactKeys,
  fields,
  frame,
  generatedId,
  namespace,
  requireValue,
  spaceId,
  text,
  type Bytes,
} from './bytes.js';
import {
  digest,
  hkdf,
  rootSecret,
  sign,
  signingKey,
  strictVerify,
  type RootSecret,
} from './crypto.js';
import { strictJson } from './json.js';
const SUITE = 'aes256gcm-hkdfsha256-ed25519-v1';
/** Non-update plaintext ceiling, matching the Rust model. */
export const MAX_PLAINTEXT = 16 * 1024 * 1024;
/** Serialized envelope ceiling: plaintext/tag/header and canonical base64 JSON overhead. */
export const MAX_ENVELOPE_JSON = Math.floor(((MAX_PLAINTEXT + 2048) * 4) / 3) + 2048;
const CONTEXT = [
  'space',
  'page',
  'epoch',
  'kind',
  'namespace',
  'authorDevice',
  'membershipRevision',
  'streamSeq',
  'prevHash',
];
export interface Context {
  space: string;
  page: string;
  epoch: string;
  kind: 'update' | 'checkpoint' | 'fold' | 'html' | 'asset';
  namespace: 'content' | 'own';
  authorDevice: string;
  membershipRevision: string;
  streamSeq: string;
  prevHash: Uint8Array;
}
/** The kind of an owner fold: a checkpoint an owner-member device signs for another owner-member
 * device's stream (colab-v1, "Owner fold"). Its header has one extra trailing field, the
 * covered device; every other kind keeps thirteen, so a reader that does not know the kind
 * refuses the header by its field count. */
export const FOLD = 'fold';
export function header(context: Context, objectId: string, coveredDevice?: string): Bytes {
  exactKeys(context, CONTEXT);
  requireValue((context.kind === FOLD) === (coveredDevice !== undefined));
  if (coveredDevice !== undefined) generatedId(coveredDevice);
  spaceId(context.space);
  generatedId(context.page);
  generatedId(context.authorDevice);
  decimal(context.epoch);
  decimal(context.membershipRevision);
  namespace(context.namespace);
  const seq = decimal(context.streamSeq, true),
    prev = copy(context.prevHash, 32);
  requireValue(typeof objectId === 'string' && /^[0-9a-f]{64}$/.test(objectId));
  requireValue(
    ['update', 'checkpoint', FOLD].includes(context.kind)
      ? seq > 0n
      : ['html', 'asset'].includes(context.kind) &&
          seq === 0n &&
          equal(prev, new Uint8Array(32)) &&
          (context.kind !== 'html' || context.namespace === 'content'),
  );
  requireValue(context.kind !== 'update' || seq !== 1n || equal(prev, new Uint8Array(32)));
  const out = frame(
    text('tmt-colab-object-v1'),
    text('1'),
    text(SUITE),
    text(context.space),
    text(context.page),
    text(context.epoch),
    text(context.kind),
    text(context.namespace),
    text(objectId),
    text(context.authorDevice),
    text(context.membershipRevision),
    text(context.streamSeq),
    prev,
    ...(coveredDevice === undefined ? [] : [text(coveredDevice)]),
  );
  requireValue(out.length <= 1024);
  return out;
}
export function decodeHeader(raw: Uint8Array): {
  context: Context;
  objectId: string;
  /** The device a `fold` covers; absent for every other kind. */
  coveredDevice?: string;
} {
  let f: Uint8Array[];
  try {
    f = fields(raw, 14);
  } catch {
    f = fields(raw, 13);
  }
  requireValue(
    decodeText(f[0]) === 'tmt-colab-object-v1' &&
      decodeText(f[1]) === '1' &&
      decodeText(f[2]) === SUITE,
  );
  const context: Context = {
    space: decodeText(f[3]),
    page: decodeText(f[4]),
    epoch: decodeText(f[5]),
    kind: decodeText(f[6]) as Context['kind'],
    namespace: decodeText(f[7]) as Context['namespace'],
    authorDevice: decodeText(f[9]),
    membershipRevision: decodeText(f[10]),
    streamSeq: decodeText(f[11]),
    prevHash: f[12],
  };
  const objectId = decodeText(f[8]);
  const coveredDevice = f.length === 14 ? decodeText(f[13]) : undefined;
  requireValue(equal(header(context, objectId, coveredDevice), raw));
  return coveredDevice === undefined ? { context, objectId } : { context, objectId, coveredDevice };
}
const ceiling = (h: Uint8Array) =>
  decodeHeader(h).context.kind === 'update' ? 256 * 1024 : MAX_PLAINTEXT;
export async function signatureInput(h: Uint8Array, ct: Uint8Array): Promise<Bytes> {
  const bytes = copy(h),
    cipher = copy(ct);
  return frame(text('tmt-colab-signature-v1'), bytes, new Uint8Array(12), await digest(cipher));
}
/** Immutable bytes; retry this same envelope. Syntax does not resolve log/session/stream policy. */
export class Envelope {
  #header: Bytes;
  #ciphertext: Bytes;
  #signature: Bytes;
  private constructor(h: Uint8Array, ct: Uint8Array, sig: Uint8Array) {
    this.#header = copy(h);
    this.#ciphertext = copy(ct);
    this.#signature = copy(sig, 64);
  }
  static fromJson(raw: Uint8Array): Envelope {
    const value = strictJson(raw, MAX_ENVELOPE_JSON);
    exactKeys(value, ['header', 'nonce', 'ciphertext', 'signature']);
    const h = binary(value.header, 1024);
    const max = ceiling(h);
    requireValue(equal(binary(value.nonce, 12, 12), new Uint8Array(12)));
    const ct = binary(value.ciphertext, max + 16);
    requireValue(ct.length >= 16);
    return new Envelope(h, ct, binary(value.signature, 64, 64));
  }
  toJson(): Bytes {
    return text(
      JSON.stringify({
        header: encodeBinary(this.#header),
        nonce: encodeBinary(new Uint8Array(12)),
        ciphertext: encodeBinary(this.#ciphertext),
        signature: encodeBinary(this.#signature),
      }),
    );
  }
  header(): Bytes {
    return copy(this.#header);
  }
  ciphertext(): Bytes {
    return copy(this.#ciphertext);
  }
  signature(): Bytes {
    return copy(this.#signature);
  }
  async hash(): Promise<Bytes> {
    return digest(
      frame(
        text('tmt-colab-envelope-hash-v1'),
        this.#header,
        new Uint8Array(12),
        this.#ciphertext,
        this.#signature,
      ),
    );
  }
  /** No caller ID or fixture seed API. A new seal always consumes WebCrypto entropy. */
  static async seal(
    context: Context,
    secret: RootSecret,
    signer: CryptoKey,
    plaintext: Uint8Array,
  ): Promise<Envelope> {
    signingKey(signer);
    exactKeys(context, CONTEXT);
    const id = Array.from(crypto.getRandomValues(new Uint8Array(32)), (b) =>
      b.toString(16).padStart(2, '0'),
    ).join('');
    const h = header(context, id);
    requireValue(plaintext.length <= ceiling(h));
    const root = rootSecret(secret),
      input = copy(plaintext);
    const key = await crypto.subtle.importKey(
      'raw',
      await hkdf(root, frame(text('tmt-colab-object-key-v1'), h)),
      'AES-GCM',
      false,
      ['encrypt'],
    );
    const ct = new Uint8Array(
      await crypto.subtle.encrypt(
        { name: 'AES-GCM', iv: new Uint8Array(12), additionalData: h, tagLength: 128 },
        key,
        input,
      ),
    );
    return new Envelope(h, ct, await sign(signer, await signatureInput(h, ct)));
  }
  /** Caller first admits current log, device chain, role, epoch and stream order. */
  async open(
    expected: Context,
    secret: RootSecret,
    devicePublic: Uint8Array,
    /** The covered device of a `fold`, which only opens with it; refused for any other kind. */
    coveredDevice?: string,
  ): Promise<Bytes> {
    const h = copy(this.#header),
      ct = copy(this.#ciphertext),
      sig = copy(this.#signature);
    requireValue(equal(h, header(expected, decodeHeader(h).objectId, coveredDevice)));
    const root = rootSecret(secret),
      publicKey = copy(devicePublic, 32);
    requireValue(await strictVerify(publicKey, sig, await signatureInput(h, ct)));
    const key = await crypto.subtle.importKey(
      'raw',
      await hkdf(root, frame(text('tmt-colab-object-key-v1'), h)),
      'AES-GCM',
      false,
      ['decrypt'],
    );
    return new Uint8Array(
      await crypto.subtle.decrypt(
        { name: 'AES-GCM', iv: new Uint8Array(12), additionalData: h, tagLength: 128 },
        key,
        ct,
      ),
    );
  }
}
/** The anchor of a fold: the covered stream's `update` envelope at the fold's sequence, whose hash
 * is the fold's previous hash and which the covered device itself signed, so the fold names a head
 * that device really wrote and not one its signer asserts. It admits neither device. */
export async function checkAnchor(
  fold: Envelope,
  anchor: Envelope,
  coveredKey: Uint8Array,
): Promise<void> {
  const f = decodeHeader(fold.header()),
    a = decodeHeader(anchor.header());
  requireValue(
    f.context.kind === FOLD &&
      f.coveredDevice !== undefined &&
      a.context.kind === 'update' &&
      a.coveredDevice === undefined &&
      a.context.authorDevice === f.coveredDevice &&
      a.context.space === f.context.space &&
      a.context.page === f.context.page &&
      a.context.epoch === f.context.epoch &&
      a.context.streamSeq === f.context.streamSeq &&
      equal(await anchor.hash(), f.context.prevHash),
  );
  requireValue(
    await strictVerify(
      copy(coveredKey, 32),
      anchor.signature(),
      await signatureInput(anchor.header(), anchor.ciphertext()),
    ),
  );
}
