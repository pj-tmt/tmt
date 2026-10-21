import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test, vi } from 'vite-plus/test';

// These minimal DOM fixtures own management access/copy; the static registry/menu is
// exercised against the real browser/IndexedDB in pairing.spec.ts.
vi.mock('../src/remote-registry-page.js', () => ({ remoteRegistryPage: () => () => {} }));
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
      getElementById: (id: string) => {
        if (!nodes.has(id)) nodes.set(id, node());
        return nodes.get(id);
      },
      querySelectorAll: () => [],
      createElement: node,
      createTextNode: (textContent: string) => ({ textContent }),
    });
    try {
      await import('../src/settings-page.js');
      if (code === 'REMOTE_SESSION_LIMIT') {
        const copy =
          "This browser has reached the owner's connection limit. Close another tab, or ask the owner to change the limit. Your pairing is unchanged.";
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
    await import('../src/settings-page.js');
    assert.deepEqual(options, [undefined, { retry: 'bounded' }]);
    const copy =
      "This browser has reached the owner's connection limit. Close another tab, or ask the owner to change the limit. Your pairing is unchanged.";
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
  textContent = '';
  className = '';
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
      selector === '[data-remote]' ? 'remote' in node.dataset : node.tag === selector,
    );
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
  for (const id of ['remotes', 'remote-list', 'remote-status', 'remote-add']) {
    const node = new Element(id === 'remote-add' ? 'button' : 'section', page);
    ids.set(id, node);
    body.append(node);
  }
  vi.stubGlobal('document', {
    getElementById: (id: string) => ids.get(id) ?? null,
    createElement: (tag: string) => new Element(tag, page),
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
  const { remoteRegistryPage } = await vi.importActual<
    typeof import('../src/remote-registry-page.js')
  >('../src/remote-registry-page.js');
  remoteRegistryPage();
  const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
  await settle();
  const dialogs = () => ids.get('remotes')!.querySelectorAll('dialog');
  const title = (dialog: Element) => dialog.children[0]!.textContent;
  const entry = { name: 'alpha', kind: 'local', url: 'https://door.test/', project: null };
  return {
    page,
    add: ids.get('remote-add')!,
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
