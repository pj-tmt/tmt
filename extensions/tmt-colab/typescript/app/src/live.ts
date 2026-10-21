import type { CompactReport, PageUsage } from './page-usage.js';
import { disclosureScope } from './attachment-service.js';
import { LiveDocumentFiles, type DocumentFiles } from './document-files.js';
import { shortPageId } from './short-links.js';
import { commentsForAnswerBatch } from './answer-batch.js';
import { inspectQuestions } from './questions.js';
import { ThreadStore, commentForAsk, conversationForAsk } from './thread-store.js';
import { readThreads, type DiscussionRef } from './thread-records.js';
import { ThreadStatusCoordinator } from './thread-status-coordinator.js';
import { statusNotificationForAsk } from './thread-status-notification.js';
import { projectThreadPresentation } from './thread-status-presentation.js';
import { ThreadStatusSeen } from './thread-status-view.js';
import { LiveAsk, pageAsks } from './live-ask.js';
import { attachment, digest, encodeBinary, requireValue, text as bytesOf } from '@tmt/colab-client';
import type { Bootstrap, PageInfo } from './bootstrap.js';
import { AdmissionRefusedError, ReopenFailedError, type Registration } from './registration.js';
import type { PageBinding, PageSnapshot, PageView, SaveOutcome, WorkOutcome } from './transport.js';
import { SessionEndedError, SessionEvictedError, type RemoteClient } from './ask-remote.js';
import { classify, SYNC_DISCONNECTED } from './connection-state.js';
import { Admission } from './admission.js';
import { Connection } from './connection.js';
import { Writer, type OwnCompaction } from './writer.js';
import {
  SAVE_REOPEN_MS,
  SaveOutcomeUnknown,
  SaveTooLarge,
  settled,
  settledAgainst,
  type SaveResult,
  type WorkAction,
  type WorkResult,
} from './save.js';
import { hunksOf } from './save-hunks.js';
import { prepareExport, hex, type ExportBundle } from './export.js';
import { gatherAttachments } from './export-attachments.js';
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
  readonly files: DocumentFiles;
  #remote: RemoteClient | null = null;
  #seen?: { deviceId: string; value: ThreadStatusSeen };
  #observation: AbortController | null = null;
  #refreshOlderAsks = true;
  #views = Promise.resolve();
  /** The first view whose threads and asks were verified has been published. Until then the page
   * shows the document alone (`discussionPending`) rather than wait for that crypto (#3894). */
  #discussionVerified = false;
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
      settled: () => this.#settled(),
      publish: (records) => this.#writer.submitOwnRecords(records),
      available: () =>
        !this.#closed && !this.#error && !this.#opening?.pending && this.#discussionVerified,
      scope: () => (this.#error ? 'blocked' : disclosureScope(this.#connection)),
    });
    this.files = new LiveDocumentFiles({
      service: this.discussion.attachments,
      epoch: () => this.page.epoch,
      source: () => this.#admitted.source,
      publish: (records) => this.#writer.submitOwnRecords(records),
      edit: (source, base, change) => this.edit(source, base, change),
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
    this.#projection = { ...this.#projection, agentActivity: undefined };
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
              (writer) => connection.objects.statusWriter(writer),
            );
            if (!context.conversation) return commentForAsk(threads, context);
            const asks = await pageAsks(own, connection.admission, (writer) =>
              connection.objects.ownSigningKey(writer),
            );
            return conversationForAsk(threads, context, asks);
          },
          answerContext: (batch) => {
            const connection = this.#connection;
            if (!connection) throw new Error('Discussion connection unavailable');
            const threads = readThreads(
              this.#admitted.own ?? {},
              { spaceId: bootstrap.space, pageId: page.pageId, epoch: page.epoch },
              (writer) => connection.objects.ownSigningKey(writer),
              (writer) => connection.objects.statusWriter(writer),
            );
            return commentsForAnswerBatch(
              threads,
              batch,
              inspectQuestions(this.#admitted.source),
              registration.deviceId,
            );
          },
          statusContext: (context) => {
            const connection = this.#connection;
            requireValue(connection !== null);
            const threads = readThreads(
              this.#admitted.own ?? {},
              { spaceId: bootstrap.space, pageId: page.pageId, epoch: page.epoch },
              (writer) => connection.objects.ownSigningKey(writer),
              (writer) => connection.objects.statusWriter(writer),
            );
            return statusNotificationForAsk(threads, context, registration.deviceId);
          },
          publish: (root, key, value) => this.#writer.submitOwn(root, key, value),
          connection: () => this.#current,
          observe: () => this.#observe(),
          activityObserved: (agentActivity) => {
            if (
              this.#closed ||
              this.#error ||
              this.ask !== facade ||
              this.registration !== registration
            )
              return;
            this.#projection = { ...this.#projection, agentActivity };
            this.#listeners.forEach((value) => value.publish(structuredClone(this.#projection)));
          },
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
    // #block clears #opening before closing the Connection, so even an
    // in-flight blocked attempt loses ownership before its callbacks run.
    // A retained recovery error fences writes, not the fresh replacement read.
    return (
      !this.#closed &&
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
          // A network failure, or a refusal that may pass, waits for the page's silent retry;
          // any other refusal of the fresh open ends the page with its typed reason.
          if (
            error instanceof TypeError ||
            ((error instanceof ReopenFailedError || error instanceof AdmissionRefusedError) &&
              classify(error).recovery === 'retry')
          )
            throw new RecoveryRequiredError(error);
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
          this.#showDocument(value);
          this.#pendingView = { value: this.#admitted, admission: a, open };
          if (!this.#processingViews) {
            this.#processingViews = true;
            this.#views = Promise.resolve().then(() => this.#publishViews());
          }
        },
        (error) => {
          if (this.#owns(open) && open.connection === c) this.#failed(error);
        },
        ['colab-sync-v1'],
        true,
      );
      this.#connection = open.connection = c;
      await c.ready;
      requireValue(this.#owns(open));
      c.send('subscribe', { cursors: [] });
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
  /** Before the first verified view: the admitted document and own maps, without the threads and
   * asks still being verified. Those are absent rather than empty, so nothing unverified can read
   * as verified, and the page shows once the connection is admitted. */
  #showDocument(value: PageView) {
    if (this.#discussionVerified) return;
    const {
      asks: _asks,
      threads: _threads,
      threadPresentations: _presentations,
      ...document
    } = value;
    this.#projection = {
      ...document,
      discussionPending: true,
      askUnavailable: this.#projection.askUnavailable ?? false,
      agentActivity: this.#projection.agentActivity,
    };
    this.#listeners.forEach((v) => v.publish(structuredClone(this.#projection)));
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
            (writer) => connection.objects.statusWriter(writer),
          );
          if (value.archived) {
            this.#observation?.abort();
            this.#observation = null;
            this.ask?.close();
            this.ask = undefined;
          }
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
            agentActivity: this.#projection.agentActivity,
          };
          this.#discussionVerified = true;
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
      error.message === SYNC_DISCONNECTED &&
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
    const { recovery } = classify(error);
    const sessionEnded = recovery === 'session';
    if (
      error instanceof SessionEvictedError ||
      (sessionEnded && (!this.sessionOwner || (open.pending && open.replaceSession))) ||
      (open.pending && !sessionEnded)
    ) {
      this.#block(error);
      return;
    }
    if (this.#attempts++ < 3 && (sessionEnded || recovery === 'resync')) {
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
    const failure = error;
    // Connection's single socket.onerror/onclose handler owns this opaque
    // message; keep its normalization here while the sync socket owner evolves.
    if (
      error.message === SYNC_DISCONNECTED ||
      (error instanceof SessionEndedError &&
        this.#recovering &&
        this.#opening?.pending &&
        this.#opening.replaceSession)
    )
      // An explicit fresh-session refusal keeps the waiting card while its
      // bounded reload fallback runs; other non-network faults stay terminal.
      error = new RecoveryRequiredError(error);
    const connection = this.#connection;
    this.#opening = null;
    this.#connection = null;
    this.#pendingView = null;
    this.#error = error;
    this.#observation?.abort();
    this.#observation = null;
    this.ask?.close();
    this.ask = undefined;
    this.#writer.close();
    connection?.close(failure);
    this.#listeners.forEach((v) => v.failed(error));
  }
  async snapshot(): Promise<PageSnapshot> {
    await this.#current;
    // The first open does not wait for verification: the document shows, and the threads and asks
    // follow through the binding. A page already verified waits for its latest view as before.
    if (this.#discussionVerified) await this.#views;
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
  /** The connection to take the page revision from. A combine another device made reaches this
   * connection as a notice only. When the serve holds a checkpoint this connection has not
   * recorded, reopen once: catch-up admits that checkpoint and verifies it. Nothing retries, and
   * nothing reopens on a timer; the caller decides what a cut still unseen afterwards means. */
  async #settled(): Promise<Connection> {
    const c = await this.#current;
    if (this.#closed || this.#error || c !== this.#connection || !c.objects.hasUnseenCut())
      return c;
    this.#failed(new Error('RESYNC_REQUIRED'));
    return this.#current;
  }
  async export(): Promise<ExportBundle> {
    const c = await this.#settled();
    const view = await c.run(async () => {
      requireValue(!this.#closed && !this.#error && c === this.#connection);
      const a = c.admission;
      a.validatePage(this.page.sharing);
      requireValue(a.head !== null && a.root !== null);
      const own = this.#admitted.own ?? {};
      const signingKeys: Record<string, Uint8Array> = {};
      const statusWriters: string[] = [];
      for (const writer of Object.keys(own)) {
        const key = c.objects.ownSigningKey(writer);
        if (key) signingKeys[writer] = key;
        if (key && c.objects.statusWriter(writer)) statusWriters.push(writer);
      }
      return {
        ...this.#admitted,
        own,
        signingKeys,
        statusWriters,
        spaceId: a.space,
        pageId: a.page,
        epoch: a.epoch,
        membershipHead: { revision: a.head.revision.toString(), statementHash: hex(a.head.hash) },
        exportedAtMs: Date.now(),
        // The page revision a document attachment is fenced by, taken with the same view.
        revision: await c.objects.revision(),
      };
    });
    const { attachments: document, revision, ...page } = view;
    // Each attachment is read under the authority in force when its turn comes: a page that
    // changed since this view lists the entry as unavailable, never as another version.
    const attachments = await gatherAttachments(
      {
        spaceId: page.spaceId,
        pageId: page.pageId,
        epoch: page.epoch,
        revision,
        document: document ?? [],
        own: page.own,
      },
      (selector) => this.discussion.attachments.read(selector),
    );
    return prepareExport({ ...page, attachmentEntries: attachments });
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
      !this.sessionOwner ||
      this.#closed ||
      (this.#error && !(this.#error instanceof RecoveryRequiredError))
    )
      return Promise.resolve(false);
    // Keep this binding's projection, composers and write fence until a fresh
    // same-device Session and Connection have admitted the replacement view.
    if (!this.#error) this.#block(new RecoveryRequiredError(new Error(SYNC_DISCONNECTED)));
    const stopped = this.#error;
    this.#recovering = Promise.resolve()
      .then(async () => {
        if (this.#closed) return false;
        this.#current = this.#startOpen(true);
        const open = this.#opening!;
        try {
          await this.#current;
          await this.#views;
          if (!this.#owns(open) || this.#error !== stopped) return false;
          // #block retired the old writer. Its existing lock/staging owner keeps
          // exact pending envelopes; replacement itself submits nothing.
          this.#writer = new Writer(this.#writer.key, () => this.#current);
          this.#error = null;
          this.#listeners.forEach((listener) =>
            listener.publish(structuredClone(this.#projection)),
          );
          this.#observe();
          return this.#owns(open) && !this.#error;
        } catch (error) {
          if (this.#closed) return false;
          if (this.#owns(open))
            this.#block(error instanceof Error ? error : new Error(text.reconnectFailed));
          // Only a verified refusal of the fresh Session permits the bounded
          // guidance/reload fallback. Network and admission failures do not.
          if (!(error instanceof SessionEndedError) || !this.sessionOwner?.recover) return false;
          try {
            const recovered = await this.sessionOwner.recover();
            if (this.#closed) return false;
            if (!recovered) this.#block(new Error(text.reconnectFailed));
            this.close();
            return recovered;
          } catch (fallback) {
            if (!this.#closed)
              this.#block(fallback instanceof Error ? fallback : new Error(text.reconnectFailed));
            return false;
          }
        }
      })
      .finally(() => {
        this.#recovering = null;
      });
    return this.#recovering;
  }
  /** Save the source through native publication. `base` is the source the editor opened. While
   * the page is still that source, the whole `source` is saved. When the page moved meanwhile, the
   * serve merges the edit, sent as anchored hunks, into the page as it is now, or refuses it as a
   * `SaveConflict` when the text it anchors to changed; a typed attachment change has no such
   * merge and needs the admitted source as its base. A lost reply is settled by exactly one status
   * request on the reopened connection; the save is never sent again. */
  async edit(
    source: string,
    base: string,
    attachments?: attachment.DocumentChange,
  ): Promise<SaveOutcome> {
    if (this.#closed || this.#error) throw new Error('Page editing unavailable');
    const c = await this.#current;
    requireValue(attachments === undefined || base === this.#admitted.source);
    // The list this save must produce, decided by the same rules native preparation applies.
    const change = attachments === undefined ? undefined : attachment.documentChange(attachments);
    const expected =
      change && !attachment.emptyDocumentChange(change)
        ? attachment.applyDocumentChange(
            this.#admitted.attachments ?? [],
            change,
            hex(await digest(bytesOf(source))),
          )
        : undefined;
    const hunks = change === undefined ? hunksOf(base, source) : undefined;
    const operationId = crypto.randomUUID();
    let result: SaveResult;
    let recovered = false;
    try {
      result = await c.save(operationId, base, source, change, hunks);
    } catch (error) {
      if (!(error instanceof Error) || error instanceof SaveTooLarge || c.active) throw error;
      result = await this.#settleLost(c, operationId);
      recovered = true;
    }
    // The first hunk that does not fit is named from the base the editor holds.
    settledAgainst(result, hunks);
    // The record of a recovered save does not say whether it merged, so its revision is the other
    // way to tell that the page shows it.
    await this.#shows(
      source,
      expected,
      result.merged ? result.sourceSha256 : undefined,
      hunks && recovered ? result.revision : undefined,
    );
    return { merged: result.merged === true };
  }
  /** Whether this device may change the page's work: the owner device's alone. */
  canWork(): boolean {
    return (
      !this.#closed &&
      !this.#error &&
      this.#connection?.admission.ownerDevice(this.registration.deviceId) === true
    );
  }
  /** One closed work action, committed by the serve as the owner's own write. Nothing is applied
   * here: the new value arrives through the page's own stream, and this resolves once the page
   * shows the commit (or the wait ran out). A refusal throws `SaveRefused` with the serve's code;
   * a lost reply is settled by exactly one status request, and the action is never sent again. */
  async work(action: WorkAction, body: Record<string, unknown>): Promise<WorkOutcome> {
    if (this.#closed || this.#error) throw new Error('Page editing unavailable');
    const c = await this.#current;
    const operationId = crypto.randomUUID();
    let result: WorkResult;
    try {
      result = await c.work(operationId, action, body);
    } catch (error) {
      if (!(error instanceof Error) || c.active) throw error;
      result = await this.#settleLost(c, operationId);
    }
    settled(result);
    await this.#showsRevision(result.revision);
    return {
      ...(result.taskId === undefined ? {} : { taskId: result.taskId }),
      ...(result.childPage === undefined ? {} : { childPage: result.childPage }),
    };
  }
  /** The serve counts every device but its own as "other". This device's own changes are not
   * other devices': it can combine them itself, so the page-size surface counts them as its own. */
  #mine(usage: PageUsage, own: number): PageUsage {
    return {
      ...usage,
      otherDeviceUpdates: Math.max(0, usage.otherDeviceUpdates - Math.min(own, usage.updates)),
    };
  }
  /** How full the page is now. Never retried: the next request asks again. */
  async usage(): Promise<PageUsage> {
    if (this.#closed || this.#error) throw new Error('Page usage unavailable');
    const c = await this.#current;
    const usage = await c.usage();
    return this.#mine(usage, c.objects.tail(this.registration.deviceId));
  }
  /** Combine the serve's own stream, then this device's: each device combines only its own. The
   * page's content is unchanged, so the editor's base is too; nothing is saved or resent. */
  async compact(): Promise<CompactReport> {
    if (this.#closed || this.#error) throw new Error('Page compaction unavailable');
    const c = await this.#current;
    const device = this.registration.deviceId;
    const before = c.objects.tail(device);
    const serve = await c.compact();
    // The serve folds this device's stream too, and that checkpoint reaches this connection as a
    // notice only: catch up to it first, or the checkpoint this device seals next would take the
    // same position with other bytes and be refused.
    const settled = await this.#settled();
    let own: OwnCompaction;
    try {
      own = await this.#writer.compact();
    } catch {
      own = { outcome: 'not-combined' };
    }
    const combined = serve.compacted || own.outcome === 'combined';
    // After this device combined, ask again so `after` is what the serve now counts.
    const after = own.outcome === 'combined' ? await settled.usage() : serve.after;
    return {
      pageId: serve.pageId,
      compacted: combined,
      ...(combined
        ? {}
        : {
            skipped:
              serve.skipped === 'not-combined' || own.outcome === 'not-combined'
                ? ('not-combined' as const)
                : ('nothing-to-combine' as const),
          }),
      by: serve.by,
      through: serve.through,
      mergedObjects: serve.mergedObjects + (own.outcome === 'combined' ? own.merged : 0),
      sourceSha256: serve.sourceSha256,
      before: this.#mine(serve.before, before),
      after: this.#mine(after, settled.objects.tail(device)),
    };
  }
  async #settleLost(lost: Connection, operationId: string): Promise<SaveResult> {
    try {
      const next = await this.#reopened(lost);
      return await next.saveStatus(operationId);
    } catch {
      throw new SaveOutcomeUnknown(operationId);
    }
  }
  /** The connection Live reopens after `lost` failed, once it is caught up. */
  async #reopened(lost: Connection): Promise<Connection> {
    const deadline = Date.now() + SAVE_REOPEN_MS;
    while (!this.#closed && !this.#error && Date.now() < deadline) {
      const next = await this.#current.catch(() => null);
      if (next && next !== lost && next.active) return next;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    throw new Error('Page connection unavailable');
  }
  /** The admitted page shows the saved source, so the editor's base moves with it. A merged save
   * is saved as a source the editor never held: it is shown once the page holds the digest the
   * serve named, or, for a save whose reply was lost, the revision its record names. */
  async #shows(
    source: string,
    attachments?: readonly attachment.AttachmentDescriptor[],
    mergedDigest?: string,
    revision?: string,
  ) {
    const deadline = Date.now() + SAVE_REOPEN_MS;
    const shown = async () =>
      mergedDigest !== undefined
        ? encodeBinary(await digest(bytesOf(this.#admitted.source))) === mergedDigest
        : (this.#admitted.source === source &&
            (attachments === undefined ||
              JSON.stringify(this.#admitted.attachments ?? []) === JSON.stringify(attachments))) ||
          (revision !== undefined &&
            (await this.#connection?.objects.revision().catch(() => null)) === revision);
    while (!(await shown()) && !this.#closed && !this.#error) {
      if (Date.now() >= deadline) return;
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
  }
  async #showsRevision(revision: string | undefined) {
    if (revision === undefined) return;
    const deadline = Date.now() + SAVE_REOPEN_MS;
    while (
      !this.#closed &&
      !this.#error &&
      Date.now() < deadline &&
      (await this.#connection?.objects.revision().catch(() => null)) !== revision
    )
      await new Promise((resolve) => setTimeout(resolve, 25));
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
