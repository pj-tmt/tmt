/** Private origin-local opener. Public registry APIs never expose the device record. */
export function request<T>(open: () => IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    const pending = open();
    pending.onsuccess = () => resolve(pending.result);
    pending.onerror = () => reject(pending.error ?? new Error('Device storage failed.'));
  });
}
export async function deviceObjects(mode: IDBTransactionMode): Promise<IDBObjectStore> {
  const opening = indexedDB.open('tmt-remote', 1);
  opening.onupgradeneeded = () => opening.result.createObjectStore('device');
  const database = await request(() => opening);
  return database.transaction('device', mode).objectStore('device');
}
const REGISTRY = 'remotes-v1';
export async function readRegistry(): Promise<unknown> {
  const objects = await deviceObjects('readonly');
  try {
    return await request(() => objects.get(REGISTRY));
  } finally {
    objects.transaction.db.close();
  }
}
/** Compare and replace one public key atomically; no retry, clear or device access. */
export async function writeRegistry(expected: unknown, text: string): Promise<void> {
  const objects = await deviceObjects('readwrite');
  const transaction = objects.transaction;
  await new Promise<void>((resolve, reject) => {
    let conflict = false;
    transaction.oncomplete = () => resolve();
    transaction.onabort = transaction.onerror = () =>
      reject(new Error(conflict ? 'Registry changed' : 'Registry unavailable'));
    const read = objects.get(REGISTRY);
    read.onsuccess = () => {
      if (read.result !== expected) {
        conflict = true;
        transaction.abort();
      } else objects.put(text, REGISTRY);
    };
  }).finally(() => transaction.db.close());
}
