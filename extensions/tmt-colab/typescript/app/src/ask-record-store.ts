import { generatedId, requireValue, spaceId } from '@tmt/colab-client';
import type { SignedAsk } from './ask-intent.js';
import type { OwnState, JsonValue } from './fold-protocol.js';
import { deleteRecord, record } from './storage.js';
import { provablyUnsent } from './ask-remote.js';
import {
  canTransition,
  readAskRecords,
  recordKey,
  validateRecord,
  verifyAsk,
  type AskLedgerView,
  type AskRecord,
  type AskRoot,
  type LedgerState,
} from './ask-records.js';

export type StoredAskDraft = Pick<SignedAsk, 'input' | 'signature'>;
/** Only signed metadata is reserved locally. A stored reservation never
 * authorizes another effect after failed publication or browser restart. */
export async function storeAskDraft(intent: SignedAsk): Promise<'created' | 'existing'> {
  const key = `ask:${intent.senderDevice}:${intent.operationId}`;
  return await navigator.locks.request(key, async () => {
    const previous = await record<StoredAskDraft>(key);
    if (previous) {
      if (previous.input !== intent.input || previous.signature !== intent.signature)
        throw new Error('INTENT_CONFLICT');
      return 'existing';
    }
    await record<StoredAskDraft>(key, { input: intent.input, signature: intent.signature });
    return 'created';
  });
}

export interface AskRecordStoreOptions {
  space: string;
  page: string;
  deviceId: string;
  publicKey: Uint8Array;
  readOwn(): OwnState;
  publish(root: AskRoot, key: string, value: JsonValue): Promise<void>;
}
/** Own stream is the public ledger; local metadata only prevents repeat effects.
 * The existing Writer owns ciphertext staging, sequence and exact append retry. */
