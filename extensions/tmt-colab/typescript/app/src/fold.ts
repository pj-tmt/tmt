import { exactKeys } from '@tmt/colab-client';
import {
  UPDATE_BYTES,
  validateProjection,
  type FoldCommand,
  type FoldResult,
} from './fold-protocol.js';

/** Decoder output is checked again by the authority-holding parent. Failure
 * terminates the Worker and flags the binding; callers must resync from scratch. */
export class Fold {
  #worker: Worker;
  #pending: { id: number; resolve(value: FoldResult): void; reject(error: Error): void } | null =
    null;
  #next = 0;
  #closed = false;
  #timer: ReturnType<typeof setTimeout> | undefined;
  constructor(
    worker = new Worker(new URL('./fold.worker.ts', import.meta.url), { type: 'module' }),
  ) {
    this.#worker = worker;
    worker.onerror = () => this.close();
    worker.onmessage = (event: MessageEvent<unknown>) => {
      try {
        exactKeys(event.data, ['id', 'source', 'title', 'update']);
        const value = event.data as unknown as FoldResult & { id: number; error?: string };
        if (!this.#pending || value.id !== this.#pending.id || value.error)
          throw new Error('Rejected decoder output');
        validateProjection(value);
        if (!(value.update instanceof Uint8Array) || value.update.length > UPDATE_BYTES)
          throw new Error('Invalid decoder output');
        const pending = this.#pending;
        this.#pending = null;
        clearTimeout(this.#timer);
        pending.resolve({ source: value.source, title: value.title, update: value.update });
      } catch {
        this.close();
      }
    };
  }
  run(command: FoldCommand): Promise<FoldResult> {
    if (this.#closed || this.#pending) return Promise.reject(new Error('Decoder unavailable'));
    if (
      command.type === 'apply' &&
      (command.updates.length > 200 ||
        command.updates.reduce((n, v) => n + v.length, 0) > UPDATE_BYTES)
    )
      return Promise.reject(new Error('Decoder input capacity'));
    return new Promise((resolve, reject) => {
      const id = ++this.#next;
      this.#pending = { id, resolve, reject };
      this.#timer = setTimeout(() => this.close(), 2000);
      try {
        this.#worker.postMessage({ id, command });
      } catch {
        this.close();
      }
    });
  }
  close() {
    this.#closed = true;
    clearTimeout(this.#timer);
    this.#worker.terminate();
    this.#pending?.reject(new Error('Decoder failed or exceeded its time budget'));
    this.#pending = null;
  }
}
