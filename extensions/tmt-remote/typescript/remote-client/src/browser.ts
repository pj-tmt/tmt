import { channelFor } from './session-channel.js';
import { RefusalError } from './operations.js';
export { operations, ClientError, RefusalError } from './operations.js';
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
export async function reopenSession(): Promise<Session> {
  const { record, key } = await paired();
  const current = await door();
  if (current.machineId !== record.paired.machineId)
    throw new Error('Paired with another machine.');
  // A schema migration may replace the non-credential route path while the origin/grant survive.
  if (
    !current.address.startsWith(`${location.origin}/r/`) ||
    !/^[a-z2-7]{16}$/.test(current.address.slice(`${location.origin}/r/`.length))
  ) {
    throw new Error('The door advertised an invalid address.');
  }
  return openSession({ ...record.paired, address: current.address }, key, current.windowId);
}

type EntryPairing =
  | { state: 'missing' | 'unavailable' }
  | { state: 'saved'; record: Stored; key: DeviceKey };

/** Local evidence only: never read a descriptor or attempt admission to paint the entry. */
async function entryPairing(): Promise<EntryPairing> {
  try {
    const record = await load();
    if (record === undefined) return { state: 'missing' };
    if (typeof record !== 'object' || record === null || Array.isArray(record))
      return { state: 'unavailable' };
    const pin = record.paired?.machinePublicKey;
    if (
      typeof record.paired !== 'object' ||
      record.paired === null ||
      Array.isArray(record.paired) ||
      record.paired.kind !== 'browser' ||
      record.paired.origin !== location.origin ||
      !entryUuid(record.paired.machineId) ||
      !entryUuid(record.paired.clientId) ||
      !entryAddress(record.paired.address) ||
      !Number.isSafeInteger(record.paired.grantRevision) ||
      record.paired.grantRevision < 1 ||
      !(record.publicKey instanceof Uint8Array) ||
      record.publicKey.length !== 32 ||
      !(pin instanceof Uint8Array) ||
      pin.length !== 32
    )
      return { state: 'unavailable' };
    return {
      state: 'saved',
      record,
      key: await DeviceKey.fromHandle(record.handle, record.publicKey),
    };
  } catch {
    return { state: 'unavailable' };
  }
}

/** Landing-page projection of existing pairing and signed Session owners; no new authority. */
export async function landingPage(): Promise<void> {
  if (document.readyState === 'loading') {
    await new Promise<void>((resolve) =>
      document.addEventListener('DOMContentLoaded', () => resolve(), { once: true }),
    );
  }
  const status = element('status');
  const pairing = element('pairing-status');
  const access = element('access-status');
  const button = element('check') as HTMLButtonElement;
  let attempt = 0;
  window.addEventListener(
    'pagehide',
    () => {
      attempt++;
    },
    { once: true },
  );
  const renderPairing = (value: EntryPairing): void => {
    pairing.textContent = {
      missing: 'No saved pairing',
      saved: 'Pairing saved in this browser',
      unavailable: 'Pairing status unknown',
    }[value.state];
    button.disabled = value.state !== 'saved';
    if (value.state === 'missing') {
      showState(status, 'waiting', 'Follow the terminal-confirmed pairing steps below.');
    } else if (value.state === 'unavailable') {
      showState(
        status,
        'blocked',
        'The saved pairing could not be read or validated. Access is unconfirmed; no connection was attempted.',
      );
    }
    element('mark').textContent = '○';
  };
  const initial = attempt;
  const local = await entryPairing();
  if (attempt !== initial || !button.isConnected) return;
  renderPairing(local);
  button.addEventListener('click', async () => {
    if (button.disabled) return;
    button.disabled = true;
    const currentAttempt = ++attempt;
    const currentPage = (): boolean => attempt === currentAttempt && button.isConnected;
    access.textContent = 'Connecting…';
    showState(status, 'waiting', 'Connecting using this browser’s saved pairing. No work is sent.');
    {
      const local = await entryPairing();
      if (!currentPage()) return;
      if (local.state !== 'saved') {
        renderPairing(local);
        access.textContent = 'Could not verify access';
        return;
      }
      let refused = false;
      try {
        const current = await door();
        if (!currentPage()) return;
        validateEntryDoor(current);
        if (current.machineId !== local.record.paired.machineId) {
          pairing.textContent = 'Saved pairing does not match this Remote';
          access.textContent = 'Not checked';
          showState(
            status,
            'blocked',
            'This door names a different machine from the saved pairing. Your saved pairing has not been changed. Confirm the machine in its local terminal.',
          );
          return;
        }
        try {
          await openSession(
            { ...local.record.paired, address: current.address },
            local.key,
            current.windowId,
            async (url, init) => {
              if (!currentPage()) throw new Error('Remote page attempt ended.');
              const response = await fetch(url, init);
              refused = response.status === 404;
              return response;
            },
          );
        } catch (error) {
          if (!currentPage()) return;
          if (!refused) throw error;
          const latest = await door();
          if (!currentPage()) return;
          validateEntryDoor(latest);
          if (
            latest.machineId !== current.machineId ||
            latest.address !== current.address ||
            latest.windowId !== current.windowId
          )
            throw error;
          access.textContent = 'Could not verify access';
          showState(
            status,
            'blocked',
            'The current door did not admit this saved pairing. This is not a verified refusal reason. Your saved pairing has not been changed. Inspect access locally with tmt remote devices.',
          );
          return;
        }
        if (!currentPage()) return;
        access.textContent = 'Access confirmed';
        showState(
          status,
          'paired',
          `Checked at ${new Date().toISOString()}. The signed response matches this browser’s saved trust pins. Use your app’s local link in this browser; no work was sent.`,
        );
      } catch {
        if (!currentPage()) return;
        access.textContent = 'Could not verify access';
        showState(
          status,
          'blocked',
          'Your saved pairing has not been changed. The door may be unavailable or its reply could not be verified. Inspect access locally with tmt remote devices; no work was sent.',
        );
      }
      // A successful check opens one Session. Do not create more merely to repaint status.
    }
  });
}
function entryUuid(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value)
  );
}
function entryAddress(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.startsWith(`${location.origin}/r/`) &&
    /^[a-z2-7]{16}$/.test(value.slice(`${location.origin}/r/`.length))
  );
}
function validateEntryDoor(current: Door): void {
  if (
    !entryAddress(current.address) ||
    !entryUuid(current.windowId) ||
    !entryUuid(current.machineId)
  ) {
    throw new Error('Invalid door descriptor.');
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
  if (!found) throw new Error(`The Remote page lacks #${id}.`);
  return found;
}
/** Present the ceremony state without coloring its instruction text. */
function showState(
  status: HTMLElement,
  state: 'waiting' | 'paired' | 'blocked',
  text: string,
): void {
  status.dataset.state = state;
  element('notice').dataset.tone = { waiting: 'waiting', paired: 'working', blocked: 'blocked' }[
    state
  ];
  element('state-label').textContent = {
    waiting: 'Waiting',
    paired: 'Paired',
    blocked: 'Unavailable',
  }[state];
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
  element('state-label').textContent = 'Ready to pair';
  status.textContent = 'Name this browser and choose Pair to request terminal confirmation.';
  button.disabled = false;
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
  showState(
    status,
    'waiting',
    'Waiting for confirmation in your terminal. Compare these words and confirm only if they match.',
  );
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
  element('entry-link').hidden = false;
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
