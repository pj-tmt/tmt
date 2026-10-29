import { useCallback, useEffect, useMemo, useState } from 'react';
import { ShortcutStore } from './shortcut-store.js';
import type { Shortcut } from './shortcuts.js';
import { text } from './strings.js';

export function useShortcuts(space: string | undefined) {
  const store = useMemo(
    () => (space === undefined ? undefined : new ShortcutStore(space)),
    [space],
  );
  const [state, setState] = useState<{ store?: ShortcutStore; items: Shortcut[]; error?: string }>({
    store,
    items: [],
  });
  const refresh = useCallback(() => {
    if (!store) return;
    try {
      setState({ store, items: store.read().items });
    } catch {
      setState({ store, items: [], error: text.shortcuts.readFailed });
    }
  }, [store]);
  useEffect(() => {
    if (!store) return;
    const changed = (event: Event) => {
      if (event instanceof StorageEvent && event.key !== null && event.key !== store.key) return;
      if (event instanceof CustomEvent && event.detail !== store.key) return;
      refresh();
    };
    window.addEventListener('storage', changed);
    window.addEventListener('colab-shortcuts', changed);
    refresh();
    return () => {
      window.removeEventListener('storage', changed);
      window.removeEventListener('colab-shortcuts', changed);
    };
  }, [store, refresh]);
  return {
    store,
    items: state.store === store ? state.items : [],
    error: state.store === store ? state.error : undefined,
  };
}
