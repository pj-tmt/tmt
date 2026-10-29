import { canonical } from './closed.js';
import { validateHomeSearch, type HomeSearch } from './page-list-filter.js';
export interface Shortcut {
  id: string;
  name: string;
  search: HomeSearch;
}
export interface ShortcutCollection {
  version: 1;
  migrated: boolean;
  items: Shortcut[];
}
export function shortcutNameError(name: string): 'empty' | 'form' | undefined {
  return !name.trim()
    ? 'empty'
    : [...name].length > 80 || /\p{Cc}/u.test(name)
      ? 'form'
      : undefined;
}
/** Scope becomes a normal project condition; the saved route never refers to another shortcut. */
export function shortcutSearch(search: HomeSearch): HomeSearch {
  const {
    scope,
    shortcut: _,
    ...validated
  } = validateHomeSearch(search as Record<string, unknown>);
  const project = [...new Set([...(scope ? [scope] : []), ...(validated.project ?? [])])];
  return Object.fromEntries(
    Object.entries({ ...validated, project: project.length ? project : undefined }).filter(
      ([, value]) => value !== undefined,
    ),
  );
}
export function readShortcuts(raw: string | null): ShortcutCollection {
  if (raw === null) return { version: 1, migrated: false, items: [] };
  const value: unknown = JSON.parse(raw);
  if (!value || typeof value !== 'object') throw new Error('Saved shortcuts cannot be read.');
  const record = value as ShortcutCollection;
  if (record.version !== 1 || typeof record.migrated !== 'boolean' || !Array.isArray(record.items))
    throw new Error('Saved shortcuts use an unsupported format.');
  const ids = new Set<string>();
  for (const item of record.items) {
    if (
      !item ||
      typeof item.id !== 'string' ||
      !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(item.id) ||
      ids.has(item.id) ||
      typeof item.name !== 'string' ||
      shortcutNameError(item.name) ||
      !item.search ||
      typeof item.search !== 'object' ||
      Array.isArray(item.search)
    )
      throw new Error('Saved shortcuts cannot be read.');
    if (canonical(shortcutSearch(item.search)) !== canonical(item.search))
      throw new Error('Saved shortcut filters cannot be read.');
    ids.add(item.id);
  }
  return record;
}