export class AskRecordStore {
  readonly scope: Readonly<{ space: string; page: string; deviceId: string }>;
  #publicKey: Uint8Array;
  constructor(private options: AskRecordStoreOptions) {
    spaceId(options.space);
    generatedId(options.page);
    generatedId(options.deviceId);
    this.scope = Object.freeze({
      space: options.space,
      page: options.page,
      deviceId: options.deviceId,
    });
    this.#publicKey = options.publicKey.slice();
  }
  async exclusive<T>(id: string, action: () => Promise<T>): Promise<T> {
    generatedId(id);
    return await navigator.locks.request(
      `ask-ledger:${this.scope.space}:${this.scope.page}:${this.scope.deviceId}:${id}`,
      action,
    );
  }
  views(): Promise<AskLedgerView[]> {
    return readAskRecords(this.options.readOwn(), this.scope, (writer) =>
      writer === this.scope.deviceId ? this.#publicKey : undefined,
    );
  }
  async view(id: string): Promise<AskLedgerView> {
    const found = (await this.views()).find((v) => v.intent.operationId === id);
    if (!found) throw new Error('ASK_NOT_FOUND');
    return found;
  }
  #pairKey(thread: string, messageId: string, machine: string, agent: string) {
    [thread, messageId, machine, agent].forEach(generatedId);
    return `ask-recipient:${this.scope.space}:${this.scope.page}:${this.scope.deviceId}:${thread}:${messageId}:${machine}:${agent}`;
  }
  /** Called only for an authoritative adopted:false outcome, never an absent read. */
  async releaseRetry(view: {
    thread: string;
    messageIds: readonly string[];
    machine: string;
    agent: string;
    operationId: string;
  }) {
    requireValue(view.messageIds.length === 1);
    generatedId(view.operationId);
    const key = this.#pairKey(view.thread, view.messageIds[0], view.machine, view.agent);
    await navigator.locks.request(key, async () => {
      if ((await record<string>(key)) === view.operationId) await deleteRecord(key);
    });
  }
  async checkRetry(
    thread: string,
    messageId: string,
    machine: string,
    agent: string,
    retryOf: string | null,
  ): Promise<void> {
    const related = (await this.views()).filter(
      (view) =>
        view.intent.thread === thread &&
        view.intent.messageIds.includes(messageId) &&
        view.intent.machine === machine &&
        view.intent.agent === agent,
    );
    requireValue(related.every(provablyUnsent));
    const pending = await record<string>(this.#pairKey(thread, messageId, machine, agent));
    if (pending !== undefined) {
      generatedId(pending);
      // Absence in this tab's admitted view cannot prove that another tab was unsent.
      requireValue(
        related.some((view) => view.intent.operationId === pending && provablyUnsent(view)),
      );
    }
    if (retryOf !== null) {
      generatedId(retryOf);
      requireValue(
        related.some(
          (view) => view.intent.operationId === retryOf && view.intent.messageIds.length === 1,
        ),
      );
    }
  }
  async #write(value: AskRecord) {
    validateRecord(value);
    const { root, key } = recordKey(value);
    const old = this.options.readOwn()[this.scope.deviceId]?.[root][key];
    if (old !== undefined) {
      if (JSON.stringify(old) !== JSON.stringify(value)) throw new Error('INTENT_CONFLICT');
      return;
    }
    await this.options.publish(root, key, structuredClone(value) as unknown as JsonValue);
  }
  /** Must run under exclusive. Durable metadata and own append both precede
   * dispatch. An interrupted reservation never authorizes another send. */
  async adopt(
    signed: SignedAsk,
    labels: { agentName: string; deviceName: string },
    retryOf?: string | null,
  ): Promise<'created' | 'existing'> {
    if (retryOf !== undefined) {
      const intent = await verifyAsk(signed, this.#publicKey);
      requireValue(
        intent.messageIds.length === 1 &&
          intent.space === this.scope.space &&
          intent.page === this.scope.page &&
          intent.senderDevice === this.scope.deviceId,
      );
      const pairKey = this.#pairKey(
        intent.thread,
        intent.messageIds[0],
        intent.machine,
        intent.agent,
      );
      // The single shared UUID fences tabs whose admitted views still lag.
      // Storage read/write errors reject before any adoption or Remote effect.
      return navigator.locks.request(pairKey, async () => {
        requireValue(retryOf !== intent.operationId);
        await this.checkRetry(
          intent.thread,
          intent.messageIds[0],
          intent.machine,
          intent.agent,
          retryOf,
        );
        await record(pairKey, intent.operationId);
        return this.adopt(signed, labels);
      });
    }
    const value: AskRecord = {
      version: 1,
      kind: 'ask',
      signed,
      agentName: labels.agentName,
      deviceName: labels.deviceName,
    };
    validateRecord(value);
    const intent = await verifyAsk(signed, this.#publicKey);
    requireValue(
      intent.space === this.scope.space &&
        intent.page === this.scope.page &&
        intent.senderDevice === this.scope.deviceId,
    );
    const previous = (await this.views()).find((v) => v.intent.operationId === intent.operationId);
    if (previous) {
      if (
        previous.signed.input !== signed.input ||
        previous.signed.signature !== signed.signature ||
        previous.signed.finalBytes !== signed.finalBytes
      )
        throw new Error('INTENT_CONFLICT');
      return 'existing';
    }
    requireValue((await this.views()).length < 1000);
    const status = await storeAskDraft(signed);
    await this.#write(value);
    return status;
  }
  async state(
    id: string,
    state: LedgerState,
    requestId: string | null = null,
    reason: string | null = null,
  ) {
    const old = await this.view(id);
    if (old.revision !== '0') requireValue(canTransition(old.state, state));
    requireValue(old.requestId === null || requestId === old.requestId);
    if (
      old.revision !== '0' &&
      old.state === state &&
      old.requestId === requestId &&
      old.reason === reason
    )
      return old;
    await this.#write({
      version: 1,
      kind: 'ask-state',
      operationId: id,
      revision: (BigInt(old.revision) + 1n).toString(),
      state,
      requestId,
      reason,
    });
    return this.view(id);
  }
  async reply(id: string, requestId: string, body: string) {
    const old = await this.view(id);
    requireValue(old.state === 'accepted' && old.requestId === requestId);
    await this.#write({
      version: 1,
      kind: 'ask-reply',
      operationId: id,
      requestId,
      agentId: old.intent.agent,
      body,
    });
    return this.view(id);
  }
}
