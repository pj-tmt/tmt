import '@tmt/browser-ui/static.css';
import '../src/style.css';
// The module bundle carries these with the switcher; the app's colour and font aliases live here.
import '../src/colab-header.css';
import { createSearchRegistry, type SearchItem, type SearchProvider } from '@tmt/browser-ui/search';
import { mount as mountEntry, type SearchEntry } from '../src/search-entry.js';
import type { PageSummary, PageTransport } from '../src/transport.js';

export const id = (n: number) => `c${String(n).padStart(7, '0')}-1111-4111-8111-000000000001`;
const route = (key: string, label: string, detail: string, icon: string): SearchItem => ({
  key,
  label,
  detail,
  icon,
  href: `/${key}/`,
});

interface Fixture {
  entry: SearchEntry;
  host: HTMLElement;
  opener: HTMLButtonElement;
  registry: ReturnType<typeof createSearchRegistry>;
  state: Record<string, { items: SearchItem[] | 'unavailable'; listeners: Set<() => void> }>;
}
let fixture: Fixture | undefined;
let reads = 0;
let went: string[] = [];
let navigated: string[] = [];
let failed = false;
let pages: PageSummary[] = [];

function provider(key: string, label: string, fixtureState: Fixture['state']): SearchProvider {
  const own = fixtureState[key]!;
  let cached: ReturnType<SearchProvider['getSnapshot']> | undefined;
  let source: SearchItem[] | 'unavailable' | undefined;
  return {
    key,
    label,
    getSnapshot() {
      if (source !== own.items) {
        source = own.items;
        cached =
          own.items === 'unavailable'
            ? { state: 'unavailable' }
            : { state: 'ready', items: own.items };
      }
      return cached!;
    },
    subscribe(listener) {
      own.listeners.add(listener);
      return () => void own.listeners.delete(listener);
    },
  };
}

/** A search host that is not a Colab route: a bar with the opener, the entry, and seeded providers. */
export function mount(
  mode: 'ready' | 'cold-failed' | 'no-colab-data' = 'ready',
  placeholder?: string,
) {
  dispose();
  reads = 0;
  went = [];
  navigated = [];
  failed = mode === 'cold-failed';
  pages = [
    {
      id: id(1),
      title: 'Alpha brief',
      sharing: 'private',
      projects: ['design'],
      lastUpdateAtMs: 30,
      summary: { comments: 0, waiting: false, needsYou: 0, author: 'Ada' },
    },
    {
      id: id(2),
      title: 'Beta review',
      sharing: 'private',
      projects: ['ops'],
      lastUpdateAtMs: 20,
      summary: { comments: 1, waiting: true, needsYou: 1, author: 'Pat' },
    },
    {
      id: id(3),
      title: 'Design notes',
      sharing: 'private',
      projects: ['design'],
      lastUpdateAtMs: 10,
      summary: { comments: 0, waiting: false, needsYou: 0, author: 'Ada' },
    },
  ];
  const transport: PageTransport = {
    backendName: 'Fixture device',
    async spaceHome() {
      reads++;
      if (failed) throw new Error('Fixture list read refused');
      return { title: 'Studio pages', pages: mode === 'no-colab-data' ? [] : pages };
    },
    async page(pageId) {
      throw new Error(`Fixture page ${pageId} is not read here`);
    },
  };
  document.body.replaceChildren();
  const bar = document.createElement('header');
  bar.id = 'fixture-bar';
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
  const state: Fixture['state'] = {
    navigation: {
      items: [
        route('home', 'Home', 'TMT home', 'house'),
        route('colab', 'Colab pages', 'Module', 'file-text'),
        route('ops-agents', 'Ops agents', 'Section', 'users'),
        route('digest', 'Digest', 'Module', 'chart-column'),
      ],
      listeners: new Set(),
    },
    ops: {
      items: [
        {
          key: 'review-agent',
          label: 'review-agent',
          detail: 'Agent · available',
          icon: 'users',
          href: '/ops/#agents',
        },
      ],
      listeners: new Set(),
    },
    remote: { items: 'unavailable', listeners: new Set() },
  };
  const registry = createSearchRegistry();
  registry.register(provider('navigation', 'Navigation', state));
  registry.register(provider('ops', 'Ops', state));
  registry.register(provider('remote', 'Remote', state));
  const entry = mountEntry(host, {
    transport,
    registry,
    navigate: async (pageId) => void navigated.push(pageId),
    pageHref: (pageId) => `/colab/p/${pageId}`,
    go: async (href) => void went.push(href),
    projectHref: (name) => `/colab/?scope=${encodeURIComponent(name)}`,
    ...(placeholder === undefined ? {} : { placeholder }),
  });
  opener.addEventListener('click', () => entry.open(opener));
  fixture = { entry, host, opener, registry, state };
}
function change(key: string, items: SearchItem[] | 'unavailable') {
  const own = fixture!.state[key]!;
  own.items = items;
  for (const listener of Array.from(own.listeners)) listener();
}
export function setRemote(items: SearchItem[] | 'unavailable') {
  change('remote', items);
}
export function removeOpsItem(key: string) {
  const own = fixture!.state.ops!;
  change(
    'ops',
    own.items === 'unavailable' ? 'unavailable' : own.items.filter((item) => item.key !== key),
  );
}
export function openDirect() {
  fixture!.entry.open(fixture!.opener);
}
export function dispose() {
  fixture?.entry.dispose();
  fixture?.registry.dispose();
  fixture = undefined;
}
export function disposeEntryOnly() {
  fixture!.entry.dispose();
}
export function proof() {
  return { reads, went: [...went], navigated: [...navigated] };
}
