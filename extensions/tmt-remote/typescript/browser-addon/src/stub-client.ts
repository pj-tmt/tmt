import type { RemoteClient, SendInput, SendState } from './remote-client.js';
export function createStubClient(): RemoteClient {
  const request = (operationId: string): SendState => ({
    operationId,
    state: 'accepted',
    requestId: `demo-${operationId}`,
  });
  return {
    async listAgents() {
      return [
        { id: 'demo-held', name: 'Demo · held → reply' },
        { id: 'demo-uncertain', name: 'Demo · uncertain → empty reply' },
        { id: 'demo-refused', name: 'Demo · refused' },
        { id: 'demo-unavailable', name: 'Demo · unavailable result' },
        { id: 'demo-unpaired', name: 'Demo · unpaired' },
      ];
    },
    async send(input: SendInput) {
      if (input.agentId === 'demo-unpaired')
        throw { code: 'unpaired', message: 'Pair this client first.' };
      if (input.agentId === 'demo-refused')
        return {
          operationId: input.operationId,
          state: 'refused',
          reason: 'Demo owner refused this request.',
        };
      return {
        operationId: input.operationId,
        state: input.agentId === 'demo-uncertain' ? 'uncertain' : 'held',
      };
    },
    async operation(operationId) {
      const { journal } = await import('./journal.js');
      const input = await journal.load();
      if (input?.operationId !== operationId)
        throw { code: 'unavailable', message: 'Demo operation is unavailable.' };
      if (input.agentId === 'demo-refused')
        return { operationId, state: 'refused', reason: 'Demo owner refused this request.' };
      if (input.agentId === 'demo-unpaired')
        throw { code: 'unpaired', message: 'Pair this client first.' };
      return request(operationId);
    },
    async result(requestId) {
      // Reading the UI journal simulates recovery without a second demo ledger.
      const { journal } = await import('./journal.js');
      const input = await journal.load();
      if (input?.agentId === 'demo-unavailable')
        return {
          requestId,
          state: 'unavailable',
          reason: 'Demo result is unavailable. Do not resend.',
        };
      return {
        requestId,
        state: 'replied',
        message:
          input?.agentId === 'demo-uncertain'
            ? ''
            : '<demo reply> Selection received. Nothing was sent to TMT.',
      };
    },
  };
}
