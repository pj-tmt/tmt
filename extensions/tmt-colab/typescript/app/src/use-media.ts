import { useMemo, useSyncExternalStore } from 'react';

/** Whether a media query matches now, and again when it changes. */
export function useMedia(query: string): boolean {
  const subscribe = useMemo(
    () => (changed: () => void) => {
      const media = matchMedia(query);
      media.addEventListener('change', changed);
      return () => media.removeEventListener('change', changed);
    },
    [query],
  );
  return useSyncExternalStore(subscribe, () => matchMedia(query).matches);
}
