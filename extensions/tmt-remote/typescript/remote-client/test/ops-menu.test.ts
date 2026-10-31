import { expect, it } from 'vite-plus/test';
import { OPS_SECTIONS, opsMenu, opsSection, opsPlace, opsHref } from '../src/ops-menu.js';
import { visibleCurrent } from '@tmt/browser-ui/shell';
import type { OpsAgent, OpsBoard } from '../src/ops-source.js';

const agent = (state: OpsAgent['state']): OpsAgent => ({
  id: '00000000-0000-4000-8000-000000000001',
  name: state,
  squad: 'sample',
  role: 'member',
  state,
  waitingOnYou: state === 'offline' ? 12 : 0,
});
const board = (): OpsBoard => ({
  agents: [agent('working'), agent('needs-you'), agent('offline')],
  squads: [{ name: 'sample' }],
  schedules: null,
  truncated: false,
});

it('keeps a fixed child list and one selected row with settings in the shared final slot', () => {
  for (const section of OPS_SECTIONS) {
    expect(opsSection(`#${section.id}`)).toBe(section.id);
    const menu = opsMenu(section.id, undefined, '/ops/');
    expect(menu.groups.flatMap((group) => group.items).map((item) => item.label)).toEqual([
      'Agents',
      'Squads',
      'Timers',
    ]);
    expect(menu.settings).toEqual({
      id: 'settings',
      label: 'Ops settings',
      href: '/ops/#settings',
      current: section.id === 'settings',
    });
    const rows = [...menu.groups.flatMap((group) => group.items), menu.settings!];
    expect(rows.filter((item) => item.current).map((item) => item.id)).toEqual([section.id]);
    expect(menu.groups.every((group) => !group.action && !group.add)).toBe(true);
  }
  expect(opsSection('#not-a-section')).toBe('agents');
});

it('uses observed totals and the Agents view state for attention without probing anything', () => {
  const source = board();
  const before = JSON.stringify(source);
  const items = opsMenu('squads', source, '/ops').groups[0]!.items;
  expect(items.map((item) => item.total)).toEqual([3, 1, undefined]);
  expect(items[0]!.count).toEqual({ text: '1', needsYou: true });
  expect(items.slice(1).every((item) => item.count === undefined)).toBe(true);
  expect(JSON.stringify(source)).toBe(before);
  const empty = opsMenu('agents', { ...source, agents: [], squads: [], schedules: [] }, '/ops');
  expect(empty.groups[0]!.items.map((item) => item.total)).toEqual([0, 0, 0]);
  expect(empty.groups[0]!.items.every((item) => !item.count)).toBe(true);
});

it('keeps loading, unavailable, truncated and null observations unknown rather than zero', () => {
  for (const source of [
    undefined,
    { ...board(), unavailable: 'Read failed' },
    { ...board(), truncated: true },
  ]) {
    const menu = opsMenu('agents', source, '/ops');
    expect(
      menu.groups[0]!.items.every((item) => item.total === undefined && item.count === undefined),
    ).toBe(true);
  }
  expect(opsMenu('schedules', board(), '/ops').groups[0]!.items[2]!.total).toBeUndefined();
});

it('passes attention through the shared pill formatter, including its 99+ bound', () => {
  const source = { ...board(), agents: Array.from({ length: 101 }, () => agent('needs-you')) };
  expect(opsMenu('agents', source, '/ops').groups[0]!.items[0]).toMatchObject({
    total: 101,
    count: { text: '99+', needsYou: true },
  });
});

it('routes channels and DMs by scoped UUID, preserving literal squad names and rejecting malformed fragments', () => {
  const place = { squad: 'a/b # team', memberId: '00000000-0000-4000-8000-000000000001' };
  expect(opsPlace(new URL(opsHref('/ops/', place), 'https://example.test').hash)).toEqual(place);
  expect(opsPlace('#channel/a%2Fb%20%23%20team')).toEqual({ squad: 'a/b # team' });
  expect(opsPlace('#dm/name/alpha')).toBe('agents');
  expect(opsPlace('#channel/%broken')).toBe('agents');
});

it('keeps UUID selection on rename, never substitutes a namesake, and shows no unread counts', () => {
  const selected = { squad: 'sample', memberId: '00000000-0000-4000-8000-000000000001' };
  const renamed = { ...agent('working'), name: 'renamed' };
  const namesake = {
    ...agent('idle'),
    id: '00000000-0000-4000-8000-000000000002',
    name: 'working',
  };
  const source = { ...board(), agents: [renamed, namesake] };
  const menu = opsMenu(selected, source, '/ops/');
  const rows = menu.groups
    .slice(1)
    .flatMap((group) => group.items.flatMap((item) => item.children ?? []));
  expect(rows.filter((row) => row.current).map((row) => row.label)).toEqual(['renamed']);
  expect(rows.every((row) => row.count === undefined && row.total === undefined)).toBe(true);
  const removed = opsMenu(selected, { ...source, agents: [namesake] }, '/ops/');
  expect(visibleCurrent(removed)).toBeUndefined();
});

it('keeps expansion host-owned and delegates the deepest visible UUID selection to the shared rail', () => {
  const source = { ...board(), agents: [agent('working')] };
  const section = { squad: 'sample', memberId: source.agents[0]!.id };
  const collapsed = new Set<string>();
  const disclosure = {
    collapsed,
    toggle: (squad: string) => {
      collapsed.add(squad);
    },
  };
  const menu = opsMenu(section, source, '/ops/', disclosure);
  const parent = menu.groups[1]!.items[0]!;
  expect(parent.expanded).toBe(true);
  expect(parent.children).toHaveLength(1);
  expect(parent.children![0]).toMatchObject({ id: `dm:sample:${section.memberId}`, current: true });
  expect(parent.children![0]!.emphasis).toBeUndefined();
  expect(visibleCurrent(menu)?.id).toBe(`dm:sample:${section.memberId}`);
  parent.toggle!();
  const hidden = opsMenu(section, source, '/ops/', disclosure);
  expect(hidden.groups[1]!.items[0]!.expanded).toBe(false);
  expect(visibleCurrent(hidden)?.id).toBe('channel:sample');
  expect(source.agents[0]!.name).toBe('working');
});
