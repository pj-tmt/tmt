import { requireValue } from '@tmt/colab-client';
import type { Bootstrap, PageInfo } from './bootstrap.js';
import type { Registration } from './registration.js';
import type { PageBinding, PageSnapshot } from './transport.js';
import type { Projection } from './fold-protocol.js';
import { Admission } from './admission.js';
import { Connection } from './connection.js';
import { Writer } from './writer.js';

/** Mounted update-only page binding. A fresh reconnect reconstructs from seq 1;
 * unsupported history is a blocking failure rather than a partial projection. */
export class Live implements PageBinding {
  #connection: Connection | null = null;
  #current: Promise<Connection>;
  #writer: Writer;
  #closed = false;
  #connecting = false;
  #attempts = 0;
  #projection: Projection = { source: '', title: '' };
  #listeners = new Set<{ publish(value: Projection): void; failed(error: Error): void }>();
  #error: Error | null = null;
  constructor(
    readonly mount: URL,
    readonly bootstrap: Bootstrap,
    readonly registration: Registration,
    readonly page: PageInfo,
    readonly signal?: AbortSignal,
  ) {
    this.#current = this.#open();
    this.#writer = new Writer(
      `writer:${bootstrap.space}:${page.pageId}:${page.epoch}:${registration.deviceId}`,
      () => this.#current,
    );
    signal?.addEventListener('abort', this.#abort, { once: true });
    if (signal?.aborted) this.close();
  }
  #abort = () => this.close();
  async #open(): Promise<Connection> {
    this.#connecting = true;
    try {
      const a = new Admission(
        this.bootstrap.space,
        this.page.pageId,
        this.page.epoch,
        this.bootstrap.owner,
        this.registration,
      );
      await a.restore();
      if (this.#closed) throw new Error('Page closed');
      const c = (this.#connection = new Connection(
        a,
        this.mount,
        this.page.sharing,
        (value) => {
          if (this.#closed) return;
          this.#projection = { source: value.source, title: value.title };
          this.#listeners.forEach((v) => v.publish(this.#projection));
        },
        (error) => this.#failed(error),
      ));
      await c.ready;
      this.#attempts = 0;
      return c;
    } finally {
      this.#connecting = false;
    }
  }
  #failed(error: Error) {
    if (this.#closed) return;
    if (
      !this.#connecting &&
      this.#attempts++ < 3 &&
      ['Sync disconnected', 'RESYNC_REQUIRED', 'Fresh membership catchup required'].includes(
        error.message,
      )
    ) {
      this.#current = this.#open();
      void this.#current.catch((next) => this.#block(next instanceof Error ? next : error));
    } else this.#block(error);
  }
  #block(error: Error) {
    this.#writer.close();
    this.#error = error;
    this.#listeners.forEach((v) => v.failed(error));
  }
  async snapshot(): Promise<PageSnapshot> {
    await this.#current;
    return {
      id: this.page.pageId,
      sharing: this.page.sharing,
      ...this.#projection,
      title: this.#projection.title || this.page.pageId,
      binding: this,
    };
  }
  subscribe(publish: (value: Projection) => void, failed: (error: Error) => void) {
    const listener = { publish, failed };
    this.#listeners.add(listener);
    if (this.#error) failed(this.#error);
    else publish(this.#projection);
    return () => {
      this.#listeners.delete(listener);
      // A same-turn subscriber replacement retains the binding (including React's
      // effect replay). Last-consumer release owns socket/Worker/writer cleanup.
      queueMicrotask(() => {
        if (this.#listeners.size === 0) this.close();
      });
    };
  }
  async edit(source: string, base: string) {
    if (this.#closed || this.#error) throw new Error('Page editing unavailable');
    const c = await this.#current;
    const prepared = await c.run(() => {
      requireValue(base === this.#projection.source);
      return c.fold.run({ type: 'prepare', source, base });
    });
    try {
      await this.#writer.submit(prepared.update);
    } finally {
      prepared.update.fill(0);
    }
  }
  close() {
    if (this.#closed) return;
    this.#closed = true;
    this.signal?.removeEventListener('abort', this.#abort);
    this.#writer.close();
    this.#connection?.close();
    this.#listeners.clear();
  }
}
