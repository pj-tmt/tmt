import { ThreadStore, commentForAsk } from './thread-store.js';
import { readThreads } from './thread-records.js';
import { LiveAsk, pageAsks } from './live-ask.js';
import { requireValue } from '@tmt/colab-client';
import type { Bootstrap, PageInfo } from './bootstrap.js';
import type { Registration } from './registration.js';
import type { PageBinding, PageSnapshot, PageView } from './transport.js';
import { SessionEndedError, type RemoteClient } from './ask-remote.js';
import { Admission } from './admission.js';
import { Connection } from './connection.js';
import { Writer } from './writer.js';
import { prepareExport, hex, type ExportBundle } from './export.js';
import { text } from './strings.js';

export interface LiveSession {
  registration: Registration;
  remote: RemoteClient | null;
}
export interface LiveSessionOwner {
  rememberTitle?(page: string, title: string, registration: Registration): Promise<void>;
  recover?(): Promise<boolean>;
  reconnect(previous: Registration): Promise<LiveSession>;
}

/** Mounted update-only page binding. A fresh reconnect reconstructs from seq 1;
 * unsupported history is a blocking failure rather than a partial projection. */
export class Live implements PageBinding {
  ask?: LiveAsk;
  readonly discussion: ThreadStore;
  #remote: RemoteClient | null = null;
  #observation: AbortController | null = null;
  #views = Promise.resolve();
  #processingViews = false;
  #pendingView: { value: PageView; admission: Admission } | null = null;
  #admitted: PageView = { source: '', title: '' };
  #connection: Connection | null = null;
  #current: Promise<Connection>;
  #writer: Writer;
  #closed = false;
  #connecting = false;
  #attempts = 0;
  #projection: PageView = { source: '', title: '' };
  #listeners = new Set<{ publish(value: PageView): void; failed(error: Error): void }>();
  #error: Error | null = null;
  constructor(
    readonly mount: URL,
    readonly bootstrap: Bootstrap,
    public registration: Registration,
    readonly page: PageInfo,
    readonly signal?: AbortSignal,
    remote: RemoteClient | null = null,
    private sessionOwner?: LiveSessionOwner,
  ) {
    this.#current = this.#open();
    this.#writer = new Writer(
      `writer:${bootstrap.space}:${page.pageId}:${page.epoch}:${registration.deviceId}`,
      () => this.#current,
    );
    this.discussion = new ThreadStore({
      spaceId: bootstrap.space,
      pageId: page.pageId,
      epoch: page.epoch,
      sharing: page.sharing,
      deviceId: () => this.registration.deviceId,
      deviceName: () => this.registration.deviceName ?? '',
      own: () => this.#admitted.own ?? {},
      connection: () => this.#current,
      publish: (records) => this.#writer.submitOwnRecords(records),
      available: () => !this.#closed && !this.#error && !this.#connecting,
    });
    this.#replaceAsk(remote);
    if (typeof document !== 'undefined')
      document.addEventListener('visibilitychange', this.#visibility);
    signal?.addEventListener('abort', this.#abort, { once: true });
    if (signal?.aborted) this.close();
  }
  #replaceAsk(remote: RemoteClient | null) {
    this.#remote = remote;
    this.ask?.close();
    const { bootstrap, page, registration } = this;
    this.ask = remote
      ? new LiveAsk({
          remote,
          space: bootstrap.space,
          page: page.pageId,
          sharing: page.sharing,
          deviceId: registration.deviceId,
          key: registration.keys.sign,
          publicKey: registration.keys.signPublic,
          own: () => this.#admitted.own ?? {},
          commentContext: (context) =>
            commentForAsk(
              readThreads(
                this.#admitted.own ?? {},
                {
                  spaceId: bootstrap.space,
                  pageId: page.pageId,
                  epoch: page.epoch,
                },
                (writer) => this.#connection?.objects.ownSigningKey(writer),
              ),
              context,
            ),
          publish: (root, key, value) => this.#writer.submitOwn(root, key, value),
          connection: () => this.#current,
          observe: () => this.#observe(),
          sessionEnded: () => {
            if (!this.#connecting && !this.#closed && !this.#error)
              this.#failed(new Error('Remote session ended'));
          },
        })
      : undefined;
  }
  #abort = () => this.close();
  #visibility = () => {
    if (document.visibilityState === 'hidden') {
      this.#observation?.abort();
    } else this.#observe();
  };
  #observe() {
    if (
      this.#closed ||
      this.#connecting ||
      !this.ask ||
      this.#error ||
      this.#observation ||
      this.#listeners.size === 0 ||
      (typeof document !== 'undefined' && document.visibilityState === 'hidden')
    )
      return;
    const controller = new AbortController();
    this.#observation = controller;
    this.#projection = { ...this.#projection, askUnavailable: false };
    void this.ask
      .observe(controller.signal)
      .catch(() => {
        if (!controller.signal.aborted && !this.#closed) {
          this.#projection = { ...this.#projection, askUnavailable: true };
          this.#listeners.forEach((v) => v.publish(structuredClone(this.#projection)));
        }
      })
      .finally(() => {
        if (this.#observation === controller) this.#observation = null;
        if (
          controller.signal.aborted &&
          !this.#closed &&
          (typeof document === 'undefined' || document.visibilityState !== 'hidden')
        )
          this.#observe();
      });
  }
  async #open(replaceSession = false): Promise<Connection> {
    this.#connecting = true;
    try {
      if (replaceSession && this.sessionOwner) {
        const session = await this.sessionOwner.reconnect(this.registration);
        if (this.#closed) throw new Error('Page closed');
        this.registration = session.registration;
        this.#replaceAsk(session.remote);
      }
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
          this.#admitted = structuredClone(value);
          this.#pendingView = { value: this.#admitted, admission: a };
          if (!this.#processingViews) {
            this.#processingViews = true;
            this.#views = Promise.resolve().then(() => this.#publishViews());
          }
        },
        (error) => {
          if (this.#connection === c) this.#failed(error);
        },
      ));
      await c.ready;
      this.#attempts = 0;
      return c;
    } finally {
      this.#connecting = false;
      this.#observe();
    }
  }
  async #publishViews(): Promise<void> {
    try {
      // At most one active and one latest pending snapshot: slow crypto never
      // builds an unbounded queue of detached own documents.
      while (this.#pendingView && !this.#closed) {
        const { value, admission } = this.#pendingView;
        this.#pendingView = null;
        const connection = this.#connection;
        if (!connection || connection.admission !== admission) continue;
        const asks = await pageAsks(value.own ?? {}, admission, (writer) =>
          connection.objects.ownSigningKey(writer),
        );
        if (this.#closed || this.#connection?.admission !== admission || this.#pendingView)
          continue;
        await this.sessionOwner?.rememberTitle?.(
          this.page.pageId,
          value.title,
          admission.registration,
        );
        if (this.#closed || this.#connection?.admission !== admission || this.#pendingView)
          continue;
        const threads = readThreads(
          value.own ?? {},
          {
            spaceId: admission.space,
            pageId: admission.page,
            epoch: admission.epoch,
          },
          (writer) => connection.objects.ownSigningKey(writer),
        );
        this.#projection = { ...value, asks, threads };

        this.#listeners.forEach((v) => v.publish(structuredClone(this.#projection)));
        this.#observe();
      }
    } catch (error) {
      if (!this.#closed)
        this.#block(error instanceof Error ? error : new Error('Ask data unavailable'));
    } finally {
      this.#processingViews = false;
    }
  }
  #failed(error: Error) {
    if (this.#closed) return;
    const sessionEnded =
      error instanceof SessionEndedError || error.message === 'Remote session ended';
    if (
      !this.#connecting &&
      this.#attempts++ < 3 &&
      (sessionEnded ||
        ['Sync disconnected', 'RESYNC_REQUIRED', 'Fresh membership catchup required'].includes(
          error.message,
        ))
    ) {
      this.#observation?.abort();
      this.#observation = null;
      this.ask?.close();
      this.ask = undefined;
      this.#listeners.forEach((v) => v.publish(structuredClone(this.#projection)));
      const previous = this.#connection;
      this.#connection = null;
      previous?.close();
      // A tunnel resync retains the verified Session and Remote port. Only a
      // session fault may replace them and end other mounted tunnels.
      if (!sessionEnded) this.#replaceAsk(this.#remote);
      this.#current = this.#open(sessionEnded);
      void this.#current.catch((next) => this.#block(next instanceof Error ? next : error));
    } else this.#block(error);
  }
  #block(error: Error) {
    this.#observation?.abort();
    this.ask?.close();
    this.#writer.close();
    this.#error = error;
    this.#listeners.forEach((v) => v.failed(error));
  }
  async snapshot(): Promise<PageSnapshot> {
    await this.#current;
    await this.#views;
    return {
      id: this.page.pageId,
      sharing: this.page.sharing,
      ...structuredClone(this.#projection),
      title: this.#projection.title || text.unknownPageTitle,
      binding: this,
    };
  }
  subscribe(publish: (value: PageView) => void, failed: (error: Error) => void) {
    const listener = { publish, failed };
    this.#listeners.add(listener);
    if (this.#error) failed(this.#error);
    else {
      publish(structuredClone(this.#projection));
      this.#observe();
    }
    return () => {
      this.#listeners.delete(listener);
      // A same-turn subscriber replacement retains the binding (including React's
      // effect replay). Last-consumer release owns socket/Worker/writer cleanup.
      queueMicrotask(() => {
        if (this.#listeners.size === 0) this.close();
      });
    };
  }
  async export(): Promise<ExportBundle> {
    const c = await this.#current;
    const view = await c.run(async () => {
      requireValue(!this.#closed && !this.#error && c === this.#connection);
      const a = c.admission;
      a.validatePage(this.page.sharing);
      requireValue(a.head !== null && a.root !== null);
      const own = this.#admitted.own ?? {};
      const signingKeys: Record<string, Uint8Array> = {};
      for (const writer of Object.keys(own)) {
        const key = c.objects.ownSigningKey(writer);
        if (key) signingKeys[writer] = key;
      }
      return {
        ...this.#admitted,
        own,
        signingKeys,
        spaceId: a.space,
        pageId: a.page,
        epoch: a.epoch,
        membershipHead: { revision: a.head.revision.toString(), statementHash: hex(a.head.hash) },
        exportedAtMs: Date.now(),
      };
    });
    return prepareExport(view);
  }
  async reconnect(): Promise<boolean> {
    if (!this.sessionOwner?.recover || this.#closed) return false;
    // Stop the page's socket, Ask and observer before any new Remote session.
    this.close();
    return this.sessionOwner.recover();
  }
  async edit(source: string, base: string) {
    if (this.#closed || this.#error) throw new Error('Page editing unavailable');
    const c = await this.#current;
    const prepared = await c.run(() => {
      requireValue(base === this.#admitted.source);
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
    this.#observation?.abort();
    if (typeof document !== 'undefined')
      document.removeEventListener('visibilitychange', this.#visibility);
    this.signal?.removeEventListener('abort', this.#abort);
    this.ask?.close();
    this.#pendingView = null;
    this.#admitted = { source: '', title: '' };
    this.#writer.close();
    this.#connection?.close();
    this.#listeners.clear();
  }
}
