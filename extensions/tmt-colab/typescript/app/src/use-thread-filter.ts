import { useCallback, useState } from 'react';
import type { ThreadFilter } from './comments-listing.js';

const KEY = 'colab.comments.filter';
function read(): ThreadFilter {
  try {
    const value = localStorage.getItem(KEY);
    if (value === 'open' || value === 'needs' || value === 'all') return value;
    return 'open';
  } catch {
    return 'open';
  }
}
/** A device-local viewing preference, never synced page data or authority. */
export function useThreadFilter() {
  const [filter, setFilter] = useState(read);
  const set = useCallback((next: ThreadFilter) => {
    setFilter(next);
    try {
      localStorage.setItem(KEY, next);
    } catch {
      /* Holds for this visit only. */
    }
  }, []);
  return [filter, set] as const;
}
