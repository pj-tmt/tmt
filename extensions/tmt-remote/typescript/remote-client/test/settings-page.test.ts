import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test, vi } from 'vite-plus/test';

// These minimal DOM fixtures own management access/copy; the static registry/menu is
// exercised against the real browser/IndexedDB in pairing.spec.ts.
vi.mock('../src/remote-registry-page.js', () => ({
  remoteRegistryPage: () => Object.assign(() => {}, { blockedBy: () => {} }),
}));
vi.mock('../src/settings-controls.js', () => ({ settingsChoice: () => () => {} }));

for (const code of [
  'REMOTE_CLOSED',
  'REMOTE_GRANT_CHANGED',
  'REMOTE_SERVE_STOPPED',
  'REMOTE_SESSION_LIMIT',
  'REMOTE_DEVICE_REVOKED',
  'REMOTE_GRANT_EXPIRED',
  'REMOTE_SESSION_ENDED',
  'REMOTE_SESSION_EVICTED',
] as const) {
  test(`settings fresh-open ${code} does not confuse capacity with ended access`, async () => {
    vi.resetModules();
    const sdk = await import('../src/browser.js');
    const error = new sdk.RefusalError(code, undefined, 8);
    vi.doMock('remote-browser-sdk', () => ({
      ...sdk,
      reopenSession: async () => {
        throw error;
      },
    }));
    function node() {
      let text = '';
      return {
        dataset: {} as Record<string, string>,
        disabled: false,
        get textContent() {
          return text;
        },
        set textContent(value: string) {
          text = value;
        },
        addEventListener() {},
        replaceChildren() {
          text = '';
        },
        append(child: { textContent: string }) {
          text += child.textContent;
        },
      };
    }
    const nodes = new Map<string, ReturnType<typeof node>>();
    vi.stubGlobal('document', {
      addEventListener() {},
      getElementById: (id: string) => {
        if (!nodes.has(id)) nodes.set(id, node());
        return nodes.get(id);
      },
      querySelectorAll: () => [],
      createElement: node,
      createTextNode: (textContent: string) => ({ textContent }),
    });
    try {
      await (await import('../src/settings-page.js')).settingsPage();
      if (code === 'REMOTE_SESSION_LIMIT') {
        const copy =
          "This browser has reached your machine's connection limit. Close another tab, or change the limit on your machine. Your pairing is unchanged.";
        assert.equal(nodes.get('access')!.textContent, copy);
        assert.equal(nodes.get('access-announcement')!.textContent, copy);
        assert.equal(nodes.get('controls-reason')!.textContent, copy);
        assert.ok(
          ![...nodes.values()].some((item) => /Access ended|access refused/.test(item.textContent)),
        );
      } else if (['REMOTE_CLOSED', 'REMOTE_GRANT_CHANGED', 'REMOTE_SERVE_STOPPED'].includes(code)) {
        assert.equal(
          nodes.get('access')!.textContent,
          "Remote may have stopped, or its reply couldn't be verified. Your pairing is unchanged.",
        );
        assert.equal(nodes.get('access-announcement')!.textContent, 'Access unconfirmed');
      } else {
        assert.equal(
          nodes.get('access')!.textContent,
          'Current browser access refused. Use the local CLI.',
        );
        assert.equal(nodes.get('access-announcement')!.textContent, 'Access ended');
      }
      assert.equal(nodes.get('refresh')!.disabled, true);
    } finally {
      vi.doUnmock('remote-browser-sdk');
      vi.unstubAllGlobals();
      vi.resetModules();
    }
  });
}

