import { shortPageId } from './short-links.js';
import { ThreadStore, commentForAsk, conversationForAsk } from './thread-store.js';
import { readThreads, type DiscussionRef } from './thread-records.js';
import { ThreadStatusCoordinator } from './thread-status-coordinator.js';
import { statusNotificationForAsk } from './thread-status-notification.js';
import { projectThreadPresentation } from './thread-status-presentation.js';
import { ThreadStatusSeen } from './thread-status-view.js';
import { LiveAsk, pageAsks } from './live-ask.js';
import { requireValue } from '@tmt/colab-client';
import type { Bootstrap, PageInfo } from './bootstrap.js';
import type { Registration } from './registration.js';
import type { PageBinding, PageSnapshot, PageView } from './transport.js';
import { SessionEndedError, SessionEvictedError, type RemoteClient } from './ask-remote.js';
import { Admission } from './admission.js';
import { Connection } from './connection.js';
import { Writer } from './writer.js';
import { prepareExport, hex, type ExportBundle } from './export.js';
import { text } from './strings.js';
import { RecoveryRequiredError } from './session-recovery.js';

export interface LiveSession {
  registration: Registration;
  remote: RemoteClient | null;
}
export interface LiveSessionOwner {
  rememberTitle?(page: string, title: string, registration: Registration): Promise<void>;
  recover?(): Promise<boolean>;
  reconnect(previous: Registration): Promise<LiveSession>;
}

interface LiveOpen {
  replaceSession: boolean;
  pending: boolean;
  registration: Registration;
  remote: RemoteClient | null;
  connection: Connection | null;
}

/** Mounted update-only page binding. A fresh reconnect reconstructs from seq 1;
 * unsupported history is a blocking failure rather than a partial projection. */
