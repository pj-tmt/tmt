export {
  bindSyncContext,
  syncDocumentPath,
  verifySealedObject,
  validateSyncDocument,
  validateSyncFields,
  validateSyncQuery,
  validateSyncInterest,
  validateSyncDiscovery,
  validateSyncMutation,
} from './sealed-sync.js';
export type {
  SyncIdentity,
  SyncToken,
  SyncAuthState,
  SyncAuthFailure,
  SyncAuthPort,
  SyncCall,
  SyncResource,
  SyncContext,
  SyncTarget,
  SealedDocument,
  SyncCursor,
  SyncQuery,
  SyncInterest,
  SyncDiscovery,
  SyncQueryRequest,
  SyncQueryPage,
  SyncDiscoveryPage,
  SyncMutation,
  SyncDiscoveryMapping,
  SyncValue,
  SyncMapping,
  SyncQueryMapping,
  SyncPage,
  SyncChange,
  SyncSubscription,
  SyncPublish,
  SyncFailure,
  SealedSyncPort,
  SealedObjectManifest,
} from './sealed-sync.js';
import { deviceObjects as store, request } from './browser-store.js';
import { remoteRegistryPage } from './remote-registry-page.js';
export { remoteRegistry, RemoteRegistryError } from './remote-registry.js';
export type {
  RemoteReference,
  RemoteCatalog,
  NavigationHint,
  RegistryHandoff,
  ImportPlan,
} from './remote-registry.js';
import type { HostedDeclaration } from './firebase-declaration.js';
export type { HostedDeclaration } from './firebase-declaration.js';
import { connectHosted, type HostedAuth } from './firebase-auth.js';
import { originLink } from './firebase-bootstrap.js';
export type { HostedAuth, SyncAdmissionGuard, SyncAdmissionCost } from './firebase-auth.js';

/** Explicit hosted bootstrap; ordinary local SDK imports never load Firebase. */
export const firebaseRemote = {
  originLink(): Promise<string> {
    return originLink(location.origin, globalThis.fetch.bind(globalThis));
  },
  connect(link: string, declaration: HostedDeclaration): Promise<HostedAuth> {
    return connectHosted(link, declaration, {
      origin: location.origin,
      fetch: globalThis.fetch.bind(globalThis),
      load: () => import(/* @vite-ignore */ '/sdk/remote-firebase-v1.js'),
    });
  },
};

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
  RemovalPreview,
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
  resolveAutomaticLink,
  cancelAutomaticOffer,
  PairingRefusedError,
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
const RECORD = 'device';

interface Stored {
  handle: CryptoKey;
  publicKey: Uint8Array;
  paired: Paired;
}

async function load(): Promise<Stored | undefined> {
  const objects = await store('readonly');
  return (await request(() => objects.get(RECORD))) as Stored | undefined;
}
async function save(record: Stored): Promise<void> {
  const objects = await store('readwrite');
  await request(() => objects.put(record, RECORD));
}

