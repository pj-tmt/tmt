import { expect, it } from 'vite-plus/test';
import type { SearchProvider } from '@tmt/browser-ui/search';
import type { ShellAnswer, ShellItem } from '@tmt/browser-ui/shell';
import { navigationSnapshot, RootSearch, type SearchModule } from '../src/root-search.js';

const here = 'https://door.test/remote/';
const rail: ShellItem[] = [
  { id: 'home', label: 'Home', href: '/', icon: 'house', current: false },
  { id: 'colab', label: 'Colab', href: '/colab/', icon: 'file-text', current: false },
  { id: 'ops', label: 'Ops', href: '/ops/', icon: 'users', current: false, disabled: true },
  { id: 'remote', label: 'Remote', href: '/remote/', icon: 'monitor-smartphone', current: true },
  { id: 'settings', label: 'TMT settings', href: '/settings/', icon: 'settings', current: false },
];
const answer: ShellAnswer = {
  version: 1,
  modules: [
    {
      extension: 'colab',
      rail: { label: 'Colab', icon: 'file-text', path: '/colab/', order: 1 },
      menu: [
        { id: 'pages', label: 'Pages', path: '/colab/' },
        { id: 'foreign', label: 'Foreign', path: 'https://other.test/' },
      ],
      settings: [{ id: 'settings', label: 'Colab settings', path: '/colab/settings' }],
    },
  ],
};

it('offers only described, admitted navigation and keeps unknown distinct from empty', () => {
  expect(navigationSnapshot(undefined, rail, here, (href) => href)).toEqual({
    state: 'unavailable',
  });
  const snapshot = navigationSnapshot(answer, rail, here, (href) => href);
  expect(snapshot.state).toBe('ready');
  if (snapshot.state !== 'ready') throw new Error('Expected ready navigation');
  expect(snapshot.items.map((item) => item.key)).toEqual([
    'home',
    'colab',
    'colab:pages',
    'colab:settings',
    'settings',
  ]);
  expect(snapshot.items.find((item) => item.key === 'colab:pages')).toMatchObject({
    href: '/colab/',
    icon: 'file-text',
    detail: 'Colab',
  });
});

it('selects the primary description even when an extra appears before it', () => {
  const primary = answer.modules[0]!;
  const extra = {
    ...primary,
    module: 'design',
    menu: [{ id: 'wrong', label: 'Design only', path: '/colab/design/' }],
    settings: [],
  };
  const snapshot = navigationSnapshot(
    { version: 1, modules: [extra, primary] },
    rail,
    here,
    (href) => href,
  );
  expect(snapshot.state).toBe('ready');
  if (snapshot.state !== 'ready') throw new Error('Expected ready navigation');
  expect(snapshot.items.map((item) => item.key)).toEqual([
    'home',
    'colab',
    'colab:pages',
    'colab:settings',
    'settings',
  ]);
  const withoutPrimary = navigationSnapshot(
    { version: 1, modules: [extra] },
    rail,
    here,
    (href) => href,
  );
  expect(withoutPrimary.state).toBe('ready');
  if (withoutPrimary.state !== 'ready') throw new Error('Expected ready navigation');
  expect(withoutPrimary.items.map((item) => item.key)).toEqual(['home', 'settings']);
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => (resolve = done));
  return { promise, resolve };
}

function fixture(load?: (entry: string) => Promise<SearchModule>) {
  const events: string[] = [];
  let options: Parameters<SearchModule['mountSearchSession']>[1] | undefined;
  const module: SearchModule = {
    mountSearchSession(_container, next) {
      options = next;
      events.push('mount');
      return { open: () => events.push('open'), dispose: () => events.push('dispose') };
    },
  };
  const search = new RootSearch({
    load:
      load ??
      (async () => {
        events.push('import');
        return module;
      }),
    container: () => ({ remove: () => events.push('remove') }) as unknown as HTMLElement,
    failed: (message) => events.push(message),
    navigate: (href) => events.push(href),
  });
  const opener = {
    isConnected: true,
    focus: () => events.push('focus'),
  } as unknown as HTMLButtonElement;
  return { search, events, module, opener, options: () => options! };
}

it('prefetches once without admitting a Colab session; repeated opens share one owner', async () => {
  const f = fixture();
  f.search.update('/entry.js', { state: 'ready', items: [] });
  expect(f.events).toEqual(['import']);
  await f.search.open(f.opener);
  await f.search.open(f.opener);
  expect(f.events).toEqual(['import', 'mount', 'open', 'open']);
  expect(f.options().placeholder).toBe('Search TMT…');
  expect(f.options().registry.groups()[0]?.key).toBe('navigation');
  await f.options().navigate('a/b');
  expect(f.events.at(-1)).toBe('/colab/pages/a%2Fb');
  expect(f.options().projectHref('a & b')).toBe('/colab/?scope=a%20%26%20b');
  f.search.dispose();
  expect(f.events.slice(-2)).toEqual(['dispose', 'remove']);
});

it('a late import cannot mount or focus a departed route, and a new route reuses only code', async () => {
  const pending = deferred<SearchModule>();
  const f = fixture(() => pending.promise);
  f.search.update('/entry.js', { state: 'ready', items: [] });
  const open = f.search.open(f.opener);
  f.search.replace();
  pending.resolve(f.module);
  await open;
  expect(f.events).toEqual([]);
  f.search.update('/entry.js', { state: 'ready', items: [] });
  await f.search.open(f.opener);
  expect(f.events).toEqual(['mount', 'open']);
  f.search.dispose();
});

it('session/context replacement withdraws providers, closes transport and clears stale navigation', async () => {
  const f = fixture();
  f.search.update('/entry.js', { state: 'ready', items: [] });
  let stopped = 0;
  const provider: SearchProvider = {
    key: 'remote',
    label: 'Remote',
    getSnapshot: () => ({ state: 'ready', items: [] }),
    subscribe: () => () => {
      stopped++;
    },
  };
  f.search.register(provider);
  await f.search.open(f.opener);
  const old = f.options().registry;
  f.search.replace();
  expect(stopped).toBe(1);
  expect(old.groups()).toEqual([]);
  expect(f.events.slice(-2)).toEqual(['dispose', 'remove']);
  f.search.update('/entry.js', { state: 'unavailable' });
  await f.search.open(f.opener);
  expect(f.options().registry.groups()).toEqual([
    { key: 'navigation', label: 'Navigation', state: 'unavailable', items: [] },
  ]);
  f.search.dispose();
});

it('failure is visible without retrying the import or starting a session', async () => {
  let loads = 0;
  const f = fixture(async () => {
    loads++;
    throw new Error('missing');
  });
  f.search.update('/missing.js', { state: 'unavailable' });
  await f.search.open(f.opener);
  expect(f.events).toEqual(['Search could not load. Reload this page to try again.', 'focus']);
  await f.search.open(f.opener);
  expect(loads).toBe(1);
  f.search.dispose();
});

it('unavailable entry and root disposal acquire nothing and ignore pending opens', async () => {
  const f = fixture();
  await f.search.open(f.opener);
  expect(f.events).toEqual(['Search is not available until this door describes Colab.']);
  f.search.dispose();
  await f.search.open(f.opener);
  expect(f.events).toHaveLength(1);
});
