import {
  active,
  boundedAdmission,
  ReopenSessionError,
  type ReopenSessionOptions,
} from './session-reopen.js';
export { ReopenSessionError } from './session-reopen.js';
export type {
  ReopenSessionOptions,
  ReopenSessionReason,
  ReopenSessionDetail,
} from './session-reopen.js';
import { channelFor } from './session-channel.js';
import { RefusalError, parseCapabilities, verifiedSessionRequest } from './operations.js';
export { operations, ClientError, RefusalError } from './operations.js';
export { management } from './management.js';
export * as budget from './budget.js';
export type {
  RemoteManagement,
  FirestoreSettingsView,
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
async function door(send: typeof fetch = fetch): Promise<Door> {
  const response = await send('/sdk/mount', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path: location.pathname }),
  });
  if (response.status !== 200) throw new Error('The door did not answer.');
  return (await response.json()) as Door;
}
async function paired(signal?: AbortSignal): Promise<{ record: Stored; key: DeviceKey }> {
  let record: Stored | undefined;
  try {
    record = await load();
  } catch (error) {
    if (signal) throw new ReopenSessionError('transient', 'pairing-unavailable');
    throw error;
  }
  active(signal);
  if (signal && record === undefined)
    throw new ReopenSessionError('unpaired', 'pairing-unavailable');
  if (!record) throw new Error('This browser is not paired.');
  return { record, key: await DeviceKey.fromHandle(record.handle, record.publicKey) };
}

const admissions = new WeakMap<Session, Promise<Session>>();
/**
 * Reopen this browser's door session for the running remote; no owner step.
 * The first caller's signal governs a coalesced bounded series; joined callers' signals are ignored.
 * A signal without bounded retry selects one typed attempt, bounded by the caller's cancellation.
 */
