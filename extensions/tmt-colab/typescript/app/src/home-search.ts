import { labelNameError } from './labels.js';
import { AUTHOR_BYTES } from './bootstrap.js';

/** The query that may filter and order the list (and a copied URL), bounded and validated. */
export interface HomeSearch {
  q?: string;
  label?: string[];
  by?: string[];
  sort?: 'title';
  archived?: true;
}
/** The route merges what this returns over the raw query, so every key is named here: one it
 * cannot read is set to `undefined` and replaces the raw value instead of reaching the list. */
export function validateHomeSearch(search: Record<string, unknown>): HomeSearch {
  const out: HomeSearch = {
    q: undefined,
    label: undefined,
    by: undefined,
    sort: undefined,
    archived: undefined,
  };
  if (typeof search.q === 'string' && search.q.trim()) out.q = search.q.trim().slice(0, 256);
  if (Array.isArray(search.label)) {
    const names = [
      ...new Set(
        search.label.filter(
          (name): name is string => typeof name === 'string' && labelNameError(name) === undefined,
        ),
      ),
    ].slice(0, 8);
    if (names.length) out.label = names;
  }
  if (Array.isArray(search.by)) {
    const names = [
      ...new Set(
        search.by.filter(
          (name): name is string =>
            typeof name === 'string' &&
            name.length > 0 &&
            new TextEncoder().encode(name).length <= AUTHOR_BYTES &&
            !/\p{Cc}/u.test(name),
        ),
      ),
    ].slice(0, 8);
    if (names.length) out.by = names;
  }
  if (search.sort === 'title') out.sort = 'title';
  if (search.archived === true || search.archived === 'true') out.archived = true;
  return out;
}
