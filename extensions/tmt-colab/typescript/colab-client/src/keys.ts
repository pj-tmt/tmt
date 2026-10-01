import { copy, equal, requireValue, type Bytes } from './bytes.js';
/** Recipient private material stays in a non-extractable native handle. No seed import API. */
export class RecipientKey {
  #key: CryptoKey;
  #public: Bytes;
  private constructor(key: CryptoKey, publicKey: Uint8Array) {
    this.#key = key;
    this.#public = copy(publicKey, 32);
  }
  static async generate(): Promise<RecipientKey> {
    const pair = (await crypto.subtle.generateKey('X25519', false, [
      'deriveBits',
    ])) as CryptoKeyPair;
    return new RecipientKey(
      pair.privateKey,
      new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey)),
    );
  }
  /** Restores opaque browser-keyring handles; the caller binds the public key through the log. */
  static async fromHandle(privateKey: CryptoKey, publicKey: Uint8Array): Promise<RecipientKey> {
    const raw = copy(publicKey, 32);
    requireValue(
      privateKey.type === 'private' &&
        !privateKey.extractable &&
        privateKey.algorithm.name === 'X25519' &&
        privateKey.usages.includes('deriveBits'),
    );
    const base = new Uint8Array(32);
    base[0] = 9;
    const peer = await crypto.subtle.importKey('raw', base, 'X25519', false, []);
    const derived = new Uint8Array(
      await crypto.subtle.deriveBits({ name: 'X25519', public: peer }, privateKey, 256),
    );
    requireValue(equal(raw, derived));
    return new RecipientKey(privateKey, raw);
  }
  publicKey(): Bytes {
    return copy(this.#public);
  }
  /** Opaque structured-clone persistence, never raw/private export. */
  handle(): CryptoKey {
    return this.#key;
  }
}
