import { expect, it } from 'vite-plus/test';
import {
  describedMenu,
  shellCount,
  shellRail,
  type ShellAnswer,
  type ShellModule,
} from '../src/shell.js';
import { moduleIcon } from '../src/shell-icons.js';

const origin = 'http://127.0.0.1:50711';
const colab: ShellModule = {
  extension: 'colab',
  rail: { label: 'Colab', icon: 'file-text', path: `${origin}/r/abc/x/colab/`, order: 100 },
  menu: [
    {
      id: 'pages',
      label: 'Pages',
      path: `${origin}/r/abc/x/colab/`,
      count: { value: 120, kind: 'plain' },
    },
  ],
  settings: [],
  needsYou: 2,
};
const remote: ShellModule = {
  extension: 'remote',
  rail: { label: 'Remote', icon: 'monitor-smartphone', path: `${origin}/remote/`, order: 200 },
  menu: [{ id: 'remote', label: 'Remote', path: `${origin}/remote/` }],
  settings: [{ id: 'settings', label: 'Remote settings', path: `${origin}/remote/settings` }],
  needsYou: 1,
};
const answer: ShellAnswer = { version: 1, modules: [colab, remote] };

it('orders Home, the described modules, Ops, Digest, Search, then TMT settings at the bottom', () => {
  const rail = shellRail(answer, { origin, href: `${origin}/` });
  expect(rail.main.map((item) => [item.id, item.href, item.current])).toEqual([
    ['home', `${origin}/`, true],
    ['colab', `${origin}/r/abc/x/colab/`, false],
    ['remote', `${origin}/remote/`, false],
    ['ops', `${origin}/ops/`, false],
    ['digest', `${origin}/digest/`, false],
    ['search', `${origin}/r/abc/x/colab/`, false],
  ]);
  expect(
    shellRail(answer, { origin, href: `${origin}/ops/` })
      .main.filter((item) => item.current)
      .map((item) => item.id),
  ).toEqual(['ops']);
  expect(rail.main[0]!.count).toEqual({ text: '3', needsYou: true });
  expect(rail.bottom.map((item) => [item.id, item.label, item.href])).toEqual([
    ['settings', 'TMT settings', `${origin}/settings/`],
  ]);
});

it('marks the longest matching module, or the host-named one, and settings on /settings', () => {
  const at = (path: string, host?: ShellModule) =>
    shellRail(answer, { origin, href: `${origin}${path}` }, host)
      .main.concat(shellRail(answer, { origin, href: `${origin}${path}` }, host).bottom)
      .filter((item) => item.current)
      .map((item) => item.id);
  expect(at('/remote/settings')).toEqual(['remote']);
  expect(at('/r/abc/x/colab/pages/p1')).toEqual(['colab']);
  expect(at('/colab/', colab)).toEqual(['colab']);
  expect(at('/settings/')).toEqual(['settings']);
});

it('shows the same rail items on every route, with or without an answer, whatever the host', () => {
  const routes = [
    '/',
    '/colab/',
    '/remote/',
    '/remote/settings',
    '/ops/',
    '/digest/',
    '/settings/',
  ];
  const hosts = [undefined, colab, remote];
  const shape = (rail: ReturnType<typeof shellRail>) =>
    [...rail.main, ...rail.bottom].map((item) => [item.id, item.label, item.icon]);
  const expected = [
    ['home', 'Home', 'house'],
    ['colab', 'Colab', 'file-text'],
    ['remote', 'Remote', 'monitor-smartphone'],
    ['ops', 'Ops', 'users'],
    ['digest', 'Digest', 'chart-column'],
    ['search', 'Search', 'search'],
    ['settings', 'TMT settings', 'settings'],
  ];
  for (const known of [answer, undefined])
    for (const route of routes)
      for (const host of hosts)
        expect(shape(shellRail(known, { origin, href: `${origin}${route}` }, host))).toEqual(
          expected,
        );
});

it('dims Colab and Search with the reason until the answer describes Colab', () => {
  const unpaired = shellRail(undefined, { origin, href: `${origin}/` }).main;
  expect(unpaired.filter((item) => item.disabled).map((item) => [item.id, item.note])).toEqual([
    ['colab', 'Pair this browser with Remote to open it'],
    ['search', 'Search opens Colab. Pair this browser with Remote to open it'],
  ]);
  expect(unpaired[0]!.count).toBeUndefined();
  const hosted = shellRail(undefined, { origin, href: `${origin}/colab/` }, colab).main;
  expect(hosted.filter((item) => item.disabled)).toEqual([]);
  expect(hosted.find((item) => item.current)?.id).toBe('colab');
  const without = shellRail({ version: 1, modules: [remote] }, { origin, href: `${origin}/` });
  expect(without.main.find((item) => item.id === 'colab')?.note).toBe(
    'Not installed on this machine',
  );
  const unknown = { version: 1 as const, modules: [{ ...colab, needsYou: undefined }, remote] };
  expect(shellRail(unknown, { origin, href: `${origin}/` }).main[0]!.count).toBeUndefined();
});

it('caps counts at 99+ and falls back to the box icon', () => {
  expect(shellCount(100)).toEqual({ text: '99+', needsYou: true });
  expect(shellCount(5, 'plain')).toEqual({ text: '5', needsYou: false });
  expect(moduleIcon('network')).toBe('box');
  expect(moduleIcon('users')).toBe('users');
});

it('turns a described module into its menu with settings last and the current entry marked', () => {
  const menu = describedMenu(remote, { origin, href: `${origin}/remote/settings` });
  expect(menu.title).toBe('Remote');
  expect(menu.groups.map((group) => group.items.map((item) => [item.label, item.current]))).toEqual(
    [[['Remote', false]], [['Remote settings', true]]],
  );
  expect(describedMenu(colab, { origin, href: `${origin}/` }).groups[0]!.items[0]!.count).toEqual({
    text: '99+',
    needsYou: false,
  });
});
