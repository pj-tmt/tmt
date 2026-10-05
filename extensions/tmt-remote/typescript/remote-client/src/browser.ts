import { channelFor } from './session-channel.js';
import { RefusalError } from './operations.js';
export { operations, ClientError, RefusalError } from './operations.js';
export { management } from './management.js';
export type {
  RemoteManagement,
  ManagementOutcome,
  RemoteSettings,
  DevicePage,
} from './management.js';
export type {
  ClientErrorCode,
  RemoteRefusalCode,
  RemoteOperations,
  RemoteAgent,
  SendInput,
  SendState,
  ResultState,
} from './operations.js';
import wordlist from '../../../rust/tmt-remote/assets/bip39-english.txt?raw';
import { fingerprintIndexes } from './canonical-bytes.js';
import {
  DeviceKey,
  certify,
  openSession,
  pair,
  parseLink,
  resolveLink,
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
export async function reopenSession(previous?: Session): Promise<Session> {
  const { record, key } = await paired();
  if (previous) {
    const old = channelFor(previous);
    if (!old) throw new TypeError('Use a verified previous Session.');
    if (
      record.paired.clientId !== old.paired.clientId ||
      record.paired.machineId !== old.paired.machineId ||
      record.paired.origin !== old.paired.origin ||
      !(record.paired.machinePublicKey instanceof Uint8Array) ||
      record.paired.machinePublicKey.length !== 32 ||
      old.paired.machinePublicKey.length !== 32 ||
      record.paired.machinePublicKey.some(
        (byte, index) => byte !== old.paired.machinePublicKey[index],
      ) ||
      record.publicKey.some((byte, index) => byte !== old.key.publicKey()[index])
    ) {
      throw new RefusalError('REMOTE_SESSION_ENDED');
    }
  }
  const current = await door();
  validateDoor(current, record);
  let refused = false;
  try {
    return await openSession(
      { ...record.paired, address: current.address },
      key,
      current.windowId,
      async (url, init) => {
        const response = await fetch(url, init);
        refused = response.status === 404;
        return response;
      },
    );
  } catch (error) {
    if (!refused || !previous) throw error;
    // A restart or stale descriptor is uncertainty, not current access refusal.
    // This is another descriptor read, never another admission or mutation.
    const latest = await door();
    validateDoor(latest, record);
    if (latest.address !== current.address || latest.windowId !== current.windowId) {
      throw new Error('The door descriptor changed during admission.');
    }
    throw new RefusalError('REMOTE_SESSION_ENDED');
  }
}
function validateDoor(current: Door, record: Stored): void {
  if (current.machineId !== record.paired.machineId)
    throw new Error('Paired with another machine.');
  if (
    typeof current.windowId !== 'string' ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(
      current.windowId,
    ) ||
    typeof current.address !== 'string' ||
    !current.address.startsWith(`${location.origin}/r/`) ||
    !/^[a-z2-7]{16}$/.test(current.address.slice(`${location.origin}/r/`.length))
  ) {
    throw new Error('The door advertised an invalid descriptor.');
  }
}

/**
 * Certify a key of the calling page's own extension. The extension comes from
 * the door's mount mapping for this page, never from the caller. Each call
 * signs a new certificate with the current issuedAtMs; verifiers own freshness.
 */
export async function certifyKey(
  purpose: 'sign' | 'enc',
  publicKey: Uint8Array,
): Promise<ExtCertificate> {
  const { key } = await paired();
  const { extension } = await door();
  if (extension === null) throw new Error('Only a mounted extension page can certify keys.');
  return certify(key, { extension, purpose, publicKey });
}

function element(id: string): HTMLElement {
  const found = document.getElementById(id);
  if (!found) throw new Error(`The pairing page lacks #${id}.`);
  return found;
}
/** Present the ceremony state without coloring its instruction text. */
function showState(
  status: HTMLElement,
  state: 'waiting' | 'paired' | 'blocked',
  text: string,
): void {
  status.dataset.state = state;
  status.textContent = text;
  element('mark').textContent = { waiting: '◆', paired: '✓', blocked: '✗' }[state];
}
/** Called by the fragment-erasing bootstrap after the page DOM is ready. */
export async function pairingPage(link: string): Promise<void> {
  if (document.readyState === 'loading') {
    await new Promise<void>((resolve) =>
      document.addEventListener('DOMContentLoaded', () => resolve(), { once: true }),
    );
  }
  const status = element('status');
  const form = element('pair') as HTMLFormElement;
  const button = form.querySelector('button')!;
  button.disabled = true;
  let parsed: ReturnType<typeof parseLink>;
  try {
    parsed = await resolveLink(link);
  } catch {
    form.hidden = true;
    showState(
      status,
      'blocked',
      'This pairing link is unavailable. Run tmt remote pair again for a new link.',
    );
    return;
  }
  button.disabled = false;
  form.addEventListener('submit', (event) => {
    event.preventDefault();
    form.hidden = true;
    const name = (element('name') as HTMLInputElement).value.trim();
    void ceremony(parsed, name, status).catch(() => {
      showState(
        status,
        'blocked',
        'Pairing did not complete. Run tmt remote pair again for a new link.',
      );
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
  words.textContent = indexes.map((i) => WORDS[i]).join(' ');
  element('comparison').hidden = false;
  showState(status, 'waiting', 'Compare these words with the terminal, then confirm there.');
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
  });
  await openSession(result, key, descriptor.windowId);
  showState(status, 'paired', 'This browser is paired. You can close this page.');
}

/** Associate a mounted WebSocket with this tab's verified session, not the shared cookie's tab.
 * Use only to construct a transport; never navigate to or log this URL.
 */
export function transportUrl(session: import('./device.js').Session, value: string | URL): string {
  const channel = channelFor(session);
  if (!channel || channel.ended) throw new RefusalError('REMOTE_SESSION_ENDED');
  const address = new URL(channel.paired.address);
  const url = new URL(value, channel.paired.address);
  if (
    url.protocol !== (address.protocol === 'https:' ? 'wss:' : 'ws:') ||
    !['http:', 'https:'].includes(address.protocol) ||
    url.host !== address.host ||
    !url.pathname.startsWith(`${address.pathname}/x/`) ||
    url.search ||
    url.hash ||
    url.username ||
    url.password
  ) {
    throw new TypeError('Use a mounted WebSocket URL on this door without query or fragment.');
  }
  url.searchParams.set('tmt-session', channel.sessionId);
  return url.href;
}
