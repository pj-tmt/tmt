import type { Journal } from './journal.js';
import type { RemoteClient, SendInput, SendState, ResultState } from './remote-client.js';
export class Intent {
  input?: Readonly<SendInput>;
  state?: SendState;
  result?: ResultState;
  errorCode?: string;
  constructor(
    private client: RemoteClient,
    private journal: Journal,
  ) {}
  async restore(): Promise<void> {
    const input = await this.journal.load();
    if (input) {
      this.input = Object.freeze(input);
      this.state = { operationId: input.operationId, state: 'uncertain' };
    }
  }
  async send(input?: SendInput): Promise<void> {
    if (!this.input) {
      if (!input) throw new Error('Review a message first.');
      // Persist exact bytes before any call that might have an effect.
      await this.journal.save(input);
      this.input = Object.freeze({ ...input });
    }
    this.result = undefined;
    this.errorCode = undefined;
    try {
      this.adopt(await this.client.send({ ...this.input }));
    } catch (error) {
      if (error && typeof error === 'object' && 'code' in error)
        this.errorCode = String(error.code);
      // Any unknown rejection may follow submission; never pretend it was unsent.
      this.state = { operationId: this.input.operationId, state: 'uncertain' };
    }
  }
  private adopt(state: SendState): void {
    if (state.operationId !== this.input?.operationId)
      throw new Error('The client returned a different operation.');
    this.state = state;
  }
  async recover(): Promise<void> {
    if (!this.input) return;
    this.errorCode = undefined;
    this.result = undefined;
    this.adopt(await this.client.operation(this.input.operationId));
    if (this.state?.state === 'accepted') {
      const result = await this.client.result(this.state.requestId);
      if (result.requestId !== this.state.requestId)
        throw new Error('The client returned a different request.');
      this.result = result;
    }
  }
  async clear(): Promise<void> {
    await this.journal.clear();
    this.errorCode = undefined;
    this.input = undefined;
    this.state = undefined;
    this.result = undefined;
  }
}
