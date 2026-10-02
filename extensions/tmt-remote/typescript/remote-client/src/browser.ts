import wordlist from '../../../rust/tmt-remote/assets/bip39-english.txt?raw';
import { base64url, fingerprintIndexes } from './canonical-bytes.js';
import {
  DeviceKey,
  certify,
  openSession,
  pair,
  parseLink,
  type ExtCertificate,
  type Paired,
  type Session,
} from './device.js';

/**
 * Browser entry of the device SDK, served by the door as `/sdk/remote-v1.js`.
 * On the pairing page it runs the ceremony; mounted extension pages import it
 * to reopen the door session and certify their own extension keys. The device
 * key lives in this origin's IndexedDB as an opaque, non-extractable CryptoKey.
 */

const WORDS = wordlist.split('\n').slice(0, 2048);
const DATABASE = 'tmt-remote';
const STORE = 'device';
const RECORD = 'device';

interface Stored {
  handle: CryptoKey;
  publicKey: Uint8Array;
  paired: Paired;
  certificates: ExtCertificate[];
}

function request<T>(open: () => IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    const pending = open();
    pending.onsuccess = () => resolve(pending.result);
    pending.onerror = () => reject(pending.error ?? new Error('Device storage failed.'));
  });
}
async function store(mode: IDBTransactionMode): Promise<IDBObjectStore> {
  const opening = indexedDB.open(DATABASE, 1);
  opening.onupgradeneeded = () => opening.result.createObjectStore(STORE);
  const database = await request(() => opening);
  return database.transaction(STORE, mode).objectStore(STORE);
}
async function load(): Promise<Stored | undefined> {
  const objects = await store('readonly');
  return (await request(() => objects.get(RECORD))) as Stored | undefined;
}
async function save(record: Stored): Promise<void> {
  const objects = await store('readwrite');
  await request(() => objects.put(record, RECORD));
}

/** The door's answer for the calling page: this run and the page's own mount. */
interface Door {
  machineId: string;
  windowId: string;
  address: string;
  extension: string | null;
  mount: string | null;
}
async function door(): Promise<Door> {
  const response = await fetch('/sdk/mount', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path: location.pathname }),
  });
  if (response.status !== 200) throw new Error('The door did not answer.');
  return (await response.json()) as Door;
}
async function paired(): Promise<{ record: Stored; key: DeviceKey }> {
  const record = await load();
  if (!record) throw new Error('This browser is not paired.');
  return { record, key: await DeviceKey.fromHandle(record.handle, record.publicKey) };
}

/** Reopen this browser's door session for the running remote; no owner step. */
export async function reopenSession(): Promise<Session> {
  const { record, key } = await paired();
  const current = await door();
  if (current.machineId !== record.paired.machineId)
    throw new Error('Paired with another machine.');
  return openSession(record.paired, key, current.windowId);
}

/**
 * Certify a key of the calling page's own extension. The extension comes from
 * the door's mount mapping for this page, never from the caller; one
 * certificate is kept per (extension, purpose, key).
 */
export async function certifyKey(
  purpose: 'sign' | 'enc',
  publicKey: Uint8Array,
): Promise<ExtCertificate> {
  const { record, key } = await paired();
  const { extension } = await door();
  if (extension === null) throw new Error('Only a mounted extension page can certify keys.');
  const encoded = base64url(publicKey);
  const existing = record.certificates.find(
    (c) => c.extension === extension && c.purpose === purpose && c.publicKey === encoded,
  );
  if (existing) return existing;
  const certificate = await certify(key, { extension, purpose, publicKey });
  await save({ ...record, certificates: [...record.certificates, certificate] });
  return certificate;
}

function element(id: string): HTMLElement {
  const found = document.getElementById(id);
  if (!found) throw new Error(`The pairing page lacks #${id}.`);
  return found;
}
/** The pairing page. The fragment holding the code is removed before anything else runs. */
function pairingPage(): void {
  const link = location.href;
  history.replaceState(null, '', location.pathname);
  const status = element('status');
  let parsed: ReturnType<typeof parseLink>;
  try {
    parsed = parseLink(link);
  } catch {
    status.textContent =
      'This pairing link is incomplete. Copy the whole link from tmt remote pair.';
    return;
  }
  const form = element('pair') as HTMLFormElement;
  form.addEventListener('submit', (event) => {
    event.preventDefault();
    form.hidden = true;
    const name = (element('name') as HTMLInputElement).value.trim();
    void ceremony(parsed, name, status).catch(() => {
      status.textContent = 'Pairing did not complete. Run tmt remote pair again for a new link.';
    });
  });
}
async function ceremony(
  { descriptor, code }: ReturnType<typeof parseLink>,
  name: string,
  status: HTMLElement,
): Promise<void> {
  const key = await DeviceKey.generate();
  const indexes = await fingerprintIndexes(key.publicKey());
  const words = element('words');
  words.textContent = `Words: ${indexes.map((i) => WORDS[i]).join(' ')}`;
  words.hidden = false;
  status.textContent = 'Compare these words with the terminal, then confirm there.';
  const result = await pair({
    descriptor,
    code,
    key,
    kind: 'browser',
    origin: location.origin,
    name,
  });
  await save({
    handle: key.handle(),
    publicKey: key.publicKey(),
    paired: result,
    certificates: [],
  });
  await openSession(result, key, descriptor.windowId);
  status.textContent = 'This browser is paired. You can close this page.';
}

if (document.documentElement.dataset.tmtPage === 'pair') pairingPage();
