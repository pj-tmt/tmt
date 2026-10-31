import { expect, it } from 'vite-plus/test';
import { OPS_SECTIONS, opsMenu, opsSection } from '../src/ops-menu.js';
import type { OpsAgent, OpsBoard } from '../src/ops-source.js';

const agent = (state: OpsAgent['state']): OpsAgent => ({
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