test('settings refresh uses bounded admission and renders its verified capacity action', async () => {
  vi.resetModules();
  const sdk = await import('../src/browser.js');
  const cause = new sdk.RefusalError('REMOTE_SESSION_LIMIT', undefined, 8);
  const options: unknown[] = [];
  vi.doMock('remote-browser-sdk', () => ({
    ...sdk,
    reopenSession: async (_previous: unknown, policy: unknown) => {
      options.push(policy);
      if (options.length === 2)
        throw new sdk.ReopenSessionError('capacity', 'admission-refused', cause);
      return {};
    },
    management: () => ({
      settings: async () => {
        throw new sdk.RefusalError('REMOTE_SESSION_ENDED');
      },
    }),
  }));
  vi.doMock('../src/firestore-page.js', () => ({ renderFirestore() {} }));
  function node() {
    let text = '';
    const children = new Map<
      string,
      {
        dataset: Record<string, string>;
        disabled: boolean;
        hidden: boolean;
        textContent: string;
        getAttribute(): null;
        setAttribute(): void;
        replaceChildren(): void;
        append(child: { textContent: string }): void;
      }
    >();
    return {
      dataset: {} as Record<string, string>,
      disabled: false,
      hidden: false,
      setAttribute() {},
      removeAttribute() {},
      getAttribute: () => null,
      addEventListener() {},
      checkValidity: () => true,
      querySelector(selector: string) {
        if (!children.has(selector)) children.set(selector, node());
        return children.get(selector)!;
      },
      get textContent() {
        return text;
      },
      set textContent(value: string) {
        text = value;
      },
      replaceChildren() {
        text = '';
      },
      append(child: { textContent: string }) {
        text += child.textContent;
      },
    };
  }
  const nodes = new Map<string, ReturnType<typeof node>>();
  const feedback = node();
  feedback.dataset.feedback = 'shared';
  vi.stubGlobal('document', {
    addEventListener() {},
    getElementById: (id: string) => {
      if (!nodes.has(id)) nodes.set(id, node());
      return nodes.get(id);
    },
    querySelectorAll: (selector: string) => (selector === '[data-feedback]' ? [feedback] : []),
    querySelector: () => feedback,
    createElement: node,
    createTextNode: (textContent: string) => ({ textContent }),
  });
  try {
    await (await import('../src/settings-page.js')).settingsPage();
    assert.deepEqual(options, [undefined, { retry: 'bounded' }]);
    const copy =
      "This browser has reached your machine's connection limit. Close another tab, or change the limit on your machine. Your pairing is unchanged.";
    for (const id of ['access', 'access-announcement', 'controls-reason'])
      assert.equal(nodes.get(id)!.textContent, copy);
    assert.equal(nodes.get('refresh')!.disabled, false);
  } finally {
    vi.doUnmock('remote-browser-sdk');
    vi.doUnmock('../src/firestore-page.js');
    vi.unstubAllGlobals();
    vi.resetModules();
  }
});

