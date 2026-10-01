import type { Worker } from '@playwright/test';
import { JOURNAL_DATABASE, JOURNAL_VERSION, JOURNAL_STORE } from '../src/journal.js';

type Operation =
  | { kind: 'reset' }
  | { kind: 'get'; key: string }
  | { kind: 'put'; key: string; value: unknown };

// Evaluation runs in the extension origin, while schema ownership stays with the journal.
export async function accessJournal(
  target: Pick<Worker, 'evaluate'>,
  operation: Operation,
): Promise<unknown> {
  return target.evaluate(
    async ({ databaseName, version, storeName, operation }) => {
      if (operation.kind === 'reset') {
        await new Promise<void>((resolve, reject) => {
          const request = indexedDB.deleteDatabase(databaseName);
          request.onsuccess = () => resolve();
          request.onerror = () =>
            reject(request.error ?? new Error('Fixture database reset failed'));
          request.onblocked = () => reject(new Error('Fixture database reset blocked'));
        });
        return;
      }
      const database = await new Promise<IDBDatabase>((resolve, reject) => {
        const request = indexedDB.open(databaseName, version);
        let blocked = false;
        request.onupgradeneeded = () => request.result.createObjectStore(storeName);
        request.onerror = () => reject(request.error ?? new Error('Fixture database open failed'));
        request.onblocked = () => {
          blocked = true;
          reject(new Error('Fixture database open blocked'));
        };
        request.onsuccess = () => {
          if (blocked) request.result.close();
          else resolve(request.result);
        };
      });
      try {
        return await new Promise<unknown>((resolve, reject) => {
          const transaction = database.transaction(
            storeName,
            operation.kind === 'put' ? 'readwrite' : 'readonly',
          );
          transaction.onabort = () =>
            reject(transaction.error ?? new Error('Fixture transaction aborted'));
          transaction.onerror = () =>
            reject(transaction.error ?? new Error('Fixture transaction failed'));
          const store = transaction.objectStore(storeName);
          const request =
            operation.kind === 'put'
              ? store.put(operation.value, operation.key)
              : store.get(operation.key);
          request.onerror = () => reject(request.error ?? new Error('Fixture request failed'));
          transaction.oncomplete = () => resolve(request.result);
        });
      } finally {
        database.close();
      }
    },
    {
      databaseName: JOURNAL_DATABASE,
      version: JOURNAL_VERSION,
      storeName: JOURNAL_STORE,
      operation,
    },
  );
}
