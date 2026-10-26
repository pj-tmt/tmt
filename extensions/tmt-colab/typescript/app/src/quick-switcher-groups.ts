import type { SearchGroup, SearchItem } from '@tmt/browser-ui/search';
import type { SwitcherResult } from './quick-switcher.js';

/** The key the root registers its module and section navigation under; its group lists first. */
export const NAVIGATION_GROUP = 'navigation';
/** Colab's own group: the ranked pages and the projects, which Colab ranks and filters itself. */
export const COLAB_GROUP = 'colab';
/** The most items one provider group offers for a query. */
export const PROVIDER_RESULTS = 12;
/** The most projects the Colab group offers beside its pages. */
export const PROJECT_RESULTS = 5;

export interface GroupHeading {
  key: string;
  label: string;
}
/** One row of the switcher: where it goes is its own page, project or provider item. */
export type SwitcherEntry =
  | { kind: 'page'; value: string; group?: GroupHeading; result: SwitcherResult }
  | { kind: 'project'; value: string; group?: GroupHeading; name: string; pages: number }
  | { kind: 'item'; value: string; group?: GroupHeading; groupKey: string; item: SearchItem };

export const pageValue = (id: string) => `page:${id}`;
export const projectValue = (name: string) => `project:${name}`;
export const itemValue = (groupKey: string, itemKey: string) =>
  `item:${encodeURIComponent(groupKey)}:${encodeURIComponent(itemKey)}`;

const fold = (value: string) => value.toLocaleLowerCase();
const WORD = /[^\p{L}\p{N}]+/u;

/** How well one word matches a name, best (0) to weakest: prefix, a word's prefix, contains. */
function nameStrength(word: string, name: string): number | undefined {
  const folded = fold(name);
  if (folded.startsWith(word)) return 0;
  if (folded.split(WORD).some((part) => part.startsWith(word))) return 1;
  if (folded.includes(word)) return 2;
  return undefined;
}

/**
 * The items a free-text query keeps, best first: every word must appear in the label, the detail or
 * the extra words, and a match in the label beats one in the rest. Without text, every item in the
 * provider's own order. Equal strength keeps the provider's order.
 */
export function matchItems(items: readonly SearchItem[], text: string): SearchItem[] {
  const words = fold(text).split(/\s+/).filter(Boolean);
  if (!words.length) return [...items];
  const kept: { item: SearchItem; strength: number }[] = [];
  for (const item of items) {
    let strength = 0;
    for (const word of words) {
      const own = nameStrength(word, item.label);
      const other = [item.detail ?? '', ...(item.words ?? [])].some((part) =>
        fold(part).includes(word),
      )
        ? 3
        : undefined;
      const best = own ?? other;
      if (best === undefined) {
        strength = -1;
        break;
      }
      strength = Math.max(strength, best);
    }
    if (strength >= 0) kept.push({ item, strength });
  }
  return kept.sort((a, b) => a.strength - b.strength).map(({ item }) => item);
}

/** The projects whose name matches the text, best first; none without text. */
export function matchProjects(
  projects: readonly { name: string; count: number }[],
  text: string,
): { name: string; count: number }[] {
  const words = fold(text).split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  const kept: { project: { name: string; count: number }; strength: number }[] = [];
  for (const project of projects) {
    const strengths = words.map((word) => nameStrength(word, project.name));
    if (strengths.some((strength) => strength === undefined)) continue;
    kept.push({ project, strength: Math.max(...(strengths as number[])) });
  }
  return kept
    .sort((a, b) => a.strength - b.strength)
    .slice(0, PROJECT_RESULTS)
    .map(({ project }) => project);
}

/**
 * Everything the open switcher lists: the navigation group first, then Colab's own, then every other
 * provider in registration order. Colab's group is `pages` as ranked (its project and author filters
 * already applied) plus the projects the text names; the provider groups take the text alone, so a
 * project or author filter never hides them. Without `providers` the list is flat and unchanged.
 */
export function composeEntries({
  providers,
  pages,
  projects,
  text,
  colabLabel,
}: {
  providers?: readonly SearchGroup[];
  pages: readonly SwitcherResult[];
  projects: readonly { name: string; count: number }[];
  text: string;
  colabLabel: string;
}): SwitcherEntry[] {
  const colab: GroupHeading | undefined = providers && { key: COLAB_GROUP, label: colabLabel };
  const own: SwitcherEntry[] = [
    ...pages.map((result): SwitcherEntry => ({
      kind: 'page',
      value: pageValue(result.page.id),
      group: colab,
      result,
    })),
    ...(providers ? matchProjects(projects, text) : []).map(({ name, count }): SwitcherEntry => ({
      kind: 'project',
      value: projectValue(name),
      group: colab,
      name,
      pages: count,
    })),
  ];
  if (!providers) return own;
  const listed = providers
    .filter((group) => group.key !== COLAB_GROUP)
    .map((group) => ({
      group,
      entries: matchItems(group.items, text)
        .slice(0, PROVIDER_RESULTS)
        .map((item): SwitcherEntry => ({
          kind: 'item',
          value: itemValue(group.key, item.key),
          group: { key: group.key, label: group.label },
          groupKey: group.key,
          item,
        })),
    }));
  const first = listed.filter(({ group }) => group.key === NAVIGATION_GROUP);
  const rest = listed.filter(({ group }) => group.key !== NAVIGATION_GROUP);
  return [...first, { entries: own }, ...rest].flatMap(({ entries }) => entries);
}
