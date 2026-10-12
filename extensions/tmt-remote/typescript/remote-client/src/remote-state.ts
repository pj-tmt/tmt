import type { RemoteReference } from './remote-registry.js';

/** Current-origin observation only. A foreign entry has no mark or live-access claim. */
export function remoteState(state: string, current = true) {
  if (!current)
    return { text: 'not checked', word: 'not checked', mark: '', tone: 'muted', action: '' };
  const value =
    state === 'Connected'
      ? ['Connected', 'connected', '●', 'working', '']
      : state === 'Checking'
        ? ['Checking', 'connecting', '◌', 'waiting', '']
        : state === 'Access ended'
          ? ['Access ended', 'access ended', '✗', 'blocked', 'Pair again']
          : state === 'Different machine'
            ? ['Different machine', 'different machine', '△', 'waiting', 'Pair again']
            : state === 'Not paired' || state === 'Pairing unreadable'
              ? [state, 'not paired', '○', 'waiting', 'Pair again']
              : ['Unconfirmed', 'unreachable', '○', 'blocked', 'Retry'];
  const [text, word, mark, tone, action] = value as [string, string, string, string, string];
  return { text: `${mark} ${text}`, word, mark, tone, action };
}

/** Both page owners and the menu identify the current Local row by the same door root. */
export function currentRemote(
  entry: Pick<RemoteReference, 'kind' | 'url'>,
  address: string | undefined,
  pageUrl: string,
): boolean {
  return (
    entry.kind === 'local' &&
    (entry.url === address || new URL(entry.url).href === new URL('./', pageUrl).href)
  );
}
