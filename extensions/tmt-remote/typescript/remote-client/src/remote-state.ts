import type { RemoteReference } from './remote-registry.js';

/** Current-origin observation only. A foreign entry has no mark or live-access claim. A connection
 * state is words only, in the neutral tone: the shared agent marks mean agent states, not these. */
export function remoteState(state: string, current = true) {
  if (!current)
    return { text: 'not checked', word: 'not checked', mark: '', tone: 'muted', action: '' };
  const value =
    state === 'Connected'
      ? ['Connected', 'connected', '']
      : state === 'Checking'
        ? ['Checking…', 'connecting', '']
        : state === 'Access ended'
          ? ['Access ended', 'access ended', 'Pair again']
          : state === 'Different machine'
            ? ['Different machine', 'different machine', 'Pair again']
            : state === 'Not paired' || state === 'Pairing unreadable'
              ? [state, 'not paired', 'Pair again']
              : ['Unconfirmed', 'unreachable', 'Retry'];
  const [text, word, action] = value as [string, string, string];
  return { text, word, mark: '', tone: 'neutral', action };
}

/** Both page owners and the menu identify the current Local row by the same door root. */
export function currentRemote(
  entry: Pick<RemoteReference, 'kind' | 'url'>,
  address: string | undefined,
  pageUrl: string,
): boolean {
  const root = new URL('./', pageUrl);
  // Remote's pages moved, while saved public references still name the door root.
  if (root.pathname === '/remote/') root.pathname = '/';
  return entry.kind === 'local' && (entry.url === address || new URL(entry.url).href === root.href);
}
