import { afterEach, beforeEach, expect, it, vi } from 'vite-plus/test';
import type { ShellAnswer, ShellMenu } from '../../../../../design/browser-ui/src/shell.js';
import { moduleLinks, settingsRows, tmtSettingsPage } from '../src/tmt-settings-page.js';
import type { RemoteShell } from '../src/remote-shell.js';
import { mockSettingsSource, type TmtSettings } from '../src/settings-source.js';

// A minimal DOM: only what the page builds and reads, enough to click, key and read text.
class Node {
  children: Node[] = [];
  dataset: Record<string, string> = {};
  attributes = new Map<string, string>();
  listeners = new Map<string, ((event: unknown) => void)[]>();
  className = '';
  hidden = false;
  href = '';
  title = '';
  type = '';
  tabIndex = 0;
  focused = false;
  private own = '';
  constructor(readonly tag: string) {}
  get textContent(): string {
    return this.own + this.children.map((child) => child.textContent).join('');
  }
  set textContent(value: string) {
    this.own = value;
    this.children = [];
  }
  append(...nodes: Node[]) {
    this.children.push(...nodes);
  }
  replaceChildren(...nodes: Node[]) {
    this.own = '';
    this.children = nodes;
  }
  setAttribute(name: string, value: string) {
    this.attributes.set(name, value);
  }
  removeAttribute(name: string) {
    this.attributes.delete(name);
  }
  addEventListener(type: string, listener: (event: unknown) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }
  focus() {
    this.focused = true;
  }
  fire(type: string, event: object = {}) {
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }
  all(match: (node: Node) => boolean): Node[] {
    return [this, ...this.children.flatMap((child) => child.all(match))].filter(match);
  }
}

const ids = new Map<string, Node>();
let title: Node;
let root: Node;
let hash = '';
let hashchange: (() => void) | undefined;
let removed = 0;
let stored = new Map<string, string>();
let menus: ShellMenu[] = [];
let described: ((answer: ShellAnswer | undefined) => void) | undefined;

let route: AbortController;
const shell = () =>
  ({
    setMenu: (menu: ShellMenu) => void menus.push(menu),
    describe: () => {},
    described: (listener: (answer: ShellAnswer | undefined) => void) => {
      described = listener;
    },
    signal: route.signal,
  }) as unknown as RemoteShell;

beforeEach(() => {
  route = new AbortController();
  ids.clear();
  menus = [];
  described = undefined;
  hash = '';
  removed = 0;
  stored = new Map();
  hashchange = undefined;
  title = new Node('h1');
  root = new Node('html');
  for (const id of ['tmt-settings-section', 'tmt-settings-note']) ids.set(id, new Node('div'));
  vi.stubGlobal('document', {
    createElement: (tag: string) => new Node(tag),
    getElementById: (id: string) => ids.get(id) ?? null,
    querySelector: () => title,
    documentElement: root,
  });
  vi.stubGlobal('location', {
    pathname: '/settings',
    get hash() {
      return hash;
    },
  });
  vi.stubGlobal('window', {
    addEventListener: (_type: string, listener: () => void, options?: { signal?: AbortSignal }) => {
      hashchange = listener;
      options?.signal?.addEventListener('abort', () => (removed += 1));
    },
  });
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => void stored.set(key, value),
    removeItem: (key: string) => void stored.delete(key),
  });
});
afterEach(() => vi.unstubAllGlobals());

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const section = () => ids.get('tmt-settings-section')!;
const note = () => ids.get('tmt-settings-note')!;
const rowTitles = () =>
  section()
    .all((node) => node.className === 'tmt-settings-label')
    .map((node) => node.textContent);

const sample: TmtSettings = {
  sample: false,
  general: {
    rows: [{ name: 'Upgrade channel', value: 'alpha', description: 'Where upgrades look' }],
  },
  identities: {
    rows: [
      { name: 'zed', lifetime: 'saved', online: false },
      { name: 'beta', lifetime: 'temporary', online: true, provider: 'codex' },
      { name: 'alpha', lifetime: 'saved', online: true, provider: 'claude' },
    ],
  },
  modules: {
    rows: [
      { name: 'colab', installed: true, version: '0.1.0', channel: 'alpha' },
      { name: 'office', installed: false },
    ],
  },
  drivers: { rows: [{ name: 'herdr', version: '0.2.0', state: 'binary changed' }] },
  hooks: { rows: [{ provider: 'claude', usage: false }] },
};

it('lists live agents first, then by name, with provider and word', () => {
  expect(settingsRows('identities', sample)).toEqual([
    { mark: '●', title: 'alpha', detail: 'saved · claude', state: 'active' },
    { mark: '●', title: 'beta', detail: 'temporary · codex', state: 'active' },
    { mark: '○', title: 'zed', detail: 'saved', state: 'offline' },
  ]);
});

it('links an installed module to the settings page the composed shell describes', () => {
  const answer: ShellAnswer = {
    version: 1,
    modules: [
      {
        extension: 'colab',
        rail: { label: 'Colab', icon: 'colab', path: '/colab/', order: 1 },
        menu: [],
        settings: [{ id: 'settings', label: 'Colab settings', path: '/colab/settings' }],
      },
    ],
  };
  expect(settingsRows('modules', sample, moduleLinks(answer))).toEqual([
    {
      mark: '●',
      title: 'Colab',
      href: '/colab/settings',
      detail: '0.1.0 · alpha',
      state: 'installed',
    },
    { mark: '○', title: 'Office', detail: 'Not installed', state: '' },
  ]);
  // Without a description the rows still list, unlinked.
  expect(settingsRows('modules', sample)[0]).not.toHaveProperty('href');
});

