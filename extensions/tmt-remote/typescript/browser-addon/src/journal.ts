import { isCapture } from './message.js';
import type { Capture } from './message.js';
import type { SendInput } from './remote-client.js';
// Only one frozen local intent, not a conversation store or a cancellation ledger.
export interface Journal {
  load(): Promise<SendInput | undefined>;
  save(input: SendInput): Promise<void>;
  clear(): Promise<void>;
}
async function access(
  mode: IDBTransactionMode,
  action: (store: IDBObjectStore) => IDBRequest,
): Promise<unknown> {
  const database = await new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open('tmt-addon-shell', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('values');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(new Error('Local draft storage is unavailable.'));
  });
  try {
    return await new Promise((resolve, reject) => {
      const transaction = database.transaction('values', mode);
      const request = action(transaction.objectStore('values'));
      transaction.oncomplete = () => resolve(request.result);
      transaction.onabort = () => reject(new Error('Local draft storage could not be updated.'));
    });
  } finally {
    database.close();
  }
}
export const journal: Journal = {
  async load() {
    const value = await access('readonly', (s) => s.get('intent'));
    if (value === undefined) return undefined;
    if (!value || typeof value !== 'object') throw new Error('Stored intent is invalid.');
    const v = value as Record<string, unknown>;
    if (
      typeof v.operationId !== 'string' ||
      typeof v.agentId !== 'string' ||
      typeof v.message !== 'string' ||
      !v.operationId ||
      !v.agentId ||
      new TextEncoder().encode(v.message).length > 65536
    )
      throw new Error('Stored intent is invalid.');
    return { operationId: v.operationId, agentId: v.agentId, message: v.message };
  },
  async save(input) {
    await access('readwrite', (s) => s.put(input, 'intent'));
  },
  async clear() {
    await access('readwrite', (s) => s.delete('intent'));
  },
};
export async function saveCapture(capture: Capture): Promise<void> {
  if (!isCapture(capture)) throw new Error('Selection source URL is invalid.');
  await access('readwrite', (s) => s.put(capture, 'capture'));
}
export async function loadCapture(): Promise<Capture | undefined> {
  const capture = await access('readwrite', (s) => {
    const request = s.get('capture');
    request.onsuccess = () => s.delete('capture');
    return request;
  });
  if (capture === undefined) return undefined;
  if (!isCapture(capture)) throw new Error('Stored selection is invalid.');
  return capture;
}
