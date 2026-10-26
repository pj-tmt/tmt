import { expect, it } from 'vite-plus/test';
import type { SearchGroup, SearchItem } from '@tmt/browser-ui/search';
import {
  COLAB_GROUP,
  NAVIGATION_GROUP,
  PROJECT_RESULTS,
  PROVIDER_RESULTS,
  composeEntries,
  itemValue,
  matchItems,
  matchProjects,
  pageValue,
  projectValue,
  scopeNames,
} from '../src/quick-switcher-groups.js';
import type { SwitcherResult } from '../src/quick-switcher.js';
import { text } from '../src/strings.js';

const item = (key: string, label: string, extra: Partial<SearchItem> = {}): SearchItem => ({
  key,
  label,
  href: `/m/${key}`,
  ...extra,
});
const group = (key: string, items: SearchItem[] | 'unavailable'): SearchGroup => ({
  key,
  label: key.toUpperCase(),
  state: items === 'unavailable' ? 'unavailable' : 'ready',
  items: items === 'unavailable' ? [] : items,
});
const result = (id: string): SwitcherResult => ({
  page: { id, title: `Page ${id}`, sharing: 'private' },
});
const projects = [
  { name: 'design', count: 4 },
  { name: 'ops', count: 2 },
];
const labels = (entries: ReturnType<typeof composeEntries>) =>
  entries.map((entry) =>
    entry.kind === 'page'
      ? `page ${entry.result.page.id}`
      : entry.kind === 'project'
        ? `project ${entry.name}`
        : `${entry.groupKey} ${entry.item.label}`,
  );

it('keeps the list flat and as ranked when no provider is registered', () => {
  const entries = composeEntries({
    pages: [result('a'), result('b')],
    projects,
    text: 'design',
    colabLabel: 'Colab',
  });
  expect(labels(entries)).toEqual(['page a', 'page b']);
  expect(entries.every((entry) => entry.group === undefined)).toBe(true);
});

it('lists navigation first, then Colab, then the other providers in registration order', () => {
  const entries = composeEntries({
    providers: [
      group('ops', [item('agent', 'review-agent')]),
      group(NAVIGATION_GROUP, [item('home', 'Home'), item('digest', 'Digest')]),
      group('remote', [item('mac', 'Studio Mac')]),
    ],
    pages: [result('a')],
    projects,
    text: '',
    colabLabel: 'Colab',
  });
  expect(labels(entries)).toEqual([
    'navigation Home',
    'navigation Digest',
    'page a',
    'ops review-agent',
    'remote Studio Mac',
  ]);
  expect(entries.map((entry) => entry.group?.label)).toEqual([
    'NAVIGATION',
    'NAVIGATION',
    'Colab',
    'OPS',
    'REMOTE',
  ]);
  expect(entries.map((entry) => entry.value)).toEqual([
    itemValue(NAVIGATION_GROUP, 'home'),
    itemValue(NAVIGATION_GROUP, 'digest'),
    pageValue('a'),
    itemValue('ops', 'agent'),
    itemValue('remote', 'mac'),
  ]);
});

it('shows no heading for an unavailable or empty group and never one named colab', () => {
  const entries = composeEntries({
    providers: [
      group('ops', 'unavailable'),
      group('remote', []),
      group(COLAB_GROUP, [item('x', 'Impostor')]),
    ],
    pages: [result('a')],
    projects,
    text: '',
    colabLabel: 'Colab',
  });
  expect(labels(entries)).toEqual(['page a']);
});

it('offers the projects the text names after the pages, never without text', () => {
  const providers = [group(NAVIGATION_GROUP, [])];
  const typed = composeEntries({
    providers,
    pages: [result('a')],
    projects,
    text: 'DES',
    colabLabel: 'Colab',
  });
  expect(labels(typed)).toEqual(['page a', 'project design']);
  expect(typed[1]).toMatchObject({ kind: 'project', pages: 4, value: projectValue('design') });
  expect(
    labels(
      composeEntries({ providers, pages: [result('a')], projects, text: '', colabLabel: 'Colab' }),
    ),
  ).toEqual(['page a']);
  const many = Array.from({ length: PROJECT_RESULTS + 3 }, (_, n) => ({
    name: `proj-${n}`,
    count: 1,
  }));
  expect(matchProjects(many, 'proj')).toHaveLength(PROJECT_RESULTS);
});