// The served document must remain usable while the SDK checks current authority.
test('settings initial choices are named disabled listboxes, without native select or confirm', async () => {
  const html = await readFile(
    new URL('../../../rust/tmt-remote/assets/settings.html', import.meta.url),
    'utf8',
  );
  const entry = await readFile(new URL('../src/settings-page.ts', import.meta.url), 'utf8');
  assert.doesNotMatch(html, /<select\b/);
  assert.doesNotMatch(entry, /\bconfirm\(/);
  for (const id of ['opening', 'limit-mode']) {
    assert.match(html, new RegExp(`id="${id}"[^>]*aria-haspopup="listbox"[^>]*disabled`, 's'));
    assert.match(html, new RegExp(`id="${id}-options"[^>]*popover="auto"[^>]*role="listbox"`, 's'));
  }
});

/** Just the element surface the remote registry page drives: tree, events, dialogs, focus. */
class Element {
  children: Element[] = [];
  parent: Element | undefined;
  connected = false;
  dataset: Record<string, string> = {};
  #text = '';
  get textContent(): string {
    return this.#text + this.children.map((child) => child.textContent).join('');
  }
  set textContent(value: string) {
    this.#text = value;
    for (const child of this.children) child.parent = undefined;
    this.children = [];
  }
  className = '';
  style = { setProperty() {} };
  type = '';
  value = '';
  title = '';
  id = '';
  disabled = false;
  open = false;
  #listeners: { type: string; run: () => void; once: boolean }[] = [];
  constructor(
    readonly tag: string,
    readonly page: { focused?: Element },
  ) {}
  get isConnected(): boolean {
    return this.connected || !!this.parent?.isConnected;
  }
  append(...children: Element[]) {
    for (const child of children) {
      child.parent = this;
      this.children.push(child);
    }
  }
  replaceWith(other: Element) {
    if (!this.parent) return;
    const parent = this.parent;
    const index = parent.children.indexOf(this);
    parent.children[index] = other;
    other.parent = parent;
    this.parent = undefined;
  }
  after(other: Element) {
    if (!this.parent) return;
    const parent = this.parent;
    parent.children.splice(parent.children.indexOf(this) + 1, 0, other);
    other.parent = parent;
  }
  prepend(child: Element) {
    child.remove();
    child.parent = this;
    this.children.unshift(child);
  }
  get childNodes() {
    return this.children;
  }
  replaceChildren(...children: Element[]) {
    for (const child of this.children) child.parent = undefined;
    this.children = [];
    this.append(...children);
  }
  remove() {
    this.parent?.children.splice(this.parent.children.indexOf(this), 1);
    this.parent = undefined;
  }
  setAttribute() {}
  addEventListener(type: string, run: () => void, options?: { once?: boolean }) {
    this.#listeners.push({ type, run, once: !!options?.once });
  }
  #dispatch(type: string) {
    for (const listener of this.#listeners.filter((entry) => entry.type === type)) {
      if (listener.once) this.#listeners.splice(this.#listeners.indexOf(listener), 1);
      listener.run();
    }
  }
  click() {
    if (!this.disabled) this.#dispatch('click');
  }
  focus() {
    this.page.focused = this;
  }
  showModal() {
    this.open = true;
  }
  close() {
    if (!this.open) return;
    this.open = false;
    this.#dispatch('close');
  }
  scrollIntoView() {}
  descendants(): Element[] {
    return this.children.flatMap((child) => [child, ...child.descendants()]);
  }
  querySelectorAll(selector: string): Element[] {
    return this.descendants().filter((node) =>
      selector === '[data-summary=members]'
        ? node.dataset.summary === 'members'
        : selector === '[data-summary=active]'
          ? node.dataset.summary === 'active'
          : selector === '[data-remote]'
            ? 'remote' in node.dataset
            : selector === '[data-role]'
              ? 'role' in node.dataset
              : selector === '[data-action="open"]'
                ? node.dataset.action === 'open'
                : selector.startsWith('.')
                  ? node.className.split(' ').includes(selector.slice(1))
                  : node.tag === selector,
    );
  }
  querySelector(selector: string): Element | undefined {
    return this.querySelectorAll(selector)[0];
  }
  button(text: string): Element {
    return this.querySelectorAll('button').find((node) => node.textContent === text)!;
  }
}

