import { decodeText, requireValue, text } from '@tmt/colab-client';
import { UPDATE_BYTES } from './fold-protocol.js';
import { titleKey } from './keyring.js';
import { open, seal, type Sealed } from './local-seal.js';
import { record } from './storage.js';

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
      const stored = await record<Sealed>(this.#record(page));
      if (stored === undefined) return undefined;
      return decodeText(
        await open(await titleKey(this.device), this.#scope(page), stored, UPDATE_BYTES),
      );
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
      const sealed = await seal(await titleKey(this.device), this.#scope(page), plaintext);
      if (signal?.aborted) return;
      await record(this.#record(page), sealed);
    } catch {
      // Persistence is best-effort presentation, never a page/management mutation.
    }
  }
}
