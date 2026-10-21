import type { RouterHistory } from '@tanstack/react-router';

/** The state key under which a history entry keeps the list's own search. */
const KEY = 'tmtColabList';
const MAX_SEARCH = 2048;

/** Whether a router path is the page list, with or without its search. */
export function isListPath(path: string): boolean {
  return path === '/' || path.startsWith('/?');
}

/** The search part of a list path (`?stage=all`), or nothing for a bare list. */
function listSearch(path: string): string | undefined {
  return path.startsWith('/?') && path.length > 2 ? path.slice(1) : undefined;
}

/** The search a history entry kept for the list; the list's own validation still reads it, so a
 * value that is not plain bounded text is no search at all. */
export function storedListSearch(state: unknown): string {
  const value = (state as Record<string, unknown> | null | undefined)?.[KEY];
  return typeof value === 'string' && value.length <= MAX_SEARCH && /^\?[^#]+$/.test(value)
    ? value
    : '';
}

/**
 * A mounted address names the list as `/colab/` and no more, so its tab and filters have no place
 * in it. Each entry keeps them in its own state instead: Back, Forward and a reload return to the
 * list as it was left, and a fresh visit to the bare list still opens the default tab.
 */
export function keepListSearch(history: RouterHistory): RouterHistory {
  const withSearch = (path: string, state: Record<string, unknown> | undefined) => {
    const search = listSearch(path);
    return search === undefined ? state : { ...state, [KEY]: search };
  };
  const { push, replace } = history;
  history.push = (path, state, options) => push(path, withSearch(path, state), options);
  history.replace = (path, state, options) => replace(path, withSearch(path, state), options);
  return history;
}