/** A stored identity is retained unless the pinned machine proves it is dead. */
async function existingPairing(signal: AbortSignal): Promise<Stored | undefined> {
  let record: Stored | undefined;
  try {
    record = await load();
  } catch {
    throw new ReopenSessionError('transient', 'pairing-unavailable');
  }
  active(signal);
  if (record === undefined) return undefined;
  try {
    await reopenSession(undefined, { retry: 'bounded', signal });
    return record;
  } catch (error) {
    if (
      error instanceof ReopenSessionError &&
      (error.reason === 'revoked' || error.reason === 'expired')
    )
      return undefined;
    throw error;
  }
}
function pairingRecovery(error: unknown): string {
  if (error instanceof ReopenSessionError && error.detail === 'pairing-unavailable')
    return 'Your saved pairing could not be read. Check browser storage and try again.';
  if (error instanceof ReopenSessionError && error.reason === 'capacity')
    return "This browser has reached the owner's connection limit. Close another tab, or ask the owner to change the limit. Your pairing is unchanged.";
  return "Remote may have stopped, or its reply couldn't be verified. Your pairing is unchanged.";
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
      if (error instanceof RefusalError) {
        if (error.code === 'REMOTE_SESSION_LIMIT')
          throw new ReopenSessionError('capacity', 'admission-refused', error);
        if (error.code === 'REMOTE_DEVICE_REVOKED' || error.code === 'REMOTE_GRANT_EXPIRED')
          throw new ReopenSessionError(
            error.code === 'REMOTE_DEVICE_REVOKED' ? 'revoked' : 'expired',
            'admission-refused',
            error,
          );
      }
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
  const registryState = remoteRegistryPage();
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
  const render = (state: EntryState, validated = false, refusalCode?: string): void => {
    const authorityEnded = ['REMOTE_DEVICE_REVOKED', 'REMOTE_GRANT_EXPIRED'].includes(
      refusalCode ?? '',
    );
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
        refusalCode === 'REMOTE_SESSION_LIMIT'
          ? "This browser has reached the owner's connection limit. Close another tab, or ask the owner to change the limit. Your pairing is unchanged."
          : authorityEnded
            ? 'This pairing was removed or has expired. To use Remote here, run tmt remote pair on the machine.'
            : "This page's connection ended. Try again. Your pairing is unchanged.",
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
        'Your saved pairing could not be read. Check browser storage and try again.',
      ],
    }[state];
    element('notice').dataset.tone =
      state === 'connected'
        ? 'working'
        : ['missing', 'checking', 'different'].includes(state)
          ? 'waiting'
          : 'blocked';
    element('notice').dataset.state = state;
    registryState(
      authorityEnded ? 'Access ended' : copy[1]!,
      element('protocol-address').textContent ?? undefined,
    );
    for (const [copyState, command] of commandActions)
      element(`copy-feedback-${copyState}-${command}`).textContent = '';
    element('mark').textContent = copy[0]!;
    element('state-label').textContent = copy[1]!;
    element('heading').textContent = copy[2]!;
    element('status').textContent =
      state === 'checking' && !validated ? "Reading this browser's pairing." : copy[3]!;
    if (state === 'refused' && authorityEnded) {
      const command = document.createElement('code');
      command.className = 'tmt-ui-code';
      command.textContent = 'tmt remote pair';
      const [before, after] = copy[3]!.split(command.textContent);
      element('status').replaceChildren(
        document.createTextNode(before!),
        command,
        document.createTextNode(after!),
      );
    }
    for (const name of ['missing', 'different', 'refused', 'unconfirmed', 'unreadable'])
      element(`steps-${name}`).hidden =
        name !== state || (name === 'refused' && !authorityEnded) || name === 'unreadable';
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
          'REMOTE_SESSION_ENDED',
          'REMOTE_SESSION_EVICTED',
          'REMOTE_DEVICE_REVOKED',
          'REMOTE_GRANT_EXPIRED',
        ].includes(error.code);
      session = undefined;
      render(
        verifiedRefusal ? 'refused' : 'unconfirmed',
        false,
        error instanceof RefusalError ? error.code : undefined,
      );
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
let pairingAttempt: AbortController | undefined;
const manualPairingText = new WeakMap<HTMLElement, string>();
export async function pairingPage(link: string): Promise<void> {
  pairingAttempt?.abort();
  const lifetime = new AbortController();
  pairingAttempt = lifetime;
  globalThis.addEventListener?.('pagehide', () => lifetime.abort(), {
    once: true,
    signal: lifetime.signal,
  });
  if (document.readyState === 'loading') {
    await new Promise<void>((resolve) =>
      document.addEventListener('DOMContentLoaded', () => resolve(), { once: true }),
    );
  }
  if (lifetime.signal.aborted) return;
  const automatic = link.includes('#auto=');
  for (const id of ['pairing-title', 'heading']) {
    const node = element(id);
    if (!manualPairingText.has(node)) manualPairingText.set(node, node.textContent ?? '');
    node.textContent = automatic ? 'Connect this browser' : manualPairingText.get(node)!;
  }
  element('pairing-eyebrow').hidden = automatic;
  if (automatic) return automaticPairingPage(link, lifetime);
  const status = element('status');
  const form = element('pair') as HTMLFormElement;
  const button = form.querySelector('button')!;
  button.disabled = true;
  form.hidden = true;
  try {
    if (await existingPairing(lifetime.signal)) {
      active(lifetime.signal);
      showState(status, 'paired', 'This browser is already paired.');
      location.replace('/remote/');
      return;
    }
  } catch (error) {
    // A different machine cannot use this record. The manual terminal ceremony
    // authenticates its replacement; automatic entry must not replace it.
    if (lifetime.signal.aborted) return;
    if (!(error instanceof ReopenSessionError && error.reason === 'mismatch')) {
      showState(status, 'blocked', pairingRecovery(error));
      return;
    }
  }
  // Enrollment may prepare a name while the manual offer loads.
  // The action stays disabled until the offer and its submit listener are ready.
  form.hidden = false;
  let parsed: ReturnType<typeof parseLink>;
  try {
    parsed = await resolveLink(link, (url, init) =>
      fetch(url, { ...init, signal: lifetime.signal }),
    );
    if (lifetime.signal.aborted) return;
  } catch {
    if (lifetime.signal.aborted) return;
    form.hidden = true;
    showState(
      status,
      'blocked',
      'This pairing link is unavailable. Run tmt remote pair again for a new link.',
    );
    return;
  }
  form.addEventListener(
    'submit',
    (event) => {
      event.preventDefault();
      const ownsFocus = form.contains(document.activeElement);
      form.hidden = true;
      if (ownsFocus) status.focus({ preventScroll: true });
      const name = (element('name') as HTMLInputElement).value.trim();
      void ceremony(parsed, name, status, lifetime.signal).catch((error) => {
        if (lifetime.signal.aborted) return;
        if (document.activeElement === element('comparison')) status.focus({ preventScroll: true });
        showState(
          status,
          'blocked',
          error instanceof ReopenSessionError
            ? pairingRecovery(error)
            : 'Pairing did not complete. Run tmt remote pair again for a new link.',
        );
      });
    },
    { signal: lifetime.signal },
  );
  form.hidden = false;
  showState(
    status,
    'waiting',
    'Name this browser and choose Pair to request terminal confirmation.',
  );
  element('state-label').textContent = 'Ready to pair';
  button.disabled = false;
}

