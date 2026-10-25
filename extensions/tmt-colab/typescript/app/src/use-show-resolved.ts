import { useCallback, useState } from 'react';

const KEY = 'colab.comments.showResolved';

function read() {
  try {
    return localStorage.getItem(KEY) === '1';
  } catch {
    return false;
  }
}

/**
 * Whether the Comments panel also lists and walks resolved threads. A per-viewer convenience kept on
 * this device (never page data): off until the viewer turns it on, and off again when storage is
 * unavailable or refuses the write.
 */
export function useShowResolved() {
  const [shown, setShown] = useState(read);
  const set = useCallback((next: boolean) => {
    setShown(next);
    try {
      if (next) localStorage.setItem(KEY, '1');
      else localStorage.removeItem(KEY);
    } catch {
      // The choice holds for this visit only.
    }
  }, []);
  return [shown, set] as const;
}
