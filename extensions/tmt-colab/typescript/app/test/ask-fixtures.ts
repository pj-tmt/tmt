import type { AdmittedSelection, AskDestination } from '../src/ask-intent.js';
import type { RemoteClient, SendInput, SendState } from '../src/ask-remote.js';
export const id = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
export function selection(): AdmittedSelection {
  return {
    space: 'a'.repeat(32),
    page: id(1),
    thread: id(2),
    messageIds: [id(3)],
    senderDevice: id(4),
    quote: '<script>untrusted()</script>\r\n😀\0\u202e',
    comment: 'Keep ! and café exact',
    title: 'Shared page',
    url: 'https://example.test/page#secret',
  };
}
export function destination(): AskDestination {
  return {
    machine: id(5),
    machineName: 'My machine',
    online: 'online',
    agent: id(6),
    agentName: 'Deterministic agent',
    delivery: 'channel',
    grantExpiresAt: null,
    deviceName: 'Fixture browser',
    grantRevision: '1',
    mode: 'direct',
  };
}
/** Test-only deterministic operation port. It supplies no production authority. */
export class RemoteDouble implements RemoteClient {
  sends: SendInput[] = [];
  reads: string[] = [];
  mode: 'accepted' | 'held' | 'throw' | 'wrong_id' = 'accepted';
  async context() {
    return {
      machineId: id(5),
      deviceId: id(4),
      grantRevision: '1',
      deviceName: 'Fixture browser',
      expiresAtMs: null,
      mode: null,
    };
  }
  async listAgents() {
    return [{ id: id(6), name: 'Deterministic agent', delivery: 'channel' as const }];
  }
  async send(input: SendInput): Promise<SendState> {
    this.sends.push({ ...input });
    if (this.mode === 'throw') throw new Error('Lost transport response with secret diagnostics');
    if (this.mode === 'held') return { state: 'held', operationId: input.operationId };
    return {
      state: 'accepted',
      operationId: this.mode === 'wrong_id' ? id(99) : input.operationId,
      requestId: `req_${id(8)}`,
    };
  }
  async operation(operationId: string): Promise<SendState> {
    this.reads.push(operationId);
    return { state: 'uncertain', operationId };
  }
  async check() {
    return { capture: 'read-only' };
  }
  async result(requestId: string) {
    return { state: 'replied' as const, requestId, message: '' };
  }
}
