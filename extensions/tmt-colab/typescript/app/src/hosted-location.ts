import { isOlderReaderFragment, parseReaderFragment, type ReaderLink } from './reader-link.js';

/** Firebase's alternate host also gets a truthful refusal, never the local sample preview. */
export function isHostedOrigin(origin: string): boolean {
  try {
    const url = new URL(origin);
    return (
      url.protocol === 'https:' && /^[a-z0-9-]+\.(web\.app|firebaseapp\.com)$/.test(url.hostname)
    );
  } catch {
    return false;
  }
}
interface Browser {
  location: Pick<Location, 'hash' | 'pathname' | 'search'>;
  history: Pick<History, 'state' | 'replaceState'>;
}
export class HostedOlderLink extends Error {
  readonly code = 'COLAB_HOSTED_OLDER_LINK';
}
/** Erase before async parsing/effects. The existing Colab parser alone owns the nine-key grammar. */
export function captureHostedLink(
  browser: Browser = { location, history },
): Promise<ReaderLink | undefined> {
  let fragment = browser.location.hash;
  browser.history.replaceState(
    browser.history.state,
    '',
    browser.location.pathname + browser.location.search,
  );
  try {
    if (!fragment) return Promise.resolve(undefined);
    if (isOlderReaderFragment(fragment)) return Promise.reject(new HostedOlderLink());
    return parseReaderFragment(fragment);
  } finally {
    fragment = '';
  }
}
