import type { RemoteReference } from './remote-registry.js';

export type ViewerRole = 'owner' | 'user';
export interface RemoteConnectionView {
  readonly text: string;
  readonly word: string;
  readonly mark: string;
  readonly tone: 'neutral' | 'muted';
  readonly action: string;
}

/** A display observation only; pairing and sending permissions never imply a role. */
export function remoteRole(role: unknown): 'Admin' | 'Member' | 'Role unknown' {
  return role === 'owner' ? 'Admin' : role === 'user' ? 'Member' : 'Role unknown';
}

/** M1 browser-role display from the existing readable management observation; grants nothing. */
export function managementRole(
  access: string,
  settingsWrite: boolean | undefined,
): ViewerRole | undefined {
  if (access !== 'live') return undefined;
  return settingsWrite === true ? 'owner' : settingsWrite === false ? 'user' : undefined;
}

/** Foreign references stay unchecked. Unknown input does not become an offline verdict. */
export function remoteState(state: string, current = true): RemoteConnectionView {
  if (!current)
    return { text: 'Not checked', word: 'not checked', mark: '◌', tone: 'muted', action: '' };
  const known: Record<string, readonly [string, string, string, string]> = {
    Connected: ['Connected', 'connected', '●', ''],
    Checking: ['Checking…', 'connecting', '◌', ''],
    'Access ended': ['Access ended', 'access ended', '✗', 'Pair again'],
    'Different machine': ['Different machine', 'different machine', '?', 'Pair again'],
    'Not paired': ['Not paired', 'not paired', '?', 'Pair again'],
    'Pairing unreadable': ['Pairing unreadable', 'pairing unreadable', '?', 'Pair again'],
    Offline: ['Offline', 'offline', '✗', 'Retry'],
    Unconfirmed: ['Connection unconfirmed', 'unconfirmed', '?', 'Retry'],
    'Not accepted': ['Not accepted', 'not accepted', '?', 'Retry'],
    "Can't reach Remote": ['Connection unconfirmed', 'unconfirmed', '?', 'Retry'],
  };
  const value = Object.hasOwn(known, state) ? known[state] : undefined;
  if (!value)
    return { text: 'Status unknown', word: 'unknown', mark: '?', tone: 'muted', action: '' };
  const [text, word, mark, action] = value;
  return { text, word, mark, action, tone: 'neutral' };
}

/** The native page's door root, shared by the registry projection and current-row check. */
export function currentDoor(pageUrl: string): string {
  const root = new URL('./', pageUrl);
  if (root.pathname === '/remote/') root.pathname = '/';
  return root.href;
}

/** Both page owners and the menu identify the current Local row by the same door root. */
export function currentRemote(
  entry: Pick<RemoteReference, 'kind' | 'url'>,
  address: string | undefined,
  pageUrl: string,
): boolean {
  return (
    entry.kind === 'local' &&
    (entry.url === address || new URL(entry.url).href === currentDoor(pageUrl))
  );
}

/** Guidance never treats transport uncertainty or unreadable storage as permission to re-pair. */
export function pairingGuidance(
  state: string,
  authorityEnded: boolean,
  invite: boolean,
): { pair: boolean; devices: boolean; status: boolean } {
  return {
    pair:
      invite ||
      state === 'missing' ||
      state === 'different' ||
      (state === 'refused' && authorityEnded),
    devices: state === 'refused' && authorityEnded && !invite,
    status: state === 'unconfirmed' && !invite,
  };
}
