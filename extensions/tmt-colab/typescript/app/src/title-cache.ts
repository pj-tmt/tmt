import { binary, decodeText, encodeBinary, exactKeys, requireValue, text } from '@tmt/colab-client';
import { UPDATE_BYTES } from './fold-protocol.js';
import { titleKey } from './keyring.js';
import { record } from './storage.js';

interface StoredTitle {
  nonce: string;
  ciphertext: string;
}

/** Optional display hints only. Plaintext exists in parent memory, never IndexedDB. */
export class TitleCache {
  constructor(
    readonly space: string,
    readonly device: string,
  ) {}
  #scope(page: string) {
    return ['tmt-colab-title-cache-v1', this.space, this.device, page];
  }
  #record(page: string) {
    return `title:${this.space}:${this.device}:${page}`;
  }
  async read(page: string): Promise<string | undefined> {
    try {
      const stored = await record<StoredTitle>(this.#record(page));
      if (stored === undefined) return undefined;
      exactKeys(stored, ['nonce', 'ciphertext']);
      const plaintext = await crypto.subtle.decrypt(
        {
          name: 'AES-GCM',
          iv: binary(stored.nonce, 12, 12),
          additionalData: text(JSON.stringify(this.#scope(page))),
        },
        await titleKey(this.device),
        binary(stored.ciphertext, UPDATE_BYTES + 16),
      );
      requireValue(plaintext.byteLength <= UPDATE_BYTES);
      return decodeText(new Uint8Array(plaintext));
    } catch {
      // Corrupt, unavailable or wrong-scope hints must never block page access.
      return undefined;
    }
  }
  /** Called only by the mounted owner after Live accepts a verified fold. */
  async remember(page: string, title: string, signal?: AbortSignal): Promise<void> {
    try {
      if (signal?.aborted) return;
      const plaintext = text(title);
      requireValue(plaintext.length <= UPDATE_BYTES);
      const nonce = crypto.getRandomValues(new Uint8Array(12));
      const ciphertext = await crypto.subtle.encrypt(
        { name: 'AES-GCM', iv: nonce, additionalData: text(JSON.stringify(this.#scope(page))) },
        await titleKey(this.device),
        plaintext,
      );
      if (signal?.aborted) return;
      await record(this.#record(page), {
        nonce: encodeBinary(nonce),
        ciphertext: encodeBinary(new Uint8Array(ciphertext)),
      });
    } catch {
      // Persistence is best-effort presentation, never a page/management mutation.
    }
  }
}
