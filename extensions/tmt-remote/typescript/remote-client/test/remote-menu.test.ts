import { afterEach, expect, it, vi } from 'vite-plus/test';
import { remoteMenu } from '../src/remote-menu.js';
import { registryView } from '../src/remote-registry.js';
import type { ShellMenu } from '@tmt/browser-ui/shell';
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

function menuAt(pathname: string) {
  vi.stubGlobal('location', new URL(`https://door.test${pathname}`));
  vi.stubGlobal('window', { addEventListener() {} });
  vi.stubGlobal('document', { addEventListener() {} });
  const menus: ShellMenu[] = [];
  const recover = vi.fn();
  const refresh = vi.fn(async () => {});
  const shell: RemoteShell = {
    setMenu: (menu) => {
      if (menu) menus.push(menu);
    },
    navigate() {},
    setTitle() {},
    described() {},
    describe() {},
    signal: new AbortController().signal,
  };
  return { observe: remoteMenu({ refresh, recover }, shell), menus, recover, refresh };
}

it('keeps the reference child list fixed and maps route context to the final settings slot', () => {
  const cases = [
    ['/remote/', 'remotes'],
    ['/remote/create', 'remotes'],
    ['/remote/join', 'remotes'],
    ['/remote/members', 'members'],
    ['/remote/my-devices', 'my-devices'],
    ['/remote/settings', 'settings'],
    ['/remote/service', 'settings'],
    // No extra Overview child: the shared shell selects the current Remote module itself.
    ['/remote/overview', undefined],
  ] as const;
  for (const [path, selected] of cases) {
    const { menus } = menuAt(path);
    const menu = menus.at(-1)!;
    expect(menu.groups.flatMap((group) => group.items).map((item) => item.id)).toEqual([
      'remotes',
      'members',
      'my-devices',
    ]);
    expect(menu.groups[1]!.label).toBe('This machine');
    expect(menu.settings).toMatchObject({
      id: 'settings',
      label: 'Remote settings',
      href: 'https://door.test/remote/settings',
    });
    const rows = [...menu.groups.flatMap((group) => group.items), menu.settings!];
    expect(rows.filter((item) => item.current).map((item) => item.id)).toEqual(
      selected ? [selected] : [],
    );
    expect(menu.groups.every((group) => !group.add)).toBe(true);
  }
});

it('uses only the registry total and keeps recovery as an existing group command, never a row', () => {
  const { observe, menus, recover, refresh } = menuAt('/remote/members');
  expect(menus.at(-1)!.groups[0]!.items[0]!.total).toBeUndefined();
  const catalog = {
    version: 1 as const,
    entries: [{ name: 'Other', kind: 'local' as const, url: 'https://other.test/', project: null }],
  };
  const view = (state: string) =>
    registryView(catalog, { pageUrl: 'https://door.test/remote/', state });
  for (const state of ['Not paired', 'Unconfirmed', 'Connected']) {
    observe(view(state));
    const menu = menus.at(-1)!;
    expect(menu.groups.flatMap((group) => group.items).map((item) => item.id)).toEqual([
      'remotes',
      'members',
      'my-devices',
    ]);
    expect(menu.groups[0]!.items[0]!.total).toBe(2);
    expect(
      menu.groups[1]!.items.every((item) => item.total === undefined && item.count === undefined),
    ).toBe(true);
    const action = menu.groups[1]!.action;
    if (state === 'Connected') expect(action).toBeUndefined();
    else {
      expect(action!.label).toBe(state === 'Not paired' ? 'Pair again' : 'Retry');
      action!.onActivate();
    }
  }
  expect(recover.mock.calls).toEqual([['Pair again'], ['Retry']]);
  expect(refresh).not.toHaveBeenCalled();
});