export function reopenSession(
  previous?: Session,
  options?: ReopenSessionOptions,
): Promise<Session> {
  if (options?.retry !== 'bounded') return reopenOnce(previous, options?.signal);
  if (previous && !channelFor(previous))
    return Promise.reject(new TypeError('Use a verified previous Session.'));
  const joined = previous && admissions.get(previous);
  if (joined) return joined;
  const pending = boundedAdmission((signal) => reopenOnce(previous, signal), options.signal);
  if (previous) {
    admissions.set(previous, pending);
    void pending
      .finally(() => {
        if (admissions.get(previous) === pending) admissions.delete(previous);
      })
      .catch(() => {});
  }
  return pending;
}
async function reopenOnce(previous?: Session, signal?: AbortSignal): Promise<Session> {
  active(signal);
  const send: typeof fetch = async (input, init) => {
    active(signal);
    let response: Response;
    try {
      response = await fetch(input, { ...init, signal });
    } catch (error) {
      active(signal);
      if (signal) throw new ReopenSessionError('unreachable', 'admission-unconfirmed');
      throw error;
    }
    active(signal);
    if (signal && response.status !== 200)
      throw new ReopenSessionError('unconfirmed', 'admission-unconfirmed');
    return response;
  };
  let stored: { record: Stored; key: DeviceKey };
  try {
    stored = await paired(signal);
  } catch (error) {
    active(signal);
    if (signal) {
      if (error instanceof ReopenSessionError) throw error;
      throw new ReopenSessionError('transient', 'pairing-unavailable');
    }
    throw error;
  }
  active(signal);
  const { record, key } = stored;
  if (signal && (record.paired.origin !== location.origin || record.paired.kind !== 'browser'))
    throw new ReopenSessionError('mismatch', 'identity-mismatch');
  if (previous) {
    const old = channelFor(previous);
    if (!old) throw new TypeError('Use a verified previous Session.');
    if (!sameIdentity(record, old.paired, old.key.publicKey())) {
      if (signal) throw new ReopenSessionError('mismatch', 'identity-mismatch');
      throw new RefusalError('REMOTE_SESSION_ENDED');
    }
  }
  const current = await door(send);
  try {
    validateDoor(current, record);
  } catch (error) {
    if (signal)
      throw new ReopenSessionError(
        current.machineId !== record.paired.machineId ? 'mismatch' : 'transient',
        current.machineId !== record.paired.machineId
          ? 'identity-mismatch'
          : 'unverifiable-response',
      );
    throw error;
  }
  let refused = false;
  try {
    const replacement = await openSession(
      { ...record.paired, address: current.address },
      key,
      current.windowId,
      async (url, init) => {
        const response = await send(url, init);
        refused = response.status === 404;
        return response;
      },
      signal ? { signal, transport: fetch } : undefined,
    );
    if (signal) {
      const latest = await paired(signal);
      active(signal);
      if (!sameIdentity(latest.record, record.paired, key.publicKey()))
        throw new ReopenSessionError('mismatch', 'identity-mismatch');
    }
    return replacement;
  } catch (error) {
    active(signal);
    if (signal) {
      if (error instanceof ReopenSessionError) throw error;
      if (error instanceof RefusalError && error.code === 'REMOTE_SESSION_LIMIT')
        throw new ReopenSessionError('capacity', 'admission-refused', error);
      throw new ReopenSessionError('transient', 'unverifiable-response');
    }
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
function sameIdentity(record: Stored, pinned: Paired, publicKey: Uint8Array): boolean {
  const pin = record.paired.machinePublicKey;
  return (
    record.paired.clientId === pinned.clientId &&
    record.paired.machineId === pinned.machineId &&
    record.paired.origin === pinned.origin &&
    pin instanceof Uint8Array &&
    pin.length === 32 &&
    pinned.machinePublicKey.length === 32 &&
    pin.every((byte, index) => byte === pinned.machinePublicKey[index]) &&
    record.publicKey.length === publicKey.length &&
    record.publicKey.every((byte, index) => byte === publicKey[index])
  );
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

type EntryState =
  | 'missing'
  | 'checking'
  | 'connected'
  | 'different'
  | 'refused'
  | 'unconfirmed'
  | 'unreadable';

/** Landing-page observation uses the verified Session lane; it never dispatches work. */
export async function landingPage(): Promise<void> {
  if (document.readyState === 'loading') {
    await new Promise<void>((resolve) =>
      document.addEventListener('DOMContentLoaded', () => resolve(), { once: true }),
    );
  }
  const button = element('check') as HTMLButtonElement;
  let attempt = 0;
  let departed = false;
  let session: Session | undefined;
  let opaque = false;
  const pageAlive = (): boolean => !departed && button.isConnected;
  window.addEventListener(
    'pagehide',
    () => {
      departed = true;
      attempt++;
    },
    { once: true },
  );
  const commandActions = [
    ['missing', 'pair'],
    ['different', 'pair'],
    ['refused', 'devices'],
    ['refused', 'pair'],
    ['unconfirmed', 'status'],
    ['unreadable', 'pair'],
  ] as const;
  const render = (state: EntryState, validated = false): void => {
    const copy = {
      missing: ['○', 'Not paired', 'Pair this browser with your machine', ''],
      checking: [
        '◌',
        'Checking',
        'Checking the connection…',
        'This browser is paired. Nothing is sent while checking.',
      ],
      connected: [
        '●',
        'Connected',
        'This browser can use Remote',
        'Open your app from its link in this browser.',
      ],
      different: [
        '△',
        'Different machine',
        'This browser is paired with another machine',
        "This Remote runs on a machine this browser isn't paired with. Your existing pairing is kept.",
      ],
      refused: [
        '×',
        'Not accepted',
        "Remote didn't accept this browser",
        'The pairing may have been removed on the machine.',
      ],
      unconfirmed: [
        '×',
        "Can't reach Remote",
        "Couldn't confirm the connection",
        "Remote may have stopped, or its reply couldn't be verified. Your pairing is unchanged.",
      ],
      unreadable: [
        '×',
        'Pairing unreadable',
        "This browser's pairing can't be read",
        'Browser storage may have been cleared or blocked. Nothing was sent.',
      ],
    }[state];
    element('notice').dataset.tone =
      state === 'connected'
        ? 'working'
        : ['missing', 'checking', 'different'].includes(state)
          ? 'waiting'
          : 'blocked';
    element('notice').dataset.state = state;
    for (const [copyState, command] of commandActions)
      element(`copy-feedback-${copyState}-${command}`).textContent = '';
    element('mark').textContent = copy[0]!;
    element('state-label').textContent = copy[1]!;
    element('heading').textContent = copy[2]!;
    element('status').textContent =
      state === 'checking' && !validated ? "Reading this browser's pairing." : copy[3]!;
    for (const name of ['missing', 'different', 'refused', 'unconfirmed', 'unreadable'])
      element(`steps-${name}`).hidden = name !== state;
    element('status').hidden = state === 'missing';
    element('pairing-note').hidden = state !== 'missing';
    element('command-location').hidden = ['checking', 'connected'].includes(state);
    button.hidden = ['missing', 'different', 'unreadable'].includes(state);
    button.disabled = state === 'checking';
  };
  for (const [state, command] of commandActions) {
    element(`copy-${state}-${command}`).addEventListener('click', async () => {
      const text = `tmt remote ${command}`;
      try {
        await navigator.clipboard.writeText(text);
        if (pageAlive()) element(`copy-feedback-${state}-${command}`).textContent = 'Copied.';
      } catch {
        if (pageAlive())
          element(`copy-feedback-${state}-${command}`).textContent =
            'Copy failed. Select the command and copy it manually.';
      }
    });
  }
  const check = async (): Promise<void> => {
    const restoreFocus = document.activeElement === button;
    const currentAttempt = ++attempt;
    const currentPage = (): boolean => pageAlive() && attempt === currentAttempt;
    render('checking');
    element('checked-time').textContent = 'Not checked';
    const local = await entryPairing();
    if (!currentPage()) return;
    if (local.state !== 'saved') {
      session = undefined;
      render(local.state === 'missing' ? 'missing' : 'unreadable');
      return;
    }
    render('checking', true);
    element('machine-id').textContent = local.record.paired.machineId.slice(0, 8);
    element('protocol-address').textContent = local.record.paired.address;
    element('trust-pin').textContent = Array.from(local.record.paired.machinePublicKey, (byte) =>
      byte.toString(16).padStart(2, '0'),
    ).join('');
    const step: { phase: 'open' | 'capabilities' } = { phase: 'open' };
    const channel = session && channelFor(session);
    const reused =
      !!channel &&
      !channel.ended &&
      channel.paired.clientId === local.record.paired.clientId &&
      channel.paired.machineId === local.record.paired.machineId &&
      channel.paired.machinePublicKey.every(
        (byte, index) => byte === local.record.paired.machinePublicKey[index],
      ) &&
      channel.key.publicKey().every((byte, index) => byte === local.record.publicKey[index]);
    const open = async (): Promise<boolean> => {
      step.phase = 'open';
      opaque = false;
      session = undefined;
      const current = await door();
      if (!currentPage()) return false;
      validateEntryDoor(current);
      element('protocol-address').textContent = current.address;
      if (current.machineId !== local.record.paired.machineId) {
        render('different');
        return false;
      }
      try {
        session = await openSession(
          { ...local.record.paired, address: current.address },
          local.key,
          current.windowId,
          async (url, init) => {
            // Page lifetime, not an individual check: a manual check reuses this channel.
            if (!pageAlive()) throw new Error('Remote page ended.');
            const response = await fetch(url, init);
            opaque = response.status === 404;
            return response;
          },
        );
      } catch (error) {
        if (!currentPage()) return false;
        if (opaque) {
          // Descriptor-only recheck, never another admission after a refused open.
          const latest = await door();
          if (!currentPage()) return false;
          validateEntryDoor(latest);
        }
        throw error;
      }
      return currentPage();
    };
    const observe = async (): Promise<void> => {
      step.phase = 'capabilities';
      await verifiedSessionRequest(session!)(
        'capabilities',
        crypto.randomUUID(),
        {},
        parseCapabilities,
      );
    };
    opaque = false;
    try {
      if (!reused && !(await open())) return;
      if (!currentPage()) return;
      try {
        await observe();
      } catch (error) {
        if (!currentPage()) return;
        if (
          !reused ||
          !(error instanceof RefusalError) ||
          !['REMOTE_SESSION_ENDED', 'REMOTE_SESSION_EVICTED'].includes(error.code)
        )
          throw error;
        // One reopen belongs to this explicit check; a fresh failure cannot loop.
        if (!(await open())) return;
        await observe();
      }
      if (!currentPage()) return;
      element('checked-time').textContent = new Intl.DateTimeFormat(undefined, {
        dateStyle: 'medium',
        timeStyle: 'short',
      }).format(new Date());
      render('connected');
    } catch (error) {
      if (!currentPage()) return;
      const verifiedRefusal =
        !opaque &&
        error instanceof RefusalError &&
        !(
          step.phase === 'capabilities' &&
          (error.code === 'REMOTE_SESSION_ENDED' ||
            (reused && error.code === 'REMOTE_SESSION_EVICTED'))
        ) &&
        [
          'REMOTE_CLOSED',
          'REMOTE_SESSION_ENDED',
          'REMOTE_SESSION_EVICTED',
          'REMOTE_DEVICE_REVOKED',
        ].includes(error.code);
      session = undefined;
      render(verifiedRefusal ? 'refused' : 'unconfirmed');
    } finally {
      if (currentPage() && restoreFocus && !button.hidden) button.focus();
    }
  };
  button.addEventListener('click', () => {
    if (!button.disabled) void check();
  });
  await check();
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
    const ownsFocus = form.contains(document.activeElement);
    form.hidden = true;
    if (ownsFocus) status.focus({ preventScroll: true });
    const name = (element('name') as HTMLInputElement).value.trim();
    void ceremony(parsed, name, status).catch(() => {
      if (document.activeElement === element('comparison')) status.focus({ preventScroll: true });
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
  const comparison = element('comparison');
  comparison.hidden = false;
  if (document.activeElement === status) comparison.focus({ preventScroll: true });
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
  if (document.activeElement === comparison || document.activeElement === status)
    element('entry-link').querySelector<HTMLAnchorElement>('a')!.focus({ preventScroll: true });
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