export class Live implements PageBinding {
  ask?: LiveAsk;
  readonly discussion: ThreadStore;
  readonly status: ThreadStatusCoordinator;
  #remote: RemoteClient | null = null;
  #seen?: { deviceId: string; value: ThreadStatusSeen };
  #observation: AbortController | null = null;
  #refreshOlderAsks = true;
  #views = Promise.resolve();
  #processingViews = false;
  #pendingView: { value: PageView; admission: Admission; open: LiveOpen } | null = null;
  #admitted: PageView = { source: '', title: '' };
  #connection: Connection | null = null;
  #current: Promise<Connection>;
  #writer: Writer;
  #closed = false;
  #opening: LiveOpen | null = null;
  #attempts = 0;
  #diagnosing: LiveOpen | null = null;
  #projection: PageView = { source: '', title: '' };
  #listeners = new Set<{ publish(value: PageView): void; failed(error: Error): void }>();
  #error: Error | null = null;
  #recovering: Promise<boolean> | null = null;
  constructor(
    readonly mount: URL,
    readonly bootstrap: Bootstrap,
    public registration: Registration,
    readonly page: PageInfo,
    readonly signal?: AbortSignal,
    remote: RemoteClient | null = null,
    private sessionOwner?: LiveSessionOwner,
  ) {
    this.#remote = remote;
    this.#current = this.#startOpen();
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
      available: () => !this.#closed && !this.#error && !this.#opening?.pending,
    });
    this.status = new ThreadStatusCoordinator({
      binding: this.discussion,
      asks: () => this.#projection.asks ?? [],
      destinations: async () => (this.ask ? this.ask.destinations() : []),
      notify: async (status, recipient, thread) =>
        this.ask
          ? this.ask.notifyStatus(
              { status: status.ref, recipient, thread },
              this.#admitted.title,
              this.mount.href,
            )
          : { adopted: false, reason: 'RECIPIENT_UNAVAILABLE' },
    });
    this.#replaceAsk(remote);
    if (typeof document !== 'undefined')
      document.addEventListener('visibilitychange', this.#visibility);
    signal?.addEventListener('abort', this.#abort, { once: true });
    if (signal?.aborted) this.close();
  }
  #replaceAsk(remote: RemoteClient | null) {
    this.#refreshOlderAsks = true;
    this.#remote = remote;
    this.ask?.close();
    const { bootstrap, page, registration } = this;
    const facade: LiveAsk | undefined = remote
      ? new LiveAsk({
          remote,
          space: bootstrap.space,
          page: page.pageId,
          shortId: shortPageId(
            page.pageId,
            bootstrap.pageIds.map((page) => page.pageId),
          ),
          sharing: page.sharing,
          deviceId: registration.deviceId,
          key: registration.keys.sign,
          publicKey: registration.keys.signPublic,
          own: () => this.#admitted.own ?? {},
          commentContext: async (context) => {
            const connection = this.#connection;
            if (!connection) throw new Error('Discussion connection unavailable');
            const own = this.#admitted.own ?? {};
            const threads = readThreads(
              own,
              { spaceId: bootstrap.space, pageId: page.pageId, epoch: page.epoch },
              (writer) => connection.objects.ownSigningKey(writer),
            );
            if (!context.conversation) return commentForAsk(threads, context);
            const asks = await pageAsks(own, connection.admission, (writer) =>
              connection.objects.ownSigningKey(writer),
            );
            return conversationForAsk(threads, context, asks);
          },
          statusContext: (context) => {
            const connection = this.#connection;
            requireValue(connection !== null);
            const threads = readThreads(
              this.#admitted.own ?? {},
              { spaceId: bootstrap.space, pageId: page.pageId, epoch: page.epoch },
              (writer) => connection.objects.ownSigningKey(writer),
            );
            return statusNotificationForAsk(threads, context, registration.deviceId);
          },
          publish: (root, key, value) => this.#writer.submitOwn(root, key, value),
          connection: () => this.#current,
          observe: () => this.#observe(),
          observationUnavailable: (unavailable) => {
            if (
              this.#closed ||
              this.#error ||
              this.ask !== facade ||
              this.registration !== registration ||
              !this.#observation ||
              this.#observation.signal.aborted
            )
              return;
            if (this.#projection.askUnavailable === unavailable) return;
            this.#projection = { ...this.#projection, askUnavailable: unavailable };
            this.#listeners.forEach((value) => value.publish(structuredClone(this.#projection)));
          },
          sessionEnded: (error) => {
            if (this.ask === facade && this.registration === registration)
              this.#failed(error ?? new SessionEndedError('REMOTE_SESSION_ENDED'));
          },
        })
      : undefined;
    this.ask = facade;
  }
  #abort = () => this.close();
  #visibility = () => {
    if (document.visibilityState === 'hidden') {
      this.#observation?.abort();
    } else {
      this.#refreshOlderAsks = true;
      this.#observe();
    }
  };
  #observe() {
    if (
      this.#closed ||
      this.#opening?.pending ||
      !this.ask ||
      this.#error ||
      this.#observation ||
      this.#listeners.size === 0 ||
      (typeof document !== 'undefined' && document.visibilityState === 'hidden')
    )
      return;
    const controller = new AbortController();
    const refreshOlder = this.#refreshOlderAsks;
    this.#refreshOlderAsks = false;
    this.#observation = controller;
    void this.ask
      .observe(controller.signal, refreshOlder)
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
  #owns(open: LiveOpen) {
    return (
      !this.#closed &&
      !this.#error &&
      this.#opening === open &&
      this.registration === open.registration &&
      this.#remote === open.remote
    );
  }
  #startOpen(replaceSession = false) {
    const open: LiveOpen = {
      replaceSession,
      pending: true,
      registration: this.registration,
      remote: this.#remote,
      connection: null,
    };
    this.#opening = open;
    const current = this.#open(open);
    void current.catch((error) => {
      // Connection.close rejects ready before its failure callback starts the
      // exact old-session diagnosis. That one read owns the pending outcome.
      if (this.#owns(open) && this.#diagnosing !== open)
        this.#block(error instanceof Error ? error : new Error('Sync unavailable'));
    });
    return current;
  }
  async #open(open: LiveOpen): Promise<Connection> {
    let ready = false;
    try {
      if (open.replaceSession) {
        if (!this.sessionOwner) throw new Error('Session replacement unavailable');
        const session = await this.sessionOwner.reconnect(open.registration).catch((error) => {
          if (error instanceof TypeError) throw new RecoveryRequiredError(error);
          throw error;
        });
        requireValue(this.#owns(open));
        this.registration = open.registration = session.registration;
        open.remote = session.remote;
        this.#replaceAsk(session.remote);
      }
      const a = new Admission(
        this.bootstrap.space,
        this.page.pageId,
        this.page.epoch,
        this.bootstrap.owner,
        open.registration,
      );
      await a.restore();
      requireValue(this.#owns(open));
      const c = new Connection(
        a,
        this.mount,
        this.page.sharing,
        (value) => {
          if (!this.#owns(open)) return;
          this.#admitted = structuredClone(value);
          this.#pendingView = { value: this.#admitted, admission: a, open };
          if (!this.#processingViews) {
            this.#processingViews = true;
            this.#views = Promise.resolve().then(() => this.#publishViews());
          }
        },
        (error) => {
          if (this.#owns(open) && open.connection === c) this.#failed(error);
        },
      );
      this.#connection = open.connection = c;
      await c.ready;
      requireValue(this.#owns(open));
      this.#attempts = 0;
      ready = true;
      return c;
    } finally {
      if (ready && this.#owns(open)) {
        open.pending = false;
        this.#observe();
      }
    }
  }
  async #publishViews(): Promise<void> {
    try {
      // At most one active and one latest pending snapshot: slow crypto never
      // builds an unbounded queue of detached own documents.
      while (this.#pendingView && !this.#closed) {
        const { value, admission, open } = this.#pendingView;
        this.#pendingView = null;
        const connection = open.connection;
        if (!this.#owns(open) || !connection || connection.admission !== admission) continue;
        try {
          const asks = await pageAsks(value.own ?? {}, admission, (writer) =>
            connection.objects.ownSigningKey(writer),
          );
          if (!this.#owns(open) || this.#pendingView) continue;
          await this.sessionOwner?.rememberTitle?.(
            this.page.pageId,
            value.title,
            open.registration,
          );
          if (!this.#owns(open) || this.#pendingView) continue;
          const threads = readThreads(
            value.own ?? {},
            { spaceId: admission.space, pageId: admission.page, epoch: admission.epoch },
            (writer) => connection.objects.ownSigningKey(writer),
          );
          this.#projection = {
            ...value,
            asks,
            threads,
            threadPresentations: threads.map((thread) =>
              projectThreadPresentation(
                thread,
                asks,
                this.#statusSeen(),
                admission.ownerDevice(this.registration.deviceId),
              ),
            ),
            askUnavailable: this.#projection.askUnavailable ?? false,
          };
          this.#listeners.forEach((v) => v.publish(structuredClone(this.#projection)));
          this.#observe();
        } catch (error) {
          if (this.#owns(open))
            this.#block(error instanceof Error ? error : new Error('Ask data unavailable'));
        }
      }
    } finally {
      this.#processingViews = false;
    }
  }
  #failed(error: Error) {
    const open = this.#opening;
    if (!open || !this.#owns(open)) return;
    // Mounted WebSocket close does not carry Remote's signed reason. Read once
    // against the old session before deciding whether to reopen or show eviction.
    const remote = open.remote;
    if (
      error.message === 'Sync disconnected' &&
      remote &&
      typeof remote.listAgents === 'function' &&
      !(open.pending && open.replaceSession)
    ) {
      if (this.#diagnosing === open) return;
      this.#diagnosing = open;
      void remote
        .listAgents()
        .then(
          () => {
            if (this.#owns(open)) this.#recoverFailure(error);
          },
          (reason: unknown) => {
            if (this.#owns(open))
              this.#recoverFailure(
                reason instanceof SessionEvictedError || reason instanceof SessionEndedError
                  ? reason
                  : error,
              );
          },
        )
        .finally(() => {
          if (this.#diagnosing === open) this.#diagnosing = null;
        });
      return;
    }
    this.#recoverFailure(error);
  }
  #recoverFailure(error: Error) {
    const open = this.#opening;
    if (!open || !this.#owns(open)) return;
    const sessionEnded =
      error instanceof SessionEndedError || error.message === 'Remote session ended';
    if (
      error instanceof SessionEvictedError ||
      (sessionEnded && (!this.sessionOwner || (open.pending && open.replaceSession))) ||
      (open.pending && !sessionEnded)
    ) {
      this.#block(error);
      return;
    }
    if (
      this.#attempts++ < 3 &&
      (sessionEnded ||
        ['Sync disconnected', 'RESYNC_REQUIRED', 'Fresh membership catchup required'].includes(
          error.message,
        ))
    ) {
      // Retire this attempt before close callbacks or late readiness can run.
      this.#opening = null;
      this.#observation?.abort();
      this.#observation = null;
      this.ask?.close();
      this.ask = undefined;
      this.#pendingView = null;
      this.#listeners.forEach((v) => v.publish(structuredClone(this.#projection)));
      this.#connection = null;
      open.connection?.close();
      // A tunnel resync retains the verified Session and Remote port. Only a
      // session fault may replace them and end other mounted tunnels.
      if (!sessionEnded) this.#replaceAsk(this.#remote);
      this.#current = this.#startOpen(sessionEnded);
    } else this.#block(error);
  }
  #block(error: Error) {
    if (error.message === 'Sync disconnected') error = new RecoveryRequiredError(error);
    const connection = this.#connection;
    this.#opening = null;
    this.#connection = null;
    this.#pendingView = null;
    this.#error = error;
    this.#observation?.abort();
    this.ask?.close();
    this.#writer.close();
    connection?.close();
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
  #statusSeen() {
    const deviceId = this.registration.deviceId;
    if (this.#seen?.deviceId === deviceId) return this.#seen.value;
    let storage: Pick<Storage, 'getItem' | 'setItem'>;
    try {
      storage = globalThis.localStorage;
      requireValue(storage !== undefined);
    } catch {
      storage = { getItem: () => null, setItem: () => {} };
    }
    const value = new ThreadStatusSeen(
      { spaceId: this.bootstrap.space, pageId: this.page.pageId, epoch: this.page.epoch },
      deviceId,
      storage,
    );
    this.#seen = { deviceId, value };
    return value;
  }
  /** Called only by the trusted parent open handler, never from rendering. */
  markThreadStatusSeen(ref: DiscussionRef) {
    if (this.#closed) return;
    const threads = this.#projection.threads ?? [];
    const matches = threads.filter(
      (thread) => thread.ref.writer === ref.writer && thread.ref.id === ref.id,
    );
    if (matches.length !== 1) return;
    const seen = this.#statusSeen();
    seen.opened(matches[0]);
    this.#projection = {
      ...this.#projection,
      threadPresentations: threads.map((thread) =>
        projectThreadPresentation(
          thread,
          this.#projection.asks ?? [],
          seen,
          this.#connection?.admission.ownerDevice(this.registration.deviceId) ?? false,
        ),
      ),
    };
    this.#listeners.forEach((listener) => listener.publish(structuredClone(this.#projection)));
  }
  reconnect(): Promise<boolean> {
    if (this.#recovering) return this.#recovering;
    if (
      !this.sessionOwner?.recover ||
      this.#closed ||
      (this.#error && !(this.#error instanceof RecoveryRequiredError))
    )
      return Promise.resolve(false);
    // Stop the page's socket, Ask and observer before any new Remote session.
    // Keep the admitted projection and subscribers through a network failure.
    if (!this.#error) this.#block(new RecoveryRequiredError(new Error('Sync disconnected')));
    this.#recovering = this.sessionOwner
      .recover()
      .then((recovered) => {
        if (!recovered && !this.#closed) this.#block(new Error(text.reconnectFailed));
        this.close();
        return recovered;
      })
      .catch((error: unknown) => {
        if (!this.#closed && error instanceof RecoveryRequiredError) {
          this.#error = error;
          this.#listeners.forEach((listener) => listener.failed(error));
        } else {
          if (!this.#closed)
            this.#block(error instanceof Error ? error : new Error(text.reconnectFailed));
          this.close();
        }
        return false;
      })
      .finally(() => {
        this.#recovering = null;
      });
    return this.#recovering;
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
    this.#opening = null;
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
