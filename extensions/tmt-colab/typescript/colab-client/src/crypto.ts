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
const fieldPrime = (1n << 255n) - 19n;
// RFC 8032 section 5.1: d = -121665/121666 modulo the fixed field prime.
const edwardsD = 37095705934669439343138083508754565189542113879843219016388785533085940283555n;
export function validEdPoint(raw: Uint8Array): boolean {
  if (raw.length !== 32) return false;
  const y = copy(raw);
  y[31] &= 127;
  if (
    !lessLE(y, unhex('ed' + 'ff'.repeat(30) + '7f')) ||
    torsion.some((p) => {
      const masked = copy(p);
      masked[31] &= 127;
      return equal(y, masked);
    })
  )
    return false;
  // Public-point decompression only; never pass secret material through this guard.
  // Canonical y and the torsion check above also exclude x=0, including negative zero.
  let coordinate = 0n;
  for (let i = 31; i >= 0; i--) coordinate = (coordinate << 8n) | BigInt(y[i]);
  const square = (coordinate * coordinate) % fieldPrime,
    u = (square + fieldPrime - 1n) % fieldPrime,
    v = (edwardsD * square + 1n) % fieldPrime;
  if (v === 0n) return false;
  // RFC 8032 section 5.1.3: x = u*v^3*(u*v^7)^((p-5)/8).
  const v2 = (v * v) % fieldPrime,
    v3 = (v2 * v) % fieldPrime,
    v7 = (v3 * v3 * v) % fieldPrime,
    base = (u * v7) % fieldPrime;
  let powered = 1n;
  // (p-5)/8 = 2^252-3. The fixed chain depends only on this public constant.
  for (let bit = 251; bit >= 0; bit--) {
    powered = (powered * powered) % fieldPrime;
    if (bit !== 1) powered = (powered * base) % fieldPrime;
  }
  const x = (((u * v3) % fieldPrime) * powered) % fieldPrime,
    check = (v * x * x) % fieldPrime;
  // Either candidate yields a square root (the second after multiplying by sqrt(-1)).
  // For nonzero x either sign bit selects a canonical root; no subgroup restriction.
  return check === u || check === (fieldPrime - u) % fieldPrime;
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