async function registryPage() {
  vi.resetModules();
  const page: { focused?: Element } = {};
  const body = new Element('body', page);
  body.connected = true;
  const ids = new Map<string, Element>();
  for (const id of ['remotes', 'remote-list', 'remote-status', 'remote-join', 'remote-head']) {
    const node = new Element('section', page);
    ids.set(id, node);
    body.append(node);
  }
  const headActions = new Element('div', page);
  headActions.className = 'remote-head-actions';
  ids.get('remote-head')!.append(headActions);
  let tableBody: Element | undefined;
  vi.stubGlobal('document', {
    body,
    getElementById: (id: string) => ids.get(id) ?? null,
    createElement: (tag: string) => new Element(tag, page),
    createElementNS: (_namespace: string, tag: string) => new Element(tag, page),
    documentElement: { dataset: { tmtPage: 'settings' } },
  });
  vi.stubGlobal('location', {
    hash: '',
    href: 'https://remote.test/settings',
    pathname: '/settings',
    search: '',
    assign() {},
  });
  vi.stubGlobal('history', { replaceState() {} });
  vi.stubGlobal('window', { addEventListener() {} });
  const registry = await vi.importActual<typeof import('../src/remote-registry.js')>(
    '../src/remote-registry.js',
  );
  const validations: { resolve(entry: unknown): void; reject(error: unknown): void }[] = [];
  const added: string[] = [];
  vi.doMock('../src/remote-registry.js', () => ({
    ...registry,
    reference: () =>
      new Promise((resolve, reject) => {
        validations.push({ resolve, reject });
      }),
    remoteRegistry: {
      list: async () => ({ version: 1, entries: [] }),
      add: async (name: string) => {
        added.push(name);
      },
    },
  }));
  vi.doMock('../src/remote-menu.js', () => ({ remoteMenu: () => () => {} }));
  vi.doMock('@tmt/browser-ui/list-table', () => ({
    createListTable: () => {
      const root = new Element('div', page);
      tableBody = new Element('div', page);
      root.append(tableBody);
      return {
        root,
        body: tableBody,
        row: (cells: Element[]) => {
          const row = new Element('div', page);
          for (const cell of cells) {
            const wrapper = new Element('div', page);
            wrapper.append(cell);
            row.append(wrapper);
          }
          return row;
        },
      };
    },
    createPageSubheader: ({ actions }: { actions: Element }) => {
      const root = new Element('header', page);
      root.className = 'tmt-ui-subheader';
      const count = new Element('span', page);
      count.className = 'tmt-ui-shell-count';
      root.append(count, actions);
      return root;
    },
  }));

  // Menu interactions/keyboard/focus use the real shared host in Chromium; this fixture owns
  // the registry dialog's validation lifetime and observation updates only.
  vi.doMock('@tmt/browser-ui/menu-host', () => ({
    mountBrowserMenu: (
      container: HTMLElement,
      options: import('@tmt/browser-ui/menu-host').BrowserMenuHostOptions<string>,
    ) => {
      const trigger = document.createElement('button');
      trigger.textContent = options.triggerText ?? options.label;
      container.append(trigger);
      for (const item of options.items) {
        const choice = document.createElement('button');
        choice.textContent = item.label;
        choice.addEventListener('click', () =>
          options.onSelect(item.key, new Event('click') as MouseEvent),
        );
        container.append(choice);
      }
      return {
        open() {},
        close() {
          trigger.focus();
        },
        update() {},
        dispose() {
          container.replaceChildren();
        },
      };
    },
  }));
  const { remoteRegistryPage } = await vi.importActual<
    typeof import('../src/remote-registry-page.js')
  >('../src/remote-registry-page.js');
  const lifetime = new AbortController();
  const observe = remoteRegistryPage(undefined, {
    signal: lifetime.signal,
    setMenu() {},
    navigate() {},
    setTitle() {},
    describe() {},
    described() {},
  });
  const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
  await settle();
  const dialogs = () => body.querySelectorAll('dialog');
  const title = (dialog: Element) => dialog.children[0]!.children[0]!.textContent;
  const entry = { name: 'alpha', kind: 'local', url: 'https://door.test/', project: null };
  return {
    page,
    list: tableBody!,
    observe,
    depart: () => lifetime.abort(),
    add: ids.get('remote-join')!.button('Remember a Remote'),
    dialogs,
    title,
    validations,
    added,
    settle,
    entry,
    registry,
  };
}
function cleanUp() {
  vi.doUnmock('../src/remote-registry.js');
  vi.doUnmock('../src/remote-menu.js');
  vi.doUnmock('@tmt/browser-ui/list-table');
  vi.doUnmock('@tmt/browser-ui/menu-host');
  vi.unstubAllGlobals();
  vi.resetModules();
}

test('an Add validation that settles after its dialog was cancelled never touches the new opening', async () => {
  const view = await registryPage();
  try {
    view.add.click();
    const first = view.dialogs()[0]!;
    first.button('Continue').click();
    first.button('Cancel').click();
    view.add.click();
    const second = view.dialogs()[0]!;
    const name = second.querySelectorAll('input')[0]!;
    name.value = 'second';
    name.focus();
    view.validations[0]!.resolve(view.entry);
    await view.settle();
    assert.deepEqual(view.dialogs(), [second]);
    assert.equal(second.open, true);
    assert.equal(view.title(second), 'Add remote');
    assert.equal(name.value, 'second');
    assert.equal(view.page.focused, name);
    assert.deepEqual(view.added, []);
  } finally {
    cleanUp();
  }
});

test('an Add validation on its live dialog still confirms, and its failure stays on that dialog', async () => {
  const view = await registryPage();
  try {
    view.add.click();
    const first = view.dialogs()[0]!;
    first.button('Continue').click();
    view.validations[0]!.reject(
      new view.registry.RemoteRegistryError('REMOTE_SELECTOR_INVALID', 'The link is invalid'),
    );
    await view.settle();
    assert.equal(first.open, true);
    assert.equal(first.querySelectorAll('p')[0]!.textContent, 'The link is invalid');
    first.button('Continue').click();
    view.validations[1]!.resolve(view.entry);
    await view.settle();
    const confirm = view.dialogs()[0]!;
    assert.equal(view.title(confirm), 'Add remote alpha?');
    confirm.button('Add').click();
    await view.settle();
    assert.deepEqual(view.added, ['alpha']);
    assert.deepEqual(view.dialogs(), []);
  } finally {
    cleanUp();
  }
});

