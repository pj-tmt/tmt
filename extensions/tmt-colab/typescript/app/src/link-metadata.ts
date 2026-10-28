import { EDIT_WAITING_MS } from './link-edit.js';
import { metadataRequest } from '@tmt/colab-client';
import type { OwnState, OwnRecord } from './fold-protocol.js';

export type MetadataInput =
  | { action: 'creator.set'; body: { selector: string }; target: Record<string, unknown> }
  | {
      action: 'question.handle' | 'question.reopen';
      body: { id: string };
      target: Record<string, unknown>;
    };
export type MetadataOutcome = { operationId: string } & (
  | { state: 'applied'; revision: string }
  | { state: 'refused'; code: string; message: string }
  | { state: 'unconfirmed' | 'not-sent' | 'ended' }
);
export interface MetadataStatus {
  readonly request: metadataRequest.MetadataRequest;
  readonly busy: boolean;
  readonly outcome?: MetadataOutcome;
}
/** One page/epoch/writer lifetime. Checking an operation only observes; it never sends. */
export interface ReaderMetadata {
  readonly scope: string;
  snapshot(): MetadataStatus | undefined;
  subscribe(listener: () => void): () => void;
  send(input: MetadataInput): Promise<MetadataOutcome>;
  check(operationId: string): Promise<MetadataOutcome>;
}
export type MetadataSend = (
  request: metadataRequest.MetadataRequest,
) => Promise<'sent' | 'refused' | 'unconfirmed'>;

export function buildMetadataRequest(input: MetadataInput): metadataRequest.MetadataRequest {
  return metadataRequest.metadataRequest(
    structuredClone({
      version: 2,
      kind: 'edit-request',
      operationId: crypto.randomUUID(),
      ...input,
    }),
  );
}
export const metadataRecord = (request: metadataRequest.MetadataRequest): OwnRecord => ({
  root: 'intents',
  key: request.operationId,
  value: request as unknown as OwnRecord['value'],
});

interface Pending {
  request: metadataRequest.MetadataRequest;
  result?: MetadataOutcome;
  wake: Set<() => void>;
}
/** Observes owner-authenticated receipts, independently of dialogs and source drafts. The sender
 * is invoked once per deliberate action; reconnect, timeout and check retain that exact request. */
export class MetadataRequests implements ReaderMetadata {
  #ended = false;
  #pending = new Map<string, Pending>();
  #own: OwnState = {};
  #owners = new Set<string>();
  #status?: MetadataStatus;
  #listeners = new Set<() => void>();
  readonly snapshot = (): MetadataStatus | undefined => this.#status;
  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };
  #notify(): void {
    for (const listener of this.#listeners) listener();
  }
  #report(entry: Pending, outcome: MetadataOutcome): MetadataOutcome {
    const settled = this.#ended ? this.#state(entry, 'ended') : (entry.result ?? outcome);
    if (this.#status?.request.operationId === entry.request.operationId) {
      this.#status = { request: entry.request, busy: false, outcome: settled };
      this.#notify();
    }
    return settled;
  }
  constructor(
    readonly scope: string,
    readonly writer: string,
    private readonly publish: MetadataSend,
    private readonly waitingMs = EDIT_WAITING_MS,
  ) {}
  async send(input: MetadataInput): Promise<MetadataOutcome> {
    const request = buildMetadataRequest(input);
    const entry: Pending = { request, wake: new Set() };
    this.#pending.set(request.operationId, entry);
    this.#status = { request, busy: true };
    this.#notify();
    if (this.#ended) return this.#report(entry, this.#state(entry, 'ended'));
    let sent: Awaited<ReturnType<MetadataSend>>;
    try {
      sent = await this.publish(request);
    } catch {
      sent = 'unconfirmed';
    }
    if (this.#ended) return this.#report(entry, this.#state(entry, 'ended'));
    if (sent === 'refused') {
      entry.result ??= this.#state(entry, 'not-sent');
      return this.#report(entry, entry.result);
    }
    // Even an uncertain append may already have an owner receipt. Retain its identity for Check.
    await this.#scan(entry);
    if (entry.result) return this.#report(entry, entry.result);
    const outcome =
      sent === 'unconfirmed' ? this.#state(entry, 'unconfirmed') : await this.#answer(entry);
    return this.#report(entry, outcome);
  }
  async check(operationId: string): Promise<MetadataOutcome> {
    const entry = this.#pending.get(operationId);
    if (!entry) return { operationId, state: 'ended' };
    if (this.#status?.request.operationId === operationId) {
      this.#status = { ...this.#status, busy: true };
      this.#notify();
    }
    return this.#report(entry, await this.#answer(entry));
  }
  /** Only admitted projections and Objects.statusWriter observations may enter here. */
  show(own: OwnState, ownerMember: (writer: string) => boolean): void {
    if (this.#ended) return;
    this.#own = own;
    this.#owners = new Set(Object.keys(own).filter(ownerMember));
    for (const entry of this.#pending.values()) void this.#scan(entry);
  }
  end(): void {
    this.#ended = true;
    this.#own = {};
    this.#owners.clear();
    if (this.#status) {
      this.#status = {
        request: this.#status.request,
        busy: false,
        outcome: { operationId: this.#status.request.operationId, state: 'ended' },
      };
      this.#notify();
    }
    for (const entry of this.#pending.values()) for (const wake of entry.wake) wake();
  }
  #state(entry: Pending, state: 'ended' | 'not-sent' | 'unconfirmed'): MetadataOutcome {
    return { operationId: entry.request.operationId, state };
  }
  async #scan(entry: Pending): Promise<void> {
    if (entry.result || this.#ended) return;
    for (const author of this.#owners) {
      const record =
        this.#own[author]?.messages[
          metadataRequest.outcomeKey(this.writer, entry.request.operationId)
        ];
      if (record === undefined) continue;
      try {
        const result = await metadataRequest.resultFor(record, true, this.writer, entry.request);
        if (!result || entry.result || this.#ended) continue;
        entry.result =
          result.state === 'rejected'
            ? {
                operationId: entry.request.operationId,
                state: 'refused',
                code: result.code,
                message: result.message,
              }
            : {
                operationId: entry.request.operationId,
                state: 'applied',
                revision: result.revision,
              };
        this.#report(entry, entry.result);
        for (const wake of entry.wake) wake();
        return;
      } catch {
        // A malformed or unbound record cannot settle this operation.
      }
    }
  }
  async #answer(entry: Pending): Promise<MetadataOutcome> {
    await this.#scan(entry);
    if (this.#ended) return this.#state(entry, 'ended');
    if (entry.result) return entry.result;
    return new Promise((resolve) => {
      const finish = () => {
        clearTimeout(timer);
        entry.wake.delete(finish);
        resolve(
          this.#ended
            ? this.#state(entry, 'ended')
            : (entry.result ?? this.#state(entry, 'unconfirmed')),
        );
      };
      const timer = setTimeout(finish, this.waitingMs);
      entry.wake.add(finish);
    });
  }
}
