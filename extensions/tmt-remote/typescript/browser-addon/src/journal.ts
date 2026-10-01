import { isCapture } from './message.js';
import type { Capture } from './message.js';
import type { SendInput } from './remote-client.js';
export const JOURNAL_DATABASE = 'tmt-addon-shell';
export const JOURNAL_VERSION = 1;
export const JOURNAL_STORE = 'values';
export const INTENT_KEY = 'intent';
export const CAPTURE_KEY = 'capture';
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
    const request = indexedDB.open(JOURNAL_DATABASE, JOURNAL_VERSION);
    request.onupgradeneeded = () => request.result.createObjectStore(JOURNAL_STORE);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(new Error('Local draft storage is unavailable.'));
  });
  try {
    return await new Promise((resolve, reject) => {
      const transaction = database.transaction(JOURNAL_STORE, mode);
      const request = action(transaction.objectStore(JOURNAL_STORE));
      transaction.oncomplete = () => resolve(request.result);
      transaction.onabort = () => reject(new Error('Local draft storage could not be updated.'));
    });
  } finally {
    database.close();
  }
}
export const journal: Journal = {
  async load() {
    const value = await access('readonly', (s) => s.get(INTENT_KEY));
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
    await access('readwrite', (s) => s.put(input, INTENT_KEY));
  },
  async clear() {
    await access('readwrite', (s) => s.delete(INTENT_KEY));
  },
};
export async function saveCapture(capture: Capture): Promise<void> {
  if (!isCapture(capture)) throw new Error('Selection source URL is invalid.');
  await access('readwrite', (s) => s.put(capture, CAPTURE_KEY));
}
export async function loadCapture(): Promise<Capture | undefined> {
  const capture = await access('readwrite', (s) => {
    const request = s.get(CAPTURE_KEY);
    request.onsuccess = () => s.delete(CAPTURE_KEY);
    return request;
  });
  if (capture === undefined) return undefined;
  if (!isCapture(capture)) throw new Error('Stored selection is invalid.');
  return capture;
}
