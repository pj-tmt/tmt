import { afterEach, expect, it, vi } from 'vite-plus/test';
import { remoteMenu } from '../src/remote-menu.js';
import { registryView } from '../src/remote-registry.js';
import type { RemoteShell } from '../src/remote-shell.js';

afterEach(() => vi.unstubAllGlobals());

it('requests the live description only for the explicit connected projection with an address', () => {
  const pageUrl = 'https://door.test/remote/';
  const address = 'https://door.test/r/abcdefghijklmnop';
  vi.stubGlobal('location', new URL(pageUrl));
  vi.stubGlobal('window', { addEventListener() {} });
  vi.stubGlobal('document', { addEventListener() {} });
  const described: (string | undefined)[] = [];
  const shell: RemoteShell = {
    setMenu() {},
    navigate() {},
    setTitle() {},
    described() {},
    describe: (address) => described.push(address),
    signal: new AbortController().signal,
  };
  const observe = remoteMenu({ refresh: async () => {}, recover() {} }, shell);
  for (const state of ['Checking', 'Unconfirmed', 'Different machine', 'Not paired'])
    observe(registryView({ version: 1, entries: [] }, { pageUrl, state, address }));
  observe(registryView({ version: 1, entries: [] }, { pageUrl, state: 'Connected' }));
  expect(described).toEqual([]);
  observe(registryView({ version: 1, entries: [] }, { pageUrl, state: 'Connected', address }));
  expect(described).toEqual([address]);
});
