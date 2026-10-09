import { binary, encodeBinary, exactKeys, requireValue, text } from '@tmt/colab-client';

/** One AES-GCM record as stored in IndexedDB; the scope is authenticated, never stored. */
export interface Sealed {
  nonce: string;
  ciphertext: string;
}

/** Binds the ciphertext to its owner scope so a copied record cannot be read elsewhere. */
export async function seal(
  key: CryptoKey,
  scope: readonly string[],
  plaintext: Uint8Array<ArrayBuffer>,
): Promise<Sealed> {
  const nonce = crypto.getRandomValues(new Uint8Array(12));
  const ciphertext = await crypto.subtle.encrypt(
    { name: 'AES-GCM', iv: nonce, additionalData: text(JSON.stringify(scope)) },
    key,
    plaintext,
  );
  return { nonce: encodeBinary(nonce), ciphertext: encodeBinary(new Uint8Array(ciphertext)) };
}

/** Throws for a malformed record, a wrong scope, tampering or an oversized plaintext. */
export async function open(
  key: CryptoKey,
  scope: readonly string[],
  stored: unknown,
  maxBytes: number,
): Promise<Uint8Array> {
  exactKeys(stored, ['nonce', 'ciphertext']);
  const { nonce, ciphertext } = stored as unknown as Sealed;
  const plaintext = await crypto.subtle.decrypt(
    { name: 'AES-GCM', iv: binary(nonce, 12, 12), additionalData: text(JSON.stringify(scope)) },
    key,
    binary(ciphertext, maxBytes + 16),
  );
  requireValue(plaintext.byteLength <= maxBytes);
  return new Uint8Array(plaintext);
}
