import type { CommentContext, commentForAsk } from './thread-store.js';
import { requireValue } from '@tmt/colab-client';
import { AskController } from './ask-attempt.js';
import type { AskDestination, AdmittedSelection } from './ask-intent.js';
import { AskRecordStore } from './ask-record-store.js';
import { readAskViews, type AskRoot } from './ask-records.js';
import type { RemoteClient, RemoteAgent, SessionEvictedError } from './ask-remote.js';
import type { AskBinding, PageAsk } from './ask-panel.js';
import type { PreviewAttempt } from './ask-preview.js';
import type { Admission } from './admission.js';
import type { Connection } from './connection.js';
import { text } from './strings.js';
import type { JsonValue, OwnState } from './fold-protocol.js';

export type AgentDestination = AskDestination & { presence?: RemoteAgent['presence'] };
export interface LiveAskOptions {
  space: string;
  page: string;
  shortId?: string;
  sharing: string;
  deviceId: string;
  key: CryptoKey;
  publicKey: Uint8Array;
  own(): OwnState;
  commentContext?(
    context: CommentContext,
  ): ReturnType<typeof commentForAsk> | Promise<ReturnType<typeof commentForAsk>>;
  publish(root: AskRoot, key: string, value: JsonValue): Promise<void>;
  connection(): Promise<Connection>;
  remote: RemoteClient;
  observe?(): void;
  sessionEnded?(error?: SessionEvictedError): void;
}

/** Page composition only. The controller owns operation policy and the Writer
 * owns encrypted publication. Constructing or reconnecting this facade sends
 * nothing; all effects start from a trusted parent's explicit action. */
export class LiveAsk implements AskBinding {
  #controller: AskController;
  #store: AskRecordStore;
  #selection: AdmittedSelection | null = null;
  #closed = false;
  constructor(private options: LiveAskOptions) {
    const store = (this.#store = new AskRecordStore({
      space: options.space,
      page: options.page,
      deviceId: options.deviceId,
      publicKey: options.publicKey,
      readOwn: options.own,
      publish: options.publish,
    }));
    this.#controller = new AskController({
      store,
      remote: options.remote,
      key: options.key,
      sessionEnded: (error) => {
        if (!this.#closed) options.sessionEnded?.(error);
      },
      selection: () => {
        requireValue(this.#selection !== null);
        return this.#selection;
      },
    });
  }
  async destinations(): Promise<AgentDestination[]> {
    requireValue(!this.#closed);
    const controller = this.#controller;
    const snapshot = await controller.destinations();
    requireValue(!this.#closed);
    return snapshot.machines.flatMap((machine) =>
      machine.agents.map((agent) => ({
        machine: machine.id,
        machineName: machine.name,
        online: machine.online,
        agent: agent.id,
        agentName: agent.name,
        presence: agent.presence,
        grantExpiresAt: snapshot.context.expiresAtMs,
        deviceName: snapshot.context.deviceName,
        grantRevision: snapshot.context.grantRevision,
        mode: snapshot.context.mode,
      })),
    );
  }
  async #admit(): Promise<Connection> {
    const c = await this.options.connection();
    await c.run(async () => {
      requireValue(!this.#closed && c.active);
      const a = c.admission;
      requireValue(a.head !== null && a.root !== null);
      a.validatePage(this.options.sharing);
      a.author(this.options.deviceId, a.head.revision.toString());
    });
    return c;
  }
  async prepare(input: Parameters<AskBinding['prepare']>[0]): Promise<PreviewAttempt> {
    const captured = structuredClone(input);
    requireValue(!this.#closed);
    const controller = this.#controller;
    const c = await this.#admit();
    const preview = await c.run(async () => {
      requireValue(!this.#closed && c.active);
      const origin = captured.context
        ? await this.options.commentContext?.(captured.context)
        : undefined;
      requireValue(!this.#closed && c.active && (!captured.context || origin !== undefined));
      this.#selection = {
        space: this.options.space,
        page: this.options.page,
        thread: origin?.thread ?? crypto.randomUUID(),
        messageIds: origin?.messageIds ?? [],
        senderDevice: this.options.deviceId,
        quote: origin?.quote ?? captured.quote,
        comment: origin?.comment ?? captured.comment,
        title: captured.title,
        url: captured.url,
        shortId: this.options.shortId,
      };
      return controller.prepare(captured.destination);
    });
    let state: PreviewAttempt['state'] = { state: 'preview' };
    let pending: ReturnType<PreviewAttempt['send']> | undefined;
    return {
      preview,
      available: true,
      get state() {
        return { ...state };
      },
      send: () => {
        if (pending) return pending;
        state = { state: 'preparing' };
        pending = (async () => {
          try {
            await this.#admit();
            state = { state: (await controller.send(preview)).state };
            this.options.observe?.();
          } catch {
            const view = (await this.#store.views()).find(
              (value) => value.intent.operationId === preview.view.operationId,
            );
            state = {
              state: view?.state === 'dispatching' ? 'uncertain' : (view?.state ?? 'failed'),
            };
          }
          return { ...state };
        })();
        return pending;
      },
    };
  }
  async recheck(operationId: string): Promise<void> {
    requireValue(!this.#closed);
    const controller = this.#controller;
    await this.#admit();
    await controller.recover(operationId);
    this.options.observe?.();
  }
  async abandon(operationId: string): Promise<void> {
    requireValue(!this.#closed);
    const controller = this.#controller;
    await this.#admit();
    await controller.abandon(operationId);
  }
  async observe(signal: AbortSignal, refreshOlder = false): Promise<void> {
    requireValue(!this.#closed);
    const controller = this.#controller;
    signal.throwIfAborted();
    await controller.observe(signal, refreshOlder);
  }
  close(): void {
    this.#closed = true;
  }
}

/** Each key comes from admitted stream authority, never record author fields.
 * Crypto is complete before these detached presentation rows reach React. */
export async function pageAsks(
  own: OwnState,
  admission: Admission,
  signingKey: (writer: string) => Uint8Array | undefined,
): Promise<PageAsk[]> {
  const views = await readAskViews(own, admission, signingKey);
  let canPublish = false;
  try {
    requireValue(admission.head !== null);
    admission.author(admission.registration.deviceId, admission.head.revision.toString());
    canPublish = true;
  } catch {
    // Historical read admission never supplies current write authority.
  }
  return views.map((view) => ({
    operationId: view.intent.operationId,
    thread: view.intent.thread,
    messageIds: view.intent.messageIds,
    writer: view.writer,
    message: view.intent.message,
    deliveredMessage: `[remote: ${view.deviceName}]\n${view.intent.message}`,
    agent: view.intent.agent,
    agentName: view.agentName || text.askAgentLabel,
    deviceName:
      view.writer === admission.registration.deviceId
        ? text.askYou
        : view.deviceName || text.askDeviceLabel,
    issuedAt: view.intent.issuedAt,
    machine: view.intent.machine,
    state: view.state,
    reason: view.reason,
    canTrack: canPublish && view.writer === admission.registration.deviceId,
    ...(view.reply ? { reply: view.reply.body } : {}),
    resultUnavailable: ['RESULT_UNAVAILABLE', 'REPLY_TOO_LARGE'].includes(view.reason ?? ''),
  }));
}
