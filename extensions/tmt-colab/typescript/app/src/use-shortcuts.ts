import { useCallback, useEffect, useMemo, useState } from 'react';
import type { ProjectPins } from './bootstrap.js';
import { ShortcutStore } from './shortcut-store.js';
import type { Shortcut } from './shortcuts.js';
export function useShortcuts(space: string | undefined, pins: ProjectPins | undefined) {
  const store = useMemo(() => new ShortcutStore(space ?? ''), [space]);
  const [state, setState] = useState<{ store: ShortcutStore; items: Shortcut[]; error?: string }>({
    store,
    items: [],
  });
  const refresh = useCallback(() => {
    try {
      setState({ store, items: store.read().items });
    } catch {
      setState({
        store,
        items: [],
        error: 'Saved shortcuts could not be read. Existing data has been kept.',
      });
    }
  }, [store]);
  const legacy = JSON.stringify(pins);
  useEffect(() => {
    const changed = (event: Event) => {
      if (event instanceof StorageEvent && event.key !== null && event.key !== store.key) return;
      if (event instanceof CustomEvent && event.detail !== store.key) return;
      refresh();
    };
    window.addEventListener('storage', changed);
    window.addEventListener('colab-shortcuts', changed);
    try {
      store.migrate(legacy ? (JSON.parse(legacy) as ProjectPins) : undefined);
      refresh();
    } catch {
      setState({
        store,
        items: [],
        error: 'Saved shortcuts could not be updated. Existing data has been kept.',
      });
    }
    return () => {
      window.removeEventListener('storage', changed);
      window.removeEventListener('colab-shortcuts', changed);
    };
  }, [store, legacy, refresh]);
  return {
    store,
    items: state.store === store ? state.items : [],
    error: state.store === store ? state.error : undefined,
  };
}