async function automaticPairingPage(link: string, lifetime: AbortController): Promise<void> {
  const status = element('status');
  element('pair').hidden = true;
  element('comparison').hidden = true;
  element('entry-link').hidden = true;
  const timer = setTimeout(() => lifetime.abort(), 60_000);
  let leaving = false;
  const departed = () => {
    leaving = true;
    lifetime.abort();
  };
  globalThis.addEventListener?.('pagehide', departed, { once: true });
  let code: Uint8Array | undefined;
  let openCommand = 'tmt remote open';
  let enrollmentAllowed = false;
  try {
    showState(status, 'waiting', 'Connecting this browser…');
    const network: typeof fetch = (url, init) => fetch(url, { ...init, signal: lifetime.signal });
    const offered = await resolveAutomaticLink(link, network);
    code = offered.code;
    const { descriptor, target } = offered;
    // The fixed public entry families name Colab; this is recovery copy, not authority.
    if (
      target === '/colab' ||
      target.startsWith('/colab/') ||
      target.startsWith('/p/') ||
      target.startsWith('/read/')
    )
      openCommand = 'tmt colab open';
    const record = await existingPairing(lifetime.signal);
    enrollmentAllowed = record === undefined;
    lifetime.signal.throwIfAborted();
    if (record) {
      // The existing signed session proves the machine pin and live device, not the inventory count.
      const key = await DeviceKey.fromHandle(record.handle, record.publicKey);
      await cancelAutomaticOffer({
        descriptor,
        code,
        key,
        kind: 'browser',
        origin: location.origin,
        name: 'Browser',
        fetch: network,
        signal: lifetime.signal,
      });
    } else {
      const key = await DeviceKey.generate();
      const result = await pair({
        descriptor,
        code,
        key,
        kind: 'browser',
        origin: location.origin,
        name: 'Browser',
        fetch: network,
        deadlineMs: 60_000,
        signal: lifetime.signal,
      });
      lifetime.signal.throwIfAborted();
      await save({ handle: key.handle(), publicKey: key.publicKey(), paired: result });
      enrollmentAllowed = false;
      await openSession(result, key, descriptor.windowId, network);
    }
    lifetime.signal.throwIfAborted();
    location.replace(target);
  } catch (error) {
    if (!leaving && pairingAttempt === lifetime) {
      showState(status, 'blocked', '');
      element('state-label').textContent = 'Not connected';
      if (error instanceof ReopenSessionError) {
        if (error.reason === 'mismatch') {
          const command = document.createElement('code');
          command.className = 'tmt-ui-code';
          command.textContent = 'tmt remote pair';
          status.replaceChildren(
            document.createTextNode(
              'This browser is paired with a different machine. To use this one, run ',
            ),
            command,
            document.createTextNode(' on it.'),
          );
        } else status.textContent = pairingRecovery(error);
        return;
      }
      const command = document.createElement('code');
      command.className = 'tmt-ui-code';
      command.textContent = openCommand;
      const refused = error instanceof PairingRefusedError;
      const nodes: Node[] = [
        document.createTextNode(
          refused
            ? 'This link no longer works. Open the page again from the machine with '
            : 'Could not connect this browser. Open the page again from the machine with ',
        ),
        command,
        document.createTextNode(refused || !enrollmentAllowed ? '.' : '. If it still fails, run '),
      ];
      if (!refused && enrollmentAllowed) {
        const fallback = document.createElement('code');
        fallback.className = 'tmt-ui-code';
        fallback.textContent = 'tmt remote pair';
        nodes.push(fallback, document.createTextNode(' there.'));
      }
      status.replaceChildren(...nodes);
    }
  } finally {
    code?.fill(0);
    clearTimeout(timer);
    globalThis.removeEventListener?.('pagehide', departed);
  }
}
async function ceremony(
  { descriptor, code }: ReturnType<typeof parseLink>,
  name: string,
  status: HTMLElement,
  signal: AbortSignal,
): Promise<void> {
  const key = await DeviceKey.generate();
  signal.throwIfAborted();
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
    signal,
  });
  signal.throwIfAborted();
  try {
    await save({
      handle: key.handle(),
      publicKey: key.publicKey(),
      paired: result,
    });
  } catch {
    throw new ReopenSessionError('transient', 'pairing-unavailable');
  }
  try {
    await openSession(result, key, descriptor.windowId, (url, init) =>
      fetch(url, { ...init, signal }),
    );
  } catch {
    throw new ReopenSessionError('unconfirmed', 'admission-unconfirmed');
  }
  signal.throwIfAborted();
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
