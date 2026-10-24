import { afterEach, beforeEach, expect, it, vi } from 'vite-plus/test';
import type { ShellAnswer, ShellMenu } from '@tmt/browser-ui/shell';
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
  get firstElementChild(): Node | undefined {
    return this.children[0];
  }
  append(...nodes: (Node | string)[]) {
    this.children.push(
      ...nodes.map((node) => {
        if (typeof node !== 'string') return node;
        const text = new Node('#text');
        text.textContent = node;
        return text;
      }),
    );
  }
  prepend(...nodes: Node[]) {
    this.children.unshift(...nodes);
  }
  replaceChildren(...nodes: (Node | string)[]) {
    this.own = '';
    this.children = [];
    this.append(...nodes);
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
  for (const id of ['tmt-settings-section', 'tmt-settings-note', 'tmt-settings-page'])
    ids.set(id, new Node('div'));
  vi.stubGlobal('document', {
    createElement: (tag: string) => new Node(tag),
    createElementNS: (_space: string, tag: string) => new Node(tag),
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
    .all((node) =>
      node.className
        .split(' ')
        .some((name) => ['tmt-ui-list-label', 'tmt-settings-identity-name'].includes(name)),
    )
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
  const detail = section().all((node) => node.className === 'tmt-ui-list-description');
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
  expect(section().textContent).toContain('Identities could not be read.');
  const mock = await mockSettingsSource.read();
  expect(mock.sample).toBe(true);
  tmtSettingsPage(shell(), mockSettingsSource);
  await settle();
  expect(note().hidden).toBe(true);
  expect(section().textContent).toContain('Sample identities');
  expect(section().textContent).toContain('These sample records are not this machine.');
});

it('reports a failed read and stops painting once cleaned up', async () => {
  let release!: (settings: TmtSettings) => void;
  tmtSettingsPage(shell(), { read: () => new Promise((resolve) => (release = resolve)) });
  // Leaving the route ends the page on its signal.
  route.abort();
  expect(removed).toBe(1);
  release(sample);
  await settle();
  // Only the wait painted before the route ended; the late read paints nothing.
  expect(section().children.map((child) => child.className)).toEqual(['tmt-page-loading']);
  route = new AbortController();
  tmtSettingsPage(shell(), { read: () => Promise.reject(new Error('no read')) });
  await settle();
  expect(note().textContent).toBe('Settings could not be read.');
  expect(note().hidden).toBe(false);
  // A failed read ends the wait.
  expect(section().children).toEqual([]);
});

it('leaves a page without its markup alone', () => {
  ids.clear();
  tmtSettingsPage(shell(), { read: async () => sample });
  expect(menus).toEqual([]);
});

it('General follows the approved appearance and terminal-card hierarchy without reference values', async () => {
  tmtSettingsPage(shell());
  await settle();
  expect(section().textContent).toContain('Appearance');
  expect(section().textContent).toContain('Core configuration');
  expect(section().textContent).toContain('This browser cannot read Core settings yet.');
  expect(section().textContent).toContain('View machine configuration in your terminal.');
  expect(section().textContent).toContain('Browser appearance');
  expect(section().textContent).not.toContain('Reference defaults');
  expect(section().textContent).not.toContain('5000');
  expect(section().textContent).not.toContain('2000');
});

it('General keeps non-default source values and specific refusal details', async () => {
  const injected: TmtSettings = {
    ...sample,
    general: {
      rows: [{ name: 'Injected setting', value: '19 s', description: 'Observed test value' }],
    },
    sample: true,
  };
  tmtSettingsPage(shell(), { read: async () => injected });
  await settle();
  expect(section().textContent).toContain('19 s');
  expect(note().textContent).toContain('Sample data');
  route.abort();
  route = new AbortController();
  tmtSettingsPage(shell(), {
    read: async () => ({ ...sample, general: { rows: [], unavailable: 'Permission refused.' } }),
  });
  await settle();
  expect(section().textContent).toContain('Permission refused.');
});

for (const denied of [false, true])
  it(`General command Copy reports ${denied ? 'denial' : 'success'} without executing it`, async () => {
    const writeText = vi.fn(() =>
      denied ? Promise.reject(new Error('denied')) : Promise.resolve(),
    );
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    tmtSettingsPage(shell());
    await settle();
    const copy = section().all(
      (node) => node.attributes.get('aria-label') === 'Copy tmt config show',
    )[0]!;
    copy.fire('click');
    await settle();
    expect(writeText).toHaveBeenCalledExactlyOnceWith('tmt config show');
    expect(section().textContent).toContain(
      denied ? 'Copy failed. Select the command and copy it manually.' : 'Copied.',
    );
  });

it('renders Identities with the approved inventory and local selected detail', async () => {
  hash = '#identities';
  const read = vi.fn(async () => sample);
  tmtSettingsPage(shell(), { read });
  await settle();
  expect(section().textContent).toContain('Identity management');
  expect(section().textContent).toContain('Lifetime');
  expect(section().textContent).toContain('Presence');
  const names = section().all((node) =>
    node.className.split(' ').includes('tmt-settings-identity-name'),
  );
  expect(names.map((node) => node.textContent)).toEqual(['alpha', 'beta', 'zed']);
  expect(names[0]!.attributes.get('aria-current')).toBe('true');
  names[1]!.fire('click');
  expect(names[1]!.attributes.get('aria-current')).toBe('true');
  expect(names[0]!.attributes.get('aria-current')).toBe('false');
  expect(read).toHaveBeenCalledTimes(1);
});

it('keeps Identities unavailable and empty states distinct without invented records', async () => {
  hash = '#identities';
  tmtSettingsPage(shell());
  await settle();
  expect(section().textContent).toContain('Web data connection pending');
  expect(section().textContent).toContain('Identity inventory unavailable');
  expect(section().all((node) => node.tag === 'tbody')).toHaveLength(0);
  expect(
    section()
      .all((node) => node.tag === 'code')
      .map((node) => node.textContent),
  ).toEqual(['tmt ls']);
  expect(note().hidden).toBe(true);
  route.abort();
  route = new AbortController();
  tmtSettingsPage(shell(), { read: async () => ({ ...sample, identities: { rows: [] } }) });
  await settle();
  expect(section().textContent).toContain('No identities yet.');
  expect(section().textContent).not.toContain('Web data connection pending');
});

it('uses product provider labels, a truthful missing value and one source read for local selection', async () => {
  hash = '#identities';
  const read = vi.fn(async () => sample);
  tmtSettingsPage(shell(), { read });
  await settle();
  expect(section().textContent).toContain('Codex');
  expect(section().textContent).toContain('Claude');
  expect(section().textContent).toContain('Not reported');
  expect(section().textContent).not.toContain('codex');
  expect(section().textContent).not.toContain('claude');
  const names = section().all((node) =>
    node.className.split(' ').includes('tmt-settings-identity-name'),
  );
  names[1]!.fire('click');
  const selected = section().all((node) => node.tag === 'tr' && 'selected' in node.dataset);
  expect(selected).toHaveLength(1);
  expect(selected[0]!.textContent).toContain('beta');
  const detail = section().all((node) => node.tag === 'aside')[0]!;
  expect(detail.textContent).toContain('Temporary');
  expect(detail.textContent).toContain('Codex');
  expect(detail.textContent).toContain('Active');
  names[2]!.fire('click');
  expect(detail.textContent).toContain('Saved');
  expect(detail.textContent).toContain('Offline');
  expect(detail.textContent).toContain('Not reported');
  expect(read).toHaveBeenCalledTimes(1);
});

it('renders remaining sample command references as code when changing sections', async () => {
  tmtSettingsPage(shell(), mockSettingsSource);
  await settle();
  expect(note().hidden).toBe(false);
  expect(note().textContent).not.toContain('`');
  expect(
    note()
      .all((node) => node.tag === 'code')
      .map((node) => node.textContent),
  ).toEqual(['tmt ls', 'tmt extension ls', 'tmt driver ls']);
  hash = '#identities';
  hashchange!();
  expect(note().hidden).toBe(true);
  expect(note().textContent).toBe('');
});

it('keeps a supplied warning beside retained identity rows', async () => {
  hash = '#identities';
  tmtSettingsPage(shell(), {
    read: async () => ({
      ...sample,
      identities: { ...sample.identities, unavailable: 'Some identity records were refused.' },
    }),
  });
  await settle();
  expect(section().textContent).toContain('Some identity records were refused.');
  expect(rowTitles()).toEqual(['alpha', 'beta', 'zed']);
});
