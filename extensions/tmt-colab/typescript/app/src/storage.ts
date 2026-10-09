/** Colab-owned IndexedDB records. A write resolves only after durable transaction
 * completion, never just the individual put success. */
export async function record<T>(key: string): Promise<T | undefined>;
export async function record<T>(key: string, value: T): Promise<void>;
export async function record<T>(key: string, ...values: [T] | []): Promise<T | undefined | void> {
  return accessRecord(key, values.length ? 'write' : 'read', values[0]);
}
/** Deletion, like a write, resolves only after the transaction commits. */
export async function deleteRecord(key: string): Promise<void> {
  await accessRecord(key, 'delete');
}
async function accessRecord<T>(
  key: string,
  mode: 'read' | 'write' | 'delete',
  value?: T,
): Promise<T | undefined | void> {
  const database = await new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open('tmt-colab', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('keys');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error('Device storage is blocked'));
  });
  try {
    return await new Promise<T | undefined | void>((resolve, reject) => {
      const transaction = database.transaction('keys', mode === 'read' ? 'readonly' : 'readwrite');
      const store = transaction.objectStore('keys');
      const pending =
        mode === 'read'
          ? store.get(key)
          : mode === 'write'
            ? store.put(value, key)
            : store.delete(key);
      transaction.oncomplete = () =>
        resolve(mode === 'read' ? (pending.result as T | undefined) : undefined);
      transaction.onabort = () => reject(transaction.error ?? new Error('Device storage failed'));
      transaction.onerror = () => reject(transaction.error);
    });
  } finally {
    database.close();
  }
}
