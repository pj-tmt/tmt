/** Colab-owned IndexedDB records. A write resolves only after durable transaction
 * completion, never just the individual put success. */
export async function record<T>(key: string): Promise<T | undefined>;
export async function record<T>(key: string, value: T): Promise<void>;
export async function record<T>(key: string, ...values: [T] | []): Promise<T | undefined | void> {
  const database = await new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open('tmt-colab', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('keys');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error('Device storage is blocked'));
  });
  try {
    return await new Promise<T | undefined | void>((resolve, reject) => {
      const transaction = database.transaction('keys', values.length ? 'readwrite' : 'readonly');
      const store = transaction.objectStore('keys');
      const pending = values.length ? store.put(values[0], key) : store.get(key);
      transaction.oncomplete = () =>
        resolve(values.length ? undefined : (pending.result as T | undefined));
      transaction.onabort = () => reject(transaction.error ?? new Error('Device storage failed'));
      transaction.onerror = () => reject(transaction.error);
    });
  } finally {
    database.close();
  }
}