it('matches provider items on the text alone and leaves them to a project or author filter', () => {
  const nav = group(NAVIGATION_GROUP, [
    item('ops-agents', 'Ops agents', { detail: 'Section' }),
    item('home', 'Home', { words: ['start'] }),
    item('colab', 'Colab pages'),
  ]);
  const text = (value: string) =>
    labels(
      composeEntries({
        providers: [nav],
        pages: [],
        projects: [],
        text: value,
        colabLabel: 'Colab',
      }),
    );
  expect(text('ops')).toEqual(['navigation Ops agents']);
  expect(text('START')).toEqual(['navigation Home']);
  expect(text('section ops')).toEqual(['navigation Ops agents']);
  expect(text('nothing')).toEqual([]);
  // The caller applies the project and author filter to the pages only: none of it reaches a group.
  expect(
    labels(
      composeEntries({ providers: [nav], pages: [], projects: [], text: '', colabLabel: 'Colab' }),
    ),
  ).toEqual(['navigation Ops agents', 'navigation Home', 'navigation Colab pages']);
});

it('ranks a label match above a detail match and keeps provider order within a strength', () => {
  const items = [
    item('a', 'Agents settings', { detail: 'Ops' }),
    item('b', 'Squads', { words: ['ops'] }),
    item('c', 'Ops agents'),
    item('d', 'Timers · ops'),
  ];
  expect(matchItems(items, 'ops').map((entry) => entry.key)).toEqual(['c', 'd', 'a', 'b']);
  expect(matchItems(items, '').map((entry) => entry.key)).toEqual(['a', 'b', 'c', 'd']);
});

it('caps each provider group', () => {
  const many = Array.from({ length: PROVIDER_RESULTS + 5 }, (_, n) => item(`k${n}`, `Item ${n}`));
  const entries = composeEntries({
    providers: [group('ops', many)],
    pages: [],
    projects: [],
    text: 'item',
    colabLabel: 'Colab',
  });
  expect(entries).toHaveLength(PROVIDER_RESULTS);
});

const labelled = (
  key: string,
  label: string,
  state: SearchGroup['state'] = 'ready',
): SearchGroup => ({
  key,
  label,
  state,
  items: [],
});
const placeholder = (groups: SearchGroup[]) =>
  text.quickSwitcher.placeholderGlobal(scopeNames(groups, text.quickSwitcher.pagesNoun));

it('names the ready groups in list order, navigation first and Colab pages after it', () => {
  const groups = [
    labelled('agents', 'Agents'),
    labelled(NAVIGATION_GROUP, 'Navigation'),
    labelled('devices', 'Devices'),
  ];
  expect(scopeNames(groups, 'pages')).toEqual(['navigation', 'pages', 'agents', 'devices']);
  expect(placeholder(groups)).toBe('Search navigation, pages, agents or devices…');
  expect(placeholder([labelled(NAVIGATION_GROUP, 'Navigation')])).toBe(
    'Search navigation or pages…',
  );
});

it('does not name a group that is unavailable, and follows it when it recovers', () => {
  const nav = labelled(NAVIGATION_GROUP, 'Navigation');
  expect(placeholder([nav, labelled('agents', 'Agents', 'unavailable')])).toBe(
    'Search navigation or pages…',
  );
  expect(placeholder([nav, labelled('agents', 'Agents')])).toBe(
    'Search navigation, pages or agents…',
  );
});

it('falls back to Search TMT… until a group is ready, and never names Colab twice', () => {
  expect(placeholder([])).toBe('Search TMT…');
  expect(placeholder([labelled(NAVIGATION_GROUP, 'Navigation', 'unavailable')])).toBe(
    'Search TMT…',
  );
  expect(placeholder([labelled(COLAB_GROUP, 'Colab')])).toBe('Search TMT…');
  expect(
    placeholder([labelled(NAVIGATION_GROUP, 'Navigation'), labelled(COLAB_GROUP, 'Colab')]),
  ).toBe('Search navigation or pages…');
});

it('keeps a label that carries capitals past its first letter', () => {
  expect(
    scopeNames(
      [labelled(NAVIGATION_GROUP, 'Navigation'), labelled('settings', 'TMT settings')],
      'pages',
    ),
  ).toEqual(['navigation', 'pages', 'TMT settings']);
});
