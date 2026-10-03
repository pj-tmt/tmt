import { coreId, decodeText, requireValue } from '@tmt/colab-client';
import { FrozenAsk, type SignedAsk } from './ask-intent.js';
import type { RemoteClient, SendState } from './ask-remote.js';
import { record } from './storage.js';

export type DraftAdoption = (intent: SignedAsk) => Promise<'created' | 'existing'>;
export type AskState =
  | SendState
  | {
      state: 'preview' | 'preparing' | 'failed';
      operationId: string;
    };

/** Immutable local draft, not the native bridge ledger or encrypted own stream.
 * Web Locks serialize same-operation adoption across tabs; a stored draft never
 * authorizes a send on reload. Transaction completion precedes any port call. */
export const storeAskDraft: DraftAdoption = async (intent) => {
  const key = `ask:${intent.senderDevice}:${intent.operationId}`;
  return await navigator.locks.request(key, async () => {
    const previous = await record<SignedAsk>(key);
    if (previous) {
      if (
        previous.input !== intent.input ||
        previous.signature !== intent.signature ||
        previous.finalBytes !== intent.finalBytes
      )
        throw new Error('INTENT_CONFLICT');
      return 'existing';
    }
    await record(key, intent);
    return 'created';
  });
};

/** One explicit attempt. Observing/reconstructing this object has no effect.
 * There is deliberately no retry: remote child-cleanup evidence is not available. */
export class AskAttempt {
  #pending?: Promise<AskState>;
  #state: AskState;
  constructor(
    readonly preview: FrozenAsk,
    private key: CryptoKey,
    private adopt: DraftAdoption = storeAskDraft,
    private remote?: RemoteClient,
  ) {
    this.#state = { state: 'preview', operationId: preview.view.operationId };
    Object.freeze(this);
  }
  get available(): boolean {
    return this.remote !== undefined;
  }
  get state(): Readonly<AskState> {
    return Object.freeze({ ...this.#state });
  }
  send(): Promise<AskState> {
    if (!this.available) return Promise.reject(new Error('Remote operations are unavailable.'));
    if (this.#pending) return this.#pending;
    this.#state = { state: 'preparing', operationId: this.preview.view.operationId };
    this.#pending = this.#send();
    return this.#pending;
  }
  async #send(): Promise<AskState> {
    const operationId = this.preview.view.operationId;
    let started = false;
    try {
      const intent = await this.preview.signed(this.key);
      if ((await this.adopt(intent)) === 'existing') {
        this.#state = { state: 'uncertain', operationId };
      } else {
        // Storage/signing may have outlasted validity; fail before the port call.
        requireValue(Date.now() < this.preview.expiresAt);
        started = true;
        const reply = await this.remote!.send({
          operationId,
          agentId: this.preview.view.agent,
          message: decodeText(this.preview.finalBytes()),
        });
        requireValue(
          reply.operationId === operationId &&
            ['held', 'accepted', 'uncertain', 'refused', 'cancelled'].includes(reply.state),
        );
        if (reply.state === 'accepted') {
          requireValue(typeof reply.requestId === 'string' && reply.requestId.startsWith('req_'));
          coreId(reply.requestId.slice(4));
        }
        this.#state = { ...reply };
      }
    } catch {
      this.#state = { state: started ? 'uncertain' : 'failed', operationId };
    }
    return this.state;
  }
}