it('marks a driver that no longer runs as approved, and shows hooks beside drivers', () => {
  expect(settingsRows('drivers', sample)).toEqual([
    {
      mark: '◆',
      waiting: true,
      title: 'herdr',
      detail: 'Host driver 0.2.0',
      state: 'binary changed',
    },
    {
      mark: '●',
      title: 'claude',
      detail: 'Agent hooks · usage collection off',
      state: 'set up',
    },
  ]);
});

it('renders General with the theme choice, stores a pick and applies it to the root', async () => {
  tmtSettingsPage(shell(), { read: async () => sample });
  await settle();
  expect(title.textContent).toBe('General');
  const tabs = section().all((node) => node.attributes.get('role') === 'tab');
  expect(tabs.map((tab) => tab.textContent)).toEqual(['System', 'Light', 'Dark']);
  // `data-theme` scopes the design tokens, so a tab must not carry one.
  expect(tabs.every((tab) => !('theme' in tab.dataset))).toBe(true);
  expect(tabs.map((tab) => tab.attributes.get('aria-selected'))).toEqual([
    'true',
    'false',
    'false',
  ]);
  tabs[2]!.fire('click');
  expect(stored.get('tmt-ui-theme')).toBe('dark');
  expect(root.attributes.get('data-theme')).toBe('dark');
  expect(tabs.map((tab) => tab.attributes.get('aria-selected'))).toEqual([
    'false',
    'false',
    'true',
  ]);
  // Arrow keys move the choice and its focus.
  const prevent = vi.fn();
  tabs[2]!.fire('keydown', { key: 'ArrowRight', preventDefault: prevent });
  expect(prevent).toHaveBeenCalled();
  expect(root.attributes.has('data-theme')).toBe(false);
  expect(stored.has('tmt-ui-theme')).toBe(false);
  expect(tabs[0]!.focused).toBe(true);
  expect(rowTitles()).toEqual(['Upgrade channel']);
  // The description sits under the label and the value in the state cell.
  const detail = section().all((node) => node.className === 'tmt-settings-detail');
  expect(detail.map((node) => node.textContent)).toEqual(['Where upgrades look']);
  expect(
    section()
      .all((node) => node.className === 'tmt-ui-list-state')
      .map((node) => node.textContent),
  ).toEqual(['alpha']);
  expect(note().hidden).toBe(true);
});

it('offers the four sections as the module menu and follows the hash', async () => {
  tmtSettingsPage(shell(), { read: async () => sample });
  await settle();
  expect(menus.at(-1)!.groups[0]!.items.map((item) => [item.label, item.current])).toEqual([
    ['General', true],
    ['Identities', false],
    ['Modules', false],
    ['Drivers & hooks', false],
  ]);
  hash = '#identities';
  hashchange!();
  expect(title.textContent).toBe('Identities');
  expect(rowTitles()).toEqual(['alpha', 'beta', 'zed']);
  // The composed shell answers later: the module rows then link.
  hash = '#modules';
  described!({
    version: 1,
    modules: [
      {
        extension: 'colab',
        rail: { label: 'Colab', icon: 'colab', path: '/colab/', order: 1 },
        menu: [],
        settings: [{ id: 's', label: 'Settings', path: '/colab/settings' }],
      },
    ],
  });
  const link = section().all((node) => node.tag === 'a')[0]!;
  expect([link.textContent, link.href]).toEqual(['Colab', '/colab/settings']);
});

it('says why a section is empty, and marks sample data', async () => {
  const empty: TmtSettings = {
    ...sample,
    identities: { rows: [], unavailable: 'Identities could not be read.' },
  };
  tmtSettingsPage(shell(), { read: async () => empty });
  await settle();
  hash = '#identities';
  hashchange!();
  expect(section().textContent).toBe('Identities could not be read.');
  const mock = await mockSettingsSource.read();
  expect(mock.sample).toBe(true);
  tmtSettingsPage(shell(), mockSettingsSource);
  await settle();
  expect(note().hidden).toBe(false);
  expect(note().textContent).toContain('Sample data');
});

it('reports a failed read and stops painting once cleaned up', async () => {
  let release!: (settings: TmtSettings) => void;
  tmtSettingsPage(shell(), { read: () => new Promise((resolve) => (release = resolve)) });
  // Leaving the route ends the page on its signal.
  route.abort();
  expect(removed).toBe(1);
  release(sample);
  await settle();
  expect(section().children).toEqual([]);
  route = new AbortController();
  tmtSettingsPage(shell(), { read: () => Promise.reject(new Error('no read')) });
  await settle();
  expect(note().textContent).toBe('Settings could not be read.');
  expect(note().hidden).toBe(false);
});

it('leaves a page without its markup alone', () => {
  ids.clear();
  tmtSettingsPage(shell(), { read: async () => sample });
  expect(menus).toEqual([]);
});
