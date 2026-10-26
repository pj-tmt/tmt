import '@tmt/browser-ui/static.css';
import '../src/style.css';
// The module bundle carries these with the switcher; the app's colour and font aliases live here.
import '../src/colab-header.css';
import { createSearchRegistry } from '@tmt/browser-ui/search';
import { mountedTransport } from '../src/mounted.js';
import { mount as mountEntry } from '../src/search-entry.js';
import { createSearchSession } from '../src/search-session.js';
import type { SearchEntry } from '../src/search-entry.js';

let current: { entry: SearchEntry; registry: ReturnType<typeof createSearchRegistry> } | undefined;
let navigated: string[] = [];

/**
 * The global search on a document that is not Colab's (an Ops page): the real session and the real
 * mounted transport over the spec's stubbed door, with the paired mount named as the module's own
 * URL names it in a built app (`mountOfModule`, covered by a unit test).
 */
export function mount(mountPath: string) {
  dispose();
  navigated = [];
  document.body.replaceChildren();
  const bar = document.createElement('header');
  const opener = document.createElement('button');
  opener.type = 'button';
  opener.id = 'search-opener';
  opener.textContent = 'Search TMT';
  const title = document.createElement('h1');
  title.textContent = 'Ops agents';
  bar.append(opener);
  const host = document.createElement('div');
  host.id = 'search-host';
  document.body.append(bar, title, host);
  const registry = createSearchRegistry();
  const mountUrl = new URL(mountPath, location.href);
  const entry = createSearchSession(
    {
      admit: (signal) => mountedTransport(signal, { mount: mountUrl }),
      mountEntry,
      page: window,
    },
    host,
    {
      registry,
      navigate: async (pageId) => void navigated.push(pageId),
      pageHref: (pageId) => `/colab/p/${pageId}`,
      go: async () => undefined,
      placeholder: 'Search TMT…',
    },
  );
  opener.addEventListener('click', () => entry.open(opener));
  current = { entry, registry };
}
export function dispose() {
  current?.entry.dispose();
  current?.registry.dispose();
  current = undefined;
}
export function proof() {
  return { navigated: [...navigated], href: location.href, historyLength: history.length };
}