test('registry observations retain the focused row control and never guess a role', async () => {
  const view = await registryPage();
  try {
    const row = view.list.children[0]!;
    const open = row.querySelector('[data-action="open"]')!;
    open.focus();
    view.observe('Offline');
    assert.equal(view.list.children[0], row);
    assert.equal(row.querySelector('[data-action="open"]'), open);
    assert.equal(view.page.focused, open);
    assert.equal(row.querySelector('.tmt-ui-list-state')!.textContent, 'Offline');
    assert.equal(row.querySelector('[data-role]')!.textContent, 'Role unknown');
    view.observe('unreported');
    assert.equal(row.querySelector('.tmt-ui-list-state')!.textContent, 'Status unknown');
  } finally {
    cleanUp();
  }
});

test('the current registry row displays only a live observed browser role', async () => {
  const view = await registryPage();
  try {
    view.observe('Connected', undefined, 'owner');
    assert.equal(view.list.children[0]!.querySelector('[data-role]')!.textContent, 'Admin');
    view.observe('Connected', undefined, 'user');
    assert.equal(view.list.children[0]!.querySelector('[data-role]')!.textContent, 'Member');
    view.observe('Connected', undefined, null);
    assert.equal(view.list.children[0]!.querySelector('[data-role]')!.textContent, 'Role unknown');
    view.observe('Connected', undefined, 'owner');
    view.observe('Unconfirmed');
    assert.equal(view.list.children[0]!.querySelector('[data-role]')!.textContent, 'Role unknown');
    view.observe('Connected');
    assert.equal(view.list.children[0]!.querySelector('[data-role]')!.textContent, 'Role unknown');
  } finally {
    cleanUp();
  }
});

test('Remote tab observations reuse the registry role projection and clear it on access loss', async () => {
  const view = await registryPage();
  try {
    const roles: string[] = [];
    const end = view.observe.watch((projection) => roles.push(projection.current.roleLabel));
    view.observe('Connected', undefined, 'owner');
    view.observe('Connected', undefined, 'user');
    view.observe('Unconfirmed');
    assert.deepEqual(roles.slice(-3), ['Admin', 'Member', 'Role unknown']);
    end();
    const length = roles.length;
    view.observe('Connected', undefined, 'owner');
    assert.equal(roles.length, length);
  } finally {
    cleanUp();
  }
});

test('registry dialog departure removes its modal and fences late validation without a write', async () => {
  const view = await registryPage();
  try {
    view.add.click();
    const dialog = view.dialogs()[0]!;
    dialog.button('Continue').click();
    assert.equal(view.validations.length, 1);
    view.depart();
    assert.equal(view.dialogs().length, 0);
    view.validations[0]!.resolve(view.entry);
    await view.settle();
    assert.equal(view.dialogs().length, 0);
    assert.equal(view.added.length, 0);
  } finally {
    cleanUp();
  }
});

test('the current row shows only admitted, non-null summary observations', async () => {
  const view = await registryPage();
  try {
    const row = view.list.children[0]!;
    const members = row.querySelector('[data-summary=members]')!;
    const active = row.querySelector('[data-summary=active]')!;
    view.observe('Connected', undefined, 'owner');
    view.observe.setSummary({
      remoteId: '11111111-1111-4111-8111-111111111111',
      memberCount: 3,
      lastActiveAtMs: null,
    });
    assert.equal(members.textContent, '3');
    assert.equal(active.textContent, '');
    view.observe('Unconfirmed');
    assert.equal(members.textContent, '');
    view.observe.setSummary({
      remoteId: '11111111-1111-4111-8111-111111111111',
      memberCount: 9,
      lastActiveAtMs: 1,
    });
    view.observe('Connected', undefined, 'user');
    assert.equal(members.textContent, '');
    assert.equal(active.textContent, '');
  } finally {
    cleanUp();
  }
});
