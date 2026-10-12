import assert from 'node:assert/strict';
import { test, vi } from 'vite-plus/test';

// These minimal DOM fixtures own management access/copy; the static registry/menu is
// exercised against the real browser/IndexedDB in pairing.spec.ts.
vi.mock('../src/remote-registry-page.js', () => ({ remoteRegistryPage: () => () => {} }));

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
          'This device already has 8 open sessions, the limit. Close another Remote tab, or change the limit with tmt remote settings sessions-per-device <n> (or off).';
        assert.equal(nodes.get('access')!.textContent, copy);
        assert.equal(nodes.get('access-announcement')!.textContent, copy);
        assert.equal(nodes.get('controls-reason')!.textContent, copy);
        assert.ok(
          ![...nodes.values()].some((item) => /Access ended|access refused/.test(item.textContent)),
        );
      } else if (['REMOTE_CLOSED', 'REMOTE_GRANT_CHANGED', 'REMOTE_SERVE_STOPPED'].includes(code)) {
        assert.equal(
          nodes.get('access')!.textContent,
          'Current browser access unconfirmed. Pair locally with tmt remote pair, or use the local CLI.',
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
      'This device already has 8 open sessions, the limit. Close another Remote tab, or change the limit with tmt remote settings sessions-per-device <n> (or off).';
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
