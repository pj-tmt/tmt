import { expect, it, vi } from 'vitest';
import { Intent } from './intent.js';
import type { SendInput, RemoteClient } from './remote-client.js';
function fixture() {
  let saved: SendInput | undefined;
  const journal = {
    load: async () => saved,
    save: async (input: SendInput) => {
      saved = structuredClone(input);
    },
    clear: async () => {
      saved = undefined;
    },
  };
  const client: RemoteClient = {
    listAgents: vi.fn(async () => []),
    send: vi.fn<RemoteClient['send']>(async (input) => ({
      operationId: input.operationId,
      state: 'held',
    })),
    operation: vi.fn<RemoteClient['operation']>(async (operationId) => ({
      operationId,
      state: 'accepted',
      requestId: 'request',
    })),
    result: vi.fn<RemoteClient['result']>(async (requestId) => ({
      requestId,
      state: 'replied',
      message: '',
    })),
  };
  return { journal, client, intent: new Intent(client, journal) };
}
const input = { operationId: 'fixed', agentId: 'uuid', message: 'exact\r\n\u202Ebytes' };
it('persists before effects and freezes exact same ID/bytes across explicit retry and restart', async () => {
  const { intent, client, journal } = fixture();
  vi.mocked(client.send).mockImplementation(async (value) => {
    expect(await journal.load()).toEqual(value);
    throw new Error('response lost');
  });
  const mutable = { ...input };
  await intent.send(mutable);
  mutable.message = 'changed';
  expect(intent.state?.state).toBe('uncertain');
  await intent.send({ ...input, operationId: 'other', message: 'other' });
  expect(client.send).toHaveBeenNthCalledWith(2, input);
  const restored = new Intent(client, journal);
  await restored.restore();
  expect(restored.input).toEqual(input);
  expect(client.send).toHaveBeenCalledTimes(2);
  await restored.recover();
  expect(client.operation).toHaveBeenCalledWith('fixed');
  expect(client.send).toHaveBeenCalledTimes(2);
  expect(restored.result).toEqual({ requestId: 'request', state: 'replied', message: '' });
});
it('never reads a result while held and does not retry unavailable results', async () => {
  const { intent, client } = fixture();
  await intent.send(input);
  vi.mocked(client.operation).mockResolvedValue({ operationId: 'fixed', state: 'held' });
  await intent.recover();
  expect(client.result).not.toHaveBeenCalled();
  vi.mocked(client.operation).mockResolvedValue({
    operationId: 'fixed',
    state: 'accepted',
    requestId: 'request',
  });
  vi.mocked(client.result).mockResolvedValue({ requestId: 'request', state: 'unavailable' });
  await intent.recover();
  expect(intent.result?.state).toBe('unavailable');
  expect(client.send).toHaveBeenCalledTimes(1);
});
it('preserves refusal and rejects mismatched operation/request observations', async () => {
  const { intent, client } = fixture();
  vi.mocked(client.send).mockResolvedValue({
    operationId: 'fixed',
    state: 'refused',
    reason: 'denied',
  });
  await intent.send(input);
  expect(intent.state?.state).toBe('refused');
  vi.mocked(client.operation).mockResolvedValue({ operationId: 'other', state: 'held' });
  await expect(intent.recover()).rejects.toThrow('different operation');
  vi.mocked(client.operation).mockResolvedValue({
    operationId: 'fixed',
    state: 'accepted',
    requestId: 'request',
  });
  vi.mocked(client.result).mockResolvedValue({ requestId: 'other', state: 'pending' });
  await expect(intent.recover()).rejects.toThrow('different request');
});
it('storage failure prevents sending and explicit clear does not cancel work', async () => {
  const { intent, client, journal } = fixture();
  journal.save = async () => {
    throw new Error('quota');
  };
  await expect(intent.send(input)).rejects.toThrow('quota');
  expect(client.send).not.toHaveBeenCalled();
  await intent.clear();
  expect(intent.input).toBeUndefined();
});
