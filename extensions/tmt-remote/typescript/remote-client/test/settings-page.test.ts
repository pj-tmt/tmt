import assert from 'node:assert/strict';
import { test, vi } from 'vite-plus/test';

for (const code of [
  'REMOTE_SESSION_LIMIT',
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
