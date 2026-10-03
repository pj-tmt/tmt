import { copy, equal, frame, requireValue, text, type Bytes } from './bytes.js';
const unhex = (hex: string): Bytes =>
  Uint8Array.from(hex.match(/../g) ?? [], (n) => Number.parseInt(n, 16));
// Pinned curve25519-dalek 5.0.0 EIGHT_TORSION; masking also rejects sign-bit variants.
const torsion = [
  '0100000000000000000000000000000000000000000000000000000000000000',
  'c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a',
  '0000000000000000000000000000000000000000000000000000000000000080',
  '26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05',
  'ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f',
  '26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc85',
  '0000000000000000000000000000000000000000000000000000000000000000',
  'c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac03fa',
].map(unhex);
function lessLE(a: Uint8Array, b: Uint8Array): boolean {
  for (let i = a.length - 1; i >= 0; i--) if (a[i] !== b[i]) return a[i] < b[i];
  return false;
}
export function validEdPoint(raw: Uint8Array): boolean {
  if (raw.length !== 32) return false;
  const y = copy(raw);
  y[31] &= 127;
  return (
    lessLE(y, unhex('ed' + 'ff'.repeat(30) + '7f')) &&
    !torsion.some((p) => {
      const masked = copy(p);
      masked[31] &= 127;
      return equal(y, masked);
    })
  );
}
export async function strictVerify(
  publicKey: Uint8Array,
  signature: Uint8Array,
  message: Uint8Array,
): Promise<boolean> {
  // Snapshot all mutable inputs before the first await, including bytes behind aliases.
  const keyBytes = copy(publicKey),
    sig = copy(signature),
    input = copy(message);
  if (
    !validEdPoint(keyBytes) ||
    sig.length !== 64 ||
    !validEdPoint(sig.slice(0, 32)) ||
    !lessLE(
      sig.slice(32),
      unhex('edd3f55c1a631258d69cf7a2def9de1400000000000000000000000000000010'),
    )
  )
    return false;
  try {
    const key = await crypto.subtle.importKey('raw', keyBytes, 'Ed25519', false, ['verify']);
    return await crypto.subtle.verify('Ed25519', key, sig, input);
  } catch {
    return false;
  }
}
export async function digest(value: Uint8Array): Promise<Bytes> {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', copy(value)));
}
export async function hmac(key: Uint8Array, input: Uint8Array): Promise<Bytes> {
  const k = copy(key),
    bytes = copy(input);
  const native = await crypto.subtle.importKey('raw', k, { name: 'HMAC', hash: 'SHA-256' }, false, [
    'sign',
  ]);
  return new Uint8Array(await crypto.subtle.sign('HMAC', native, bytes));
}
/**
 * A 32-byte root or a non-extractable HKDF/deriveBits handle imported from one.
 * WebCrypto hides handle input length; the importing caller must validate 32 bytes.
 * Handles remain opaque and are never exported by these primitives.
 */
export type RootSecret = Uint8Array | CryptoKey;
export function rootSecret(value: RootSecret): RootSecret {
  if (value instanceof Uint8Array) return copy(value, 32);
  requireValue(
    value.type === 'secret' &&
      !value.extractable &&
      value.algorithm.name === 'HKDF' &&
      value.usages.includes('deriveBits'),
  );
  return value;
}
export async function hkdf(key: RootSecret, info: Uint8Array): Promise<Bytes> {
  const k = rootSecret(key),
    bytes = copy(info);
  const native =
    k instanceof Uint8Array
      ? await crypto.subtle.importKey('raw', copy(k), 'HKDF', false, ['deriveBits'])
      : k;
  return new Uint8Array(
    await crypto.subtle.deriveBits(
      { name: 'HKDF', hash: 'SHA-256', salt: new Uint8Array(), info: bytes },
      native,
      256,
    ),
  );
}
export function signingKey(key: CryptoKey): void {
  requireValue(
    key.type === 'private' &&
      !key.extractable &&
      key.algorithm.name === 'Ed25519' &&
      key.usages.includes('sign'),
  );
}
export async function sign(key: CryptoKey, input: Uint8Array): Promise<Bytes> {
  signingKey(key);
  return new Uint8Array(await crypto.subtle.sign('Ed25519', key, copy(input)));
}
export async function deriveSpaceId(owner: Uint8Array): Promise<string> {
  const bytes = copy(owner, 32);
  requireValue(validEdPoint(bytes));
  const hash = await digest(frame(text('tmt-colab-space-id-v1'), bytes));
  const alphabet = 'abcdefghijklmnopqrstuvwxyz234567';
  let out = '';
  for (let i = 0; i < 32; i++) {
    const bit = i * 5,
      n = (hash[bit >> 3] << 8) | hash[(bit >> 3) + 1];
    out += alphabet[(n >> (11 - (bit % 8))) & 31];
  }
  return out;
}
export async function probeCapabilities(): Promise<void> {
  requireValue(globalThis.isSecureContext === true);
  const ed = (await crypto.subtle.generateKey('Ed25519', false, [
    'sign',
    'verify',
  ])) as CryptoKeyPair;
  const x = (await crypto.subtle.generateKey('X25519', false, ['deriveBits'])) as CryptoKeyPair;
  const p = new Uint8Array(await crypto.subtle.exportKey('raw', ed.publicKey));
  requireValue(await strictVerify(p, await sign(ed.privateKey, text('probe')), text('probe')));
  await crypto.subtle.deriveBits({ name: 'X25519', public: x.publicKey }, x.privateKey, 256);
}
